//! Session-id keyed LoopAgent map.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use dsh_agent_loop::{AgentStatus, CancelCause, CancelOptions, LoopAgent, LoopOptions};
use dsh_kernel::Context;
use dsh_llm::LlmRuntime;
use dsh_session::{
    AppendSink, Message, MessageId, SESSION_FORMAT_VERSION, Session, SessionHeader, SessionId,
};
use dsh_system_prompt::SystemPrompt;
use dsh_tools::ToolRuntime;
use tokio::sync::Mutex as AsyncMutex;

use crate::AgentError;

type SinkFactory = Arc<dyn Fn(&str) -> AppendSink + Send + Sync>;
type SessionCreateHook = Arc<dyn Fn(&mut Session) + Send + Sync>;

/// Driver permit plus session/inbox state for one live agent.
struct AgentInner {
    driver: AsyncMutex<()>,
    state: Mutex<LoopAgent>,
}

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
    inner: Arc<AgentInner>,
}

impl AgentHandle {
    /// Session identity.
    #[must_use]
    pub fn id(&self) -> &SessionId {
        &self.id
    }

    /// Lock session/inbox state. Hold only for short reads; do not hold across `.await`.
    pub fn lock(&self) -> std::sync::MutexGuard<'_, LoopAgent> {
        self.inner.state.lock().expect("loop agent state")
    }

    /// Queue `message` on next-turn without taking the driver permit.
    ///
    /// # Errors
    ///
    /// [`AgentError::Loop`] from the inbox splice.
    pub async fn followup(&self, message: Message) -> Result<(), AgentError> {
        self.inner
            .state
            .lock()
            .expect("loop agent state")
            .followup(message)?;
        Ok(())
    }

    /// Queue `message` on next-step and latch a wake, without taking the driver permit.
    ///
    /// # Errors
    ///
    /// [`AgentError::Loop`] from the inbox splice.
    pub async fn steer(&self, message: Message) -> Result<(), AgentError> {
        self.inner
            .state
            .lock()
            .expect("loop agent state")
            .steer(message)?;
        Ok(())
    }

    /// Queue `message` on next-step without latching a wake or taking the driver permit.
    ///
    /// # Errors
    ///
    /// [`AgentError::Loop`] from the inbox splice.
    pub async fn inject(&self, message: Message) -> Result<(), AgentError> {
        self.inner
            .state
            .lock()
            .expect("loop agent state")
            .inject(message)?;
        Ok(())
    }

    /// Abort the current reservation, then wait until the driver permit is free.
    ///
    /// # Errors
    ///
    /// [`AgentError::Loop`] from inbox `clear`.
    pub async fn cancel(&self) -> Result<(), AgentError> {
        self.inner
            .state
            .lock()
            .expect("loop agent state")
            .cancel(CancelCause::User, CancelOptions::default())?;
        let _driver = self.inner.driver.lock().await;
        Ok(())
    }

    /// Acquire the driver permit and drive until idle.
    ///
    /// Locks session state only around synchronous mutations and releases it
    /// across LLM stream and tool-body `.await` points.
    ///
    /// # Errors
    ///
    /// [`AgentError::Loop`] from the turn driver.
    pub async fn run_until_idle(&self) -> Result<(), AgentError> {
        let _driver = self.inner.driver.lock().await;
        LoopAgent::run_until_idle_locked(&self.inner.state).await?;
        Ok(())
    }
}

/// Live agents keyed by session id string.
pub struct AgentRegistry {
    ctx: Context,
    agents: Mutex<HashMap<String, AgentHandle>>,
    llm: Arc<Mutex<LlmRuntime>>,
    tools: Arc<Mutex<ToolRuntime>>,
    prompt: SystemPrompt,
    sink_factory: Mutex<Option<SinkFactory>>,
    session_create: Mutex<Vec<SessionCreateHook>>,
}

impl AgentRegistry {
    /// Empty registry wrapping owned `llm` / `tools` in new mutexes.
    ///
    /// Tests that do not share kernel services use this constructor.
    /// [`create`](Self::create) stores the same mutex Arcs on each `LoopAgent`.
    #[must_use]
    pub fn new(llm: LlmRuntime, tools: ToolRuntime, prompt: SystemPrompt) -> Self {
        Self::from_shared(
            Context::new(),
            Arc::new(Mutex::new(llm)),
            Arc::new(Mutex::new(tools)),
            prompt,
        )
    }

    /// Hold the kernel's `llm` and `tools` mutexes by Arc.
    ///
    /// `list_providers`, [`llm`](Self::llm), and [`tools`](Self::tools) lock those
    /// mutexes, so sibling `register_adapter` / `register` on the same mutexes stays
    /// visible. [`create`](Self::create) stores the same mutex Arcs on each `LoopAgent`.
    #[must_use]
    pub fn from_shared(
        ctx: Context,
        llm: Arc<Mutex<LlmRuntime>>,
        tools: Arc<Mutex<ToolRuntime>>,
        prompt: SystemPrompt,
    ) -> Self {
        Self {
            ctx,
            agents: Mutex::new(HashMap::new()),
            llm,
            tools,
            prompt,
            sink_factory: Mutex::new(None),
            session_create: Mutex::new(Vec::new()),
        }
    }

    /// Run `hook` on a newly built session before [`LoopAgent::new`].
    pub fn on_session_create(&self, hook: Arc<dyn Fn(&mut Session) + Send + Sync>) {
        self.session_create
            .lock()
            .expect("session create")
            .push(hook);
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

    /// Clone the shared tools mutex. Continuable children pass a distinct mutex into [`resume_with_tools`].
    #[must_use]
    pub fn tools_arc(&self) -> Arc<Mutex<ToolRuntime>> {
        Arc::clone(&self.tools)
    }

    /// Install a per-session append sink factory. `None` clears it.
    pub fn set_sink_factory(&self, factory: Option<SinkFactory>) {
        *self.sink_factory.lock().expect("sink") = factory;
    }

    /// Create or return the live handle for `opts.session_id`.
    ///
    /// # Errors
    ///
    /// [`AgentError::Loop`] when inbox replay of the session fails.
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
        for hook in self.session_create.lock().expect("session create").iter() {
            hook(&mut session);
        }
        let loop_agent = LoopAgent::new(
            self.ctx.clone(),
            session,
            LoopOptions {
                provider: opts.provider,
                model: opts.model,
                max_tokens: opts.max_tokens,
                max_parallel_tool_calls: dsh_agent_loop::DEFAULT_MAX_PARALLEL_TOOL_CALLS,
            },
            Arc::clone(&self.tools),
            self.prompt.clone(),
            Arc::clone(&self.llm),
        )?;
        let handle = AgentHandle {
            id: opts.session_id,
            inner: Arc::new(AgentInner {
                driver: AsyncMutex::new(()),
                state: Mutex::new(loop_agent),
            }),
        };
        self.agents
            .lock()
            .expect("agents")
            .insert(key, handle.clone());
        Ok(handle)
    }

    /// Resume `session` or return the live handle when `session.id()` is already registered.
    ///
    /// `options.session_id` must equal `session.id()`. `on_session_create` hooks run;
    /// permission pinning is idempotent on an existing log.
    ///
    /// # Errors
    ///
    /// [`AgentError::ResumeIdMismatch`] when the ids differ.
    /// [`AgentError::Loop`] when inbox replay of the loaded session fails.
    pub fn resume(
        &self,
        session: Session,
        options: CreateAgentOptions,
    ) -> Result<AgentHandle, AgentError> {
        self.resume_with_tools(session, options, None)
    }

    /// Resume `session` with an optional child-local tool runtime.
    ///
    /// `None` uses the registry's shared tools mutex. Continuable children pass a
    /// clone so `report` can register without appearing on the parent runtime.
    ///
    /// # Errors
    ///
    /// [`AgentError::ResumeIdMismatch`] when the ids differ.
    /// [`AgentError::Loop`] when inbox replay of the loaded session fails.
    pub fn resume_with_tools(
        &self,
        mut session: Session,
        options: CreateAgentOptions,
        tools: Option<Arc<Mutex<ToolRuntime>>>,
    ) -> Result<AgentHandle, AgentError> {
        if session.id().as_str() != options.session_id.as_str() {
            return Err(AgentError::ResumeIdMismatch(
                session.id().as_str().to_string(),
                options.session_id.as_str().to_string(),
            ));
        }
        let key = options.session_id.as_str().to_string();
        {
            let agents = self.agents.lock().expect("agents");
            if let Some(existing) = agents.get(&key) {
                return Ok(existing.clone());
            }
        }
        if let Some(factory) = self.sink_factory.lock().expect("sink").clone() {
            session.set_append_sink(Some(factory(options.session_id.as_str())));
        }
        for hook in self.session_create.lock().expect("session create").iter() {
            hook(&mut session);
        }
        let tools = tools.unwrap_or_else(|| Arc::clone(&self.tools));
        let loop_agent = LoopAgent::new(
            self.ctx.clone(),
            session,
            LoopOptions {
                provider: options.provider,
                model: options.model,
                max_tokens: options.max_tokens,
                max_parallel_tool_calls: dsh_agent_loop::DEFAULT_MAX_PARALLEL_TOOL_CALLS,
            },
            tools,
            self.prompt.clone(),
            Arc::clone(&self.llm),
        )?;
        let handle = AgentHandle {
            id: options.session_id,
            inner: Arc::new(AgentInner {
                driver: AsyncMutex::new(()),
                state: Mutex::new(loop_agent),
            }),
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
        let handle = self
            .get(session_id)
            .ok_or_else(|| AgentError::UnknownSession(session_id.to_string()))?;
        handle.followup(message).await?;
        Ok(id)
    }

    /// Drive until idle. Acquires the driver permit for the whole call.
    ///
    /// # Errors
    ///
    /// [`AgentError::UnknownSession`] or loop failure.
    pub async fn run_until_idle(&self, session_id: &str) -> Result<(), AgentError> {
        let handle = self
            .get(session_id)
            .ok_or_else(|| AgentError::UnknownSession(session_id.to_string()))?;
        handle.run_until_idle().await
    }

    /// Current whole-agent status.
    ///
    /// # Errors
    ///
    /// [`AgentError::UnknownSession`].
    pub async fn status(&self, session_id: &str) -> Result<AgentStatus, AgentError> {
        let handle = self
            .get(session_id)
            .ok_or_else(|| AgentError::UnknownSession(session_id.to_string()))?;
        Ok(handle.lock().status())
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
        let guard = agent.lock();
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

    #[tokio::test]
    async fn append_sink_receives_session_events() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let registry = registry_with_text("sink-hi");
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

    #[test]
    fn session_create_hook_runs_before_loop_agent_is_published() {
        let registry = registry_with_text("unused");
        registry.on_session_create(Arc::new(|session| {
            session
                .append(SessionEvent::AgentInboxSpliced {
                    seq: 0,
                    time: 0,
                    data: dsh_session::InboxSplicedData {
                        target: dsh_session::InboxTarget::NextTurn,
                        start: 0,
                        removed_count: None,
                        inserted: vec![Message {
                            id: MessageId::new("from-hook"),
                            role: MessageRole::User,
                            content: vec![ContentBlock::Text {
                                text: "hook-message".into(),
                            }],
                            source: MessageSource::User,
                        }],
                        outcome: None,
                    },
                    ignorable: None,
                })
                .expect("hook append");
        }));
        let handle = registry
            .create(CreateAgentOptions {
                session_id: SessionId::new("sess-hook"),
                cwd: None,
                provider: "mock".into(),
                model: "mock".into(),
                max_tokens: None,
            })
            .unwrap();
        let agent = handle.lock();
        assert!(
            agent
                .inbox
                .next_turn()
                .iter()
                .any(|message| message.id.as_str() == "from-hook"),
            "LoopAgent::new must replay splices the session-create hook appended"
        );
    }

    fn inbox_contains(handle: &super::AgentHandle, text: &str) -> bool {
        let agent = handle.lock();
        agent
            .inbox
            .next_turn()
            .iter()
            .chain(agent.inbox.next_step().iter())
            .any(|message| {
                message.content.iter().any(|block| match block {
                    ContentBlock::Text { text: body } => body == text,
                    _ => false,
                })
            })
    }

    fn user_text(id: &str, text: &str) -> Message {
        Message {
            id: MessageId::new(id),
            role: MessageRole::User,
            content: vec![ContentBlock::Text { text: text.into() }],
            source: MessageSource::User,
        }
    }

    async fn hanging_tool_agent() -> super::AgentHandle {
        use dsh_llm::tool_call_response;
        use dsh_tools::ToolDefinition;
        let adapter = Arc::new(MockAdapter::new(vec![MockScript::Chunks(
            tool_call_response("c1", "hang", &serde_json::json!({}), None),
        )]));
        let mut llm = LlmRuntime::new();
        llm.register_adapter("mock", adapter);
        let mut tools = ToolRuntime::new(ToolPresentationMode::Native);
        tools.register(ToolDefinition {
            name: "hang".into(),
            description: "never returns".into(),
            parameters: serde_json::json!({"type": "object"}),
            execute: Box::new(|_args, exec| {
                Box::pin(async move {
                    exec.signal.cancelled().await;
                    Err(dsh_tools::ToolError::Other("hung until cancel".into()))
                })
            }),
            render: Box::new(|_, _| Vec::new()),
            is_concurrency_safe: None,
        });
        let registry = AgentRegistry::new(
            llm,
            tools,
            SystemPrompt::new(SystemPromptConfig::default()).unwrap(),
        );
        let handle = registry
            .create(CreateAgentOptions {
                session_id: SessionId::new("hang-1"),
                cwd: None,
                provider: "mock".into(),
                model: "mock".into(),
                max_tokens: None,
            })
            .unwrap();
        handle.followup(user_text("go", "go")).await.unwrap();
        handle
    }

    #[tokio::test]
    async fn followup_during_tool_await_does_not_need_driver_permit() {
        let agent = hanging_tool_agent().await;
        let handle = agent.clone();
        let driver = tokio::spawn(async move { handle.run_until_idle().await });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        agent
            .followup(user_text("settlement", "settlement"))
            .await
            .unwrap();
        assert!(inbox_contains(&agent, "settlement"));
        agent.cancel().await.unwrap();
        let _ = driver.await;
    }

    #[test]
    fn from_shared_sees_adapter_registered_on_the_same_mutex() {
        let llm = Arc::new(Mutex::new(LlmRuntime::new()));
        let tools = Arc::new(Mutex::new(ToolRuntime::new(ToolPresentationMode::Native)));
        let registry = AgentRegistry::from_shared(
            dsh_kernel::Context::new(),
            Arc::clone(&llm),
            Arc::clone(&tools),
            SystemPrompt::new(SystemPromptConfig::default()).unwrap(),
        );
        assert!(
            registry.list_providers().is_empty(),
            "{:?}",
            registry.list_providers()
        );
        llm.lock().expect("llm").register_adapter(
            "mock",
            Arc::new(MockAdapter::new(vec![MockScript::Chunks(text_response(
                "x",
            ))])),
        );
        assert!(
            registry.list_providers().iter().any(|p| p == "mock"),
            "{:?}",
            registry.list_providers()
        );
    }
}
