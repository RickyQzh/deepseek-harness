//! Named provider registry with capability-checking one-shot `start`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use dsh_agent::AgentHandle;
use dsh_kernel::Context;

use crate::{SubagentError, SubagentProvider, SubagentResult, SubagentStartRequest};

/// Named-provider registry provided as `subagents`.
///
/// `inject::<SubagentRuntime>()` yields `Arc<Self>`, so [`Self::register_provider`]
/// takes `&self` with an interior mutex.
pub struct SubagentRuntime {
    providers: Mutex<HashMap<String, Arc<dyn SubagentProvider>>>,
    ctx: Context,
}

impl SubagentRuntime {
    /// Empty registry on `ctx`.
    #[must_use]
    pub fn new(ctx: Context) -> Self {
        Self {
            providers: Mutex::new(HashMap::new()),
            ctx,
        }
    }

    /// Kernel context this runtime was constructed with.
    #[must_use]
    pub fn context(&self) -> &Context {
        &self.ctx
    }

    /// Register `provider`. Duplicate [`SubagentProvider::name`] values fail.
    ///
    /// # Errors
    ///
    /// [`SubagentError::DuplicateProvider`] when a provider with that name is already registered.
    pub fn register_provider(
        &self,
        provider: Arc<dyn SubagentProvider>,
    ) -> Result<(), SubagentError> {
        let name = provider.name().to_string();
        if name.is_empty() {
            return Err(SubagentError::other(
                "subagent provider name must be non-empty",
            ));
        }
        let mut providers = self.lock();
        if providers.contains_key(&name) {
            return Err(SubagentError::DuplicateProvider(name));
        }
        providers.insert(name, provider);
        Ok(())
    }

    /// Validate the named provider exists and supports requested capabilities, then delegate.
    ///
    /// Does not enter continuation. Does not call [`SubagentProvider::prepare_continuable`].
    ///
    /// # Errors
    ///
    /// [`SubagentError::UnknownProvider`], [`SubagentError::UnsupportedCapability`], or the provider.
    pub async fn start(
        &self,
        provider: &str,
        request: SubagentStartRequest,
        parent: AgentHandle,
    ) -> Result<SubagentResult, SubagentError> {
        let selected = {
            let providers = self.lock();
            providers.get(provider).cloned()
        };
        let selected =
            selected.ok_or_else(|| SubagentError::UnknownProvider(provider.to_string()))?;
        if request.max_depth.is_some() && !selected.capabilities().depth_limit {
            return Err(SubagentError::UnsupportedCapability(
                selected.name().to_string(),
                "depthLimit".into(),
            ));
        }
        selected.start(request, parent).await
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Arc<dyn SubagentProvider>>> {
        self.providers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::SubagentRuntime;
    use crate::{
        ContinuableCreateSpec, SubagentCapabilities, SubagentError, SubagentProvider,
        SubagentResult, SubagentStartRequest, SubagentStopReason,
    };
    use dsh_agent::{AgentHandle, AgentRegistry, CreateAgentOptions};
    use dsh_kernel::Context;
    use dsh_llm::{LlmRuntime, MockAdapter, MockScript, text_response};
    use dsh_session::{ContentBlock, Message, MessageId, MessageRole, MessageSource, SessionId};
    use dsh_system_prompt::{SystemPrompt, SystemPromptConfig};
    use dsh_tools::{AbortFlag, ToolPresentationMode, ToolRuntime};
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct FakeSpawn {
        prepare_called: Arc<AtomicBool>,
    }

    impl SubagentProvider for FakeSpawn {
        fn name(&self) -> &str {
            "spawn"
        }

        fn capabilities(&self) -> SubagentCapabilities {
            SubagentCapabilities::all()
        }

        fn inherits_parent_context(&self) -> bool {
            false
        }

        fn start(
            &self,
            request: SubagentStartRequest,
            parent: AgentHandle,
        ) -> Pin<Box<dyn Future<Output = Result<SubagentResult, SubagentError>> + Send + '_>>
        {
            Box::pin(async move {
                let parent_has_secret = parent
                    .lock()
                    .session
                    .derive_messages()
                    .iter()
                    .any(|message| message_text(message).contains("secret parent fact"));
                assert!(parent_has_secret, "parent must still hold its own history");
                assert!(!self.inherits_parent_context());
                Ok(SubagentResult {
                    stop_reason: SubagentStopReason::Completed,
                    output: request.prompt,
                })
            })
        }

        fn prepare_continuable(&self, _parent: &AgentHandle) -> ContinuableCreateSpec {
            self.prepare_called.store(true, Ordering::SeqCst);
            ContinuableCreateSpec { seed: None }
        }
    }

    fn message_text(message: &Message) -> String {
        message
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }

    fn prompt(text: &str) -> SubagentStartRequest {
        SubagentStartRequest {
            label: None,
            prompt: vec![ContentBlock::Text { text: text.into() }],
            parent_id: SessionId::new("pending"),
            signal: AbortFlag::new(),
            max_depth: None,
        }
    }

    fn registry_with_text(text: &str) -> AgentRegistry {
        let adapter = Arc::new(MockAdapter::new(vec![MockScript::Chunks(text_response(
            text,
        ))]));
        let mut llm = LlmRuntime::new();
        llm.register_adapter("mock", adapter);
        AgentRegistry::new(
            llm,
            ToolRuntime::new(ToolPresentationMode::Native),
            SystemPrompt::new(SystemPromptConfig::default()).unwrap(),
        )
    }

    async fn parent_with_history(text: &str) -> AgentHandle {
        let registry = registry_with_text("ack");
        let parent = registry
            .create(CreateAgentOptions {
                session_id: SessionId::new("parent-1"),
                cwd: None,
                provider: "mock".into(),
                model: "mock".into(),
                max_tokens: None,
            })
            .unwrap();
        parent
            .followup(Message {
                id: MessageId::new("m1"),
                role: MessageRole::User,
                content: vec![ContentBlock::Text { text: text.into() }],
                source: MessageSource::User,
            })
            .await
            .unwrap();
        parent.run_until_idle().await.unwrap();
        parent
    }

    #[tokio::test]
    async fn spawn_child_does_not_see_parent_history() {
        let prepare_called = Arc::new(AtomicBool::new(false));
        let rt = SubagentRuntime::new(Context::new());
        rt.register_provider(Arc::new(FakeSpawn {
            prepare_called: Arc::clone(&prepare_called),
        }))
        .unwrap();
        let parent = parent_with_history("secret parent fact").await;
        let result = rt
            .start("spawn", prompt("repeat any secret"), parent.clone())
            .await
            .unwrap();
        assert!(matches!(result.stop_reason, SubagentStopReason::Completed));
        assert!(!result.output.iter().any(|block| match block {
            ContentBlock::Text { text } => text.contains("secret parent fact"),
            _ => false,
        }));
        assert!(!prepare_called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn unknown_provider_fails_loud() {
        let rt = SubagentRuntime::new(Context::new());
        let parent = parent_with_history("hi").await;
        let err = rt.start("missing", prompt("hi"), parent).await.unwrap_err();
        assert!(matches!(err, SubagentError::UnknownProvider(name) if name == "missing"));
    }

    #[tokio::test]
    async fn max_depth_without_capability_fails() {
        struct NoDepth;

        impl SubagentProvider for NoDepth {
            fn name(&self) -> &str {
                "external"
            }

            fn capabilities(&self) -> SubagentCapabilities {
                SubagentCapabilities {
                    output_schema: false,
                    depth_limit: false,
                    tool_filter: false,
                    persona: false,
                }
            }

            fn inherits_parent_context(&self) -> bool {
                false
            }

            fn start(
                &self,
                _request: SubagentStartRequest,
                _parent: AgentHandle,
            ) -> Pin<Box<dyn Future<Output = Result<SubagentResult, SubagentError>> + Send + '_>>
            {
                Box::pin(async { Err(SubagentError::other("must not start")) })
            }

            fn prepare_continuable(&self, _parent: &AgentHandle) -> ContinuableCreateSpec {
                ContinuableCreateSpec { seed: None }
            }
        }

        let rt = SubagentRuntime::new(Context::new());
        rt.register_provider(Arc::new(NoDepth)).unwrap();
        let parent = parent_with_history("hi").await;
        let mut request = prompt("hi");
        request.max_depth = Some(3);
        let err = rt.start("external", request, parent).await.unwrap_err();
        assert!(
            matches!(err, SubagentError::UnsupportedCapability(name, cap) if name == "external" && cap == "depthLimit")
        );
    }

    #[test]
    fn duplicate_provider_fails_loud() {
        let rt = SubagentRuntime::new(Context::new());
        let prepare_called = Arc::new(AtomicBool::new(false));
        rt.register_provider(Arc::new(FakeSpawn {
            prepare_called: Arc::clone(&prepare_called),
        }))
        .unwrap();
        let err = rt
            .register_provider(Arc::new(FakeSpawn { prepare_called }))
            .unwrap_err();
        assert!(matches!(err, SubagentError::DuplicateProvider(name) if name == "spawn"));
    }
}
