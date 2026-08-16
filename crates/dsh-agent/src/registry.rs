//! Session-id keyed LoopAgent map.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use dsh_agent_loop::{AgentStatus, LoopAgent, LoopOptions};
use dsh_llm::LlmRuntime;
use dsh_session::{
    AppendSink, Message, MessageId, SESSION_FORMAT_VERSION, Session, SessionHeader, SessionId,
};
use dsh_system_prompt::SystemPrompt;
use dsh_tools::ToolRuntime;
use tokio::sync::Mutex as AsyncMutex;

use crate::AgentError;

type SinkFactory = Arc<dyn Fn(&str) -> AppendSink + Send + Sync>;

/// Options for [`AgentRegistry::create`].
pub struct CreateAgentOptions {
    /// Session identity shared by the registry and the log.
    pub session_id: SessionId,
    /// Absolute working directory recorded on the header.
    pub cwd: Option<String>,
    /// Provider route.
    pub provider: String,
    /// Model id.
    pub model: String,
    /// Optional output-token cap.
    pub max_tokens: Option<u64>,
}

/// Cloneable handle to one live agent.
#[derive(Clone)]
pub struct AgentHandle {
    id: SessionId,
    agent: Arc<AsyncMutex<LoopAgent>>,
}

impl AgentHandle {
    /// Session identity.
    #[must_use]
    pub fn id(&self) -> &SessionId {
        &self.id
    }

    /// Lock the live agent so tests can read `session.events()`.
    #[cfg(test)]
    pub(crate) async fn lock_for_test(&self) -> tokio::sync::MutexGuard<'_, LoopAgent> {
        self.agent.lock().await
    }
}

/// Live agents keyed by session id string.
pub struct AgentRegistry {
    agents: Mutex<HashMap<String, AgentHandle>>,
    llm: Mutex<LlmRuntime>,
    tools: Mutex<ToolRuntime>,
    prompt: SystemPrompt,
    sink_factory: Mutex<Option<SinkFactory>>,
}

impl AgentRegistry {
    /// Empty registry that clones `llm` / `tools` into each created agent.
    #[must_use]
    pub fn new(llm: LlmRuntime, tools: ToolRuntime, prompt: SystemPrompt) -> Self {
        Self {
            agents: Mutex::new(HashMap::new()),
            llm: Mutex::new(llm),
            tools: Mutex::new(tools),
            prompt,
            sink_factory: Mutex::new(None),
        }
    }

    /// Provider route ids on the shared runtime.
    #[must_use]
    pub fn list_providers(&self) -> Vec<String> {
        self.llm.lock().expect("llm").list_providers()
    }

    /// Mutable access to the shared LLM runtime (adapter registration).
    pub fn llm(&self) -> std::sync::MutexGuard<'_, LlmRuntime> {
        self.llm.lock().expect("llm")
    }

    /// Mutable access to the shared tool runtime (tool registration).
    pub fn tools(&self) -> std::sync::MutexGuard<'_, ToolRuntime> {
        self.tools.lock().expect("tools")
    }

    /// Install a per-session append sink factory. `None` clears it.
    pub fn set_sink_factory(&self, factory: Option<SinkFactory>) {
        *self.sink_factory.lock().expect("sink") = factory;
    }

    /// Create or return the live handle for `opts.session_id`.
    ///
    /// # Errors
    ///
    /// [`AgentError::Loop`] when inbox replay of an empty session fails (should not happen).
    pub fn create(&self, opts: CreateAgentOptions) -> Result<AgentHandle, AgentError> {
        let key = opts.session_id.as_str().to_string();
        {
            let agents = self.agents.lock().expect("agents");
            if let Some(existing) = agents.get(&key) {
                return Ok(existing.clone());
            }
        }
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let mut session = Session::new(SessionHeader {
            version: SESSION_FORMAT_VERSION,
            id: opts.session_id.clone(),
            created_at,
            cwd: opts.cwd.clone(),
            parent_session: None,
            seed_length: None,
            origin: None,
            delegation_depth: None,
            agent_preset: None,
        });
        if let Some(factory) = self.sink_factory.lock().expect("sink").clone() {
            session.set_append_sink(Some(factory(opts.session_id.as_str())));
        }
        let loop_agent = LoopAgent::new(
            session,
            LoopOptions {
                provider: opts.provider,
                model: opts.model,
                max_tokens: opts.max_tokens,
                max_parallel_tool_calls: dsh_agent_loop::DEFAULT_MAX_PARALLEL_TOOL_CALLS,
            },
            self.tools.lock().expect("tools").clone(),
            self.prompt.clone(),
            self.llm.lock().expect("llm").clone(),
        )?;
        let handle = AgentHandle {
            id: opts.session_id,
            agent: Arc::new(AsyncMutex::new(loop_agent)),
        };
        self.agents
            .lock()
            .expect("agents")
            .insert(key, handle.clone());
        Ok(handle)
    }

    /// Live handle, if present.
    #[must_use]
    pub fn get(&self, session_id: &str) -> Option<AgentHandle> {
        self.agents.lock().expect("agents").get(session_id).cloned()
    }

    async fn lock(
        &self,
        session_id: &str,
    ) -> Result<tokio::sync::OwnedMutexGuard<LoopAgent>, AgentError> {
        let handle = self
            .get(session_id)
            .ok_or_else(|| AgentError::UnknownSession(session_id.to_string()))?;
        Ok(Arc::clone(&handle.agent).lock_owned().await)
    }

    /// Queue `message` on next-turn. Does not start a driver.
    ///
    /// # Errors
    ///
    /// [`AgentError::UnknownSession`] or inbox splice failure.
    pub async fn followup(
        &self,
        session_id: &str,
        message: Message,
    ) -> Result<MessageId, AgentError> {
        let id = message.id.clone();
        let mut agent = self.lock(session_id).await?;
        agent.followup(message)?;
        Ok(id)
    }

    /// Drive until idle. Holds the agent mutex for the whole call.
    ///
    /// # Errors
    ///
    /// [`AgentError::UnknownSession`] or loop failure.
    pub async fn run_until_idle(&self, session_id: &str) -> Result<(), AgentError> {
        let mut agent = self.lock(session_id).await?;
        agent.run_until_idle().await?;
        Ok(())
    }

    /// Current whole-agent status.
    ///
    /// # Errors
    ///
    /// [`AgentError::UnknownSession`].
    pub async fn status(&self, session_id: &str) -> Result<AgentStatus, AgentError> {
        let agent = self.lock(session_id).await?;
        Ok(agent.status())
    }

    /// [`run_until_idle`](Self::run_until_idle) then assert idle.
    ///
    /// # Errors
    ///
    /// Same as `run_until_idle`.
    pub async fn when_idle(&self, session_id: &str) -> Result<(), AgentError> {
        self.run_until_idle(session_id).await?;
        debug_assert_eq!(self.status(session_id).await?, AgentStatus::Idle);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentRegistry, CreateAgentOptions};
    use dsh_agent_loop::AgentStatus;
    use dsh_llm::{LlmRuntime, MockAdapter, MockScript, text_response};
    use dsh_session::{
        ContentBlock, LogEvent, Message, MessageId, MessageRole, MessageSource, SessionEvent,
        SessionId, TurnEndReason,
    };
    use dsh_system_prompt::{SystemPrompt, SystemPromptConfig};
    use dsh_tools::{ToolPresentationMode, ToolRuntime};
    use std::sync::{Arc, Mutex};

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

    #[tokio::test]
    async fn create_followup_run_until_idle_text_only() {
        let registry = registry_with_text("hello from registry");
        let handle = registry
            .create(CreateAgentOptions {
                session_id: SessionId::new("sess-1"),
                cwd: Some("/work".into()),
                provider: "mock".into(),
                model: "mock".into(),
                max_tokens: None,
            })
            .unwrap();
        assert_eq!(handle.id().as_str(), "sess-1");
        assert!(registry.get("sess-1").is_some());
        let message_id = registry
            .followup(
                "sess-1",
                Message {
                    id: MessageId::new("m1"),
                    role: MessageRole::User,
                    content: vec![ContentBlock::Text { text: "go".into() }],
                    source: MessageSource::User,
                },
            )
            .await
            .unwrap();
        assert_eq!(message_id.as_str(), "m1");
        assert_eq!(registry.status("sess-1").await.unwrap(), AgentStatus::Idle);
        registry.when_idle("sess-1").await.unwrap();
        assert_eq!(registry.status("sess-1").await.unwrap(), AgentStatus::Idle);
        let agent = registry.get("sess-1").unwrap();
        let guard = agent.lock_for_test().await;
        let serialized = format!("{:?}", guard.session.events());
        assert!(serialized.contains("hello from registry"), "{serialized}");
        let completed = guard.session.events().iter().any(|event| {
            matches!(
                event,
                LogEvent::Known(SessionEvent::TurnEnd { data, .. })
                    if matches!(data.reason, TurnEndReason::Completed)
            )
        });
        assert!(completed);
    }

    #[allow(unused_mut)]
    #[tokio::test]
    async fn append_sink_receives_session_events() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut registry = registry_with_text("sink-hi");
        let seen_clone = Arc::clone(&seen);
        registry.set_sink_factory(Some(Arc::new(move |_id: &str| {
            let seen = Arc::clone(&seen_clone);
            Arc::new(move |event: &LogEvent| {
                seen.lock()
                    .expect("seen")
                    .push(event.event_type().to_string());
            })
        })));
        registry
            .create(CreateAgentOptions {
                session_id: SessionId::new("sess-sink"),
                cwd: None,
                provider: "mock".into(),
                model: "mock".into(),
                max_tokens: None,
            })
            .unwrap();
        registry
            .followup(
                "sess-sink",
                Message {
                    id: MessageId::new("m1"),
                    role: MessageRole::User,
                    content: vec![ContentBlock::Text { text: "go".into() }],
                    source: MessageSource::User,
                },
            )
            .await
            .unwrap();
        registry.run_until_idle("sess-sink").await.unwrap();
        let types = seen.lock().expect("seen").clone();
        assert!(types.iter().any(|t| t == "turn/end"), "{types:?}");
        assert!(types.iter().any(|t| t == "assistant/message"), "{types:?}");
    }

    #[test]
    fn unknown_session_is_an_error() {
        let registry = registry_with_text("x");
        assert!(registry.get("missing").is_none());
    }

    #[test]
    fn list_providers_includes_registered_mock() {
        let registry = registry_with_text("x");
        assert!(registry.list_providers().iter().any(|p| p == "mock"));
    }
}
