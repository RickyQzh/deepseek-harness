//! Idle / maintenance / running phase machine, sticky turn reasons, and tool dispatch.

use std::future::Future;
use std::sync::{Arc, Mutex};

use dsh_kernel::{Context, Next};
use dsh_llm::retry::RetryScope;
use dsh_llm::{BlockAssembler, GenerateOptions, LlmRuntime};
use dsh_session::{
    AssistantChunkData, AssistantMessageData, ContentBlock, EpochHeader, FinishReason, InboxTarget,
    LlmCallConfig, LlmFailure, LogEvent, Message, RequestHeaderData, RequestHeaderReason, Session,
    SessionEvent, SessionId, StepBoundaryData, StreamChunk, SurfaceOp, TurnEndData, TurnEndReason,
    TurnStartData, canonical_header, header_equals,
};
use dsh_system_prompt::{
    AssembleContext, PromptAssembly, SystemPrompt, join_context_sections, render_context_sections,
    render_prompt,
};
use dsh_tools::{AbortFlag, ToolRuntime};
use futures::StreamExt;

use crate::compaction_scope::CompactionScope;
use crate::error::LoopError;
use crate::inbox::Inbox;
use crate::runtime_context::RuntimeContextProjection;
use crate::tool_calls::{ToolCallHost, ToolCallsOutcome};

#[cfg(test)]
pub use crate::RUNTIME_CONTEXT_SOURCE;

/// Default maximum in-flight parallel-safe tool calls per agent step.
pub const DEFAULT_MAX_PARALLEL_TOOL_CALLS: usize = 10;

/// Kernel waterfall name for pre-step admission.
pub const EVENT_AGENT_PRE_STEP: &str = "agent/pre-step";
/// Kernel waterfall name for model-request error recovery.
pub use dsh_llm::retry::EVENT_AGENT_REQUEST_ERROR;
/// Recovery choice after a terminal model-request error or abort finish.
pub use dsh_llm::retry::RequestErrorAction;

type RequestListener = Box<dyn Fn(&LlmCallConfig) -> LlmCallConfig + Send + Sync>;

/// Externally visible activity: maintenance reports idle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentStatus {
    /// No running turn driver.
    Idle,
    /// A turn driver holds the agent.
    Running,
}

/// Internal reservation: idle, a maintenance job, or a turn driver.
#[derive(Clone, Debug)]
pub enum Phase {
    /// No driver is reserved.
    Idle {
        /// Last completed turn number, or 0.
        last_turn: u64,
    },
    /// A maintenance job holds the agent; waking input latches until it returns.
    Maintenance {
        /// Cancellation flag for the job.
        abort: AbortFlag,
        /// Last completed turn number.
        last_turn: u64,
        /// Whether `followup`/`steer` latched a wake during the job.
        wake_requested: bool,
    },
    /// A turn driver holds the agent.
    Running {
        /// Cancellation flag for the current turn.
        abort: AbortFlag,
        /// Current turn number (0 before the first `turn/start`).
        turn: u64,
        /// Current step number (0 before the first `step/start`).
        step: u64,
        /// Whether an aborted running driver should replay a wake.
        wake_requested: bool,
    },
}

/// Why [`LoopAgent::cancel`] aborted work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelCause {
    /// Explicit user cancel.
    User,
    /// Agent disposal.
    Disposed,
}

/// Options for [`LoopAgent::cancel`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CancelOptions {
    /// When true, pending inbox messages are kept.
    pub keep_inbox: bool,
}

/// Pre-step waterfall result: enter the step with these messages, or reject the turn.
#[derive(Clone, Debug, PartialEq)]
pub enum PreStepDecision {
    /// Proceed with these user-role messages (claimed input plus optional snapshot).
    Enter {
        /// Messages to append as `user/message` before the model call.
        messages: Vec<Message>,
    },
    /// End the turn as blocked without a step.
    Reject,
}

/// Extra fields a pre-step listener may read. The waterfall value is [`PreStepDecision`].
#[derive(Clone, Debug)]
pub struct PreStepPayload {
    /// Claimed user-role messages plus an optional runtime-context snapshot.
    pub messages: Vec<Message>,
    /// Open turn number.
    pub turn: u64,
    /// Step that would be entered; `0` before the first step of the turn.
    pub step: u64,
}

/// Extra fields a request-error listener may read. The waterfall value is [`RequestErrorAction`].
#[derive(Clone, Debug)]
pub struct RequestErrorPayload {
    /// Turn of the failed request.
    pub turn: u64,
    /// Step of the failed request.
    pub step: u64,
    /// Provider route of the failed request.
    pub provider: String,
    /// Terminal model-request failure.
    pub failure: LlmFailure,
}

/// Route and scheduler defaults for one loop instance.
#[derive(Clone, Debug)]
pub struct LoopOptions {
    /// Provider route selecting the adapter.
    pub provider: String,
    /// Model id passed to the adapter.
    pub model: String,
    /// Output token cap; omitted when the adapter should apply its default.
    pub max_tokens: Option<u64>,
    /// Maximum in-flight parallel-safe tool calls per step.
    pub max_parallel_tool_calls: usize,
}

impl Default for LoopOptions {
    fn default() -> Self {
        Self {
            provider: "mock".into(),
            model: "mock".into(),
            max_tokens: None,
            max_parallel_tool_calls: DEFAULT_MAX_PARALLEL_TOOL_CALLS,
        }
    }
}

/// Drop adapter-default `reasoning_effort` and `max_tokens` before the next seed.
fn request_proposal(header: &EpochHeader) -> LlmCallConfig {
    let mut proposal = header.config.clone();
    if header
        .adapter_defaults
        .as_ref()
        .and_then(|d| d.reasoning_effort)
        == Some(true)
    {
        proposal.reasoning_effort = None;
    }
    if header.adapter_defaults.as_ref().and_then(|d| d.max_tokens) == Some(true) {
        proposal.max_tokens = None;
    }
    proposal
}

/// Scripted driver over one session's inbox, prompt, tools, and LLM runtime.
pub struct LoopAgent {
    /// Session identity copied from the session header.
    pub id: SessionId,
    /// Route and scheduler defaults.
    pub options: LoopOptions,
    /// Append-only session log.
    pub session: Session,
    /// Durable pending-message projection.
    pub inbox: Inbox,
    /// Shared tool registry used by tool-call scheduling.
    pub tools: Arc<Mutex<ToolRuntime>>,
    /// System prompt and runtime-context assembly.
    pub prompt: SystemPrompt,
    /// Shared adapter registry used to stream model output.
    pub llm: Arc<Mutex<LlmRuntime>>,
    ctx: Context,
    phase: Phase,
    request_header_logged: bool,
    runtime_context: RuntimeContextProjection,
    on_request: Vec<RequestListener>,
    wake_latched: bool,
    cancel_cause: Option<CancelCause>,
}

impl LoopAgent {
    /// Replay inbox and runtime-context state, then start idle.
    ///
    /// # Errors
    ///
    /// [`LoopError::Invalid`] when persisted inbox splices do not apply.
    pub fn new(
        ctx: Context,
        session: Session,
        options: LoopOptions,
        tools: Arc<Mutex<ToolRuntime>>,
        prompt: SystemPrompt,
        llm: Arc<Mutex<LlmRuntime>>,
    ) -> Result<Self, LoopError> {
        let inbox = Inbox::replay(&session)?;
        let runtime_context = RuntimeContextProjection::replay(&session);
        let last_turn = last_turn_from(&session);
        Ok(Self {
            id: session.id().clone(),
            options,
            inbox,
            tools,
            prompt,
            llm,
            ctx,
            phase: Phase::Idle { last_turn },
            request_header_logged: false,
            runtime_context,
            on_request: Vec::new(),
            wake_latched: false,
            cancel_cause: None,
            session,
        })
    }

    /// Idle and maintenance report [`AgentStatus::Idle`]; running reports [`AgentStatus::Running`].
    #[must_use]
    pub fn status(&self) -> AgentStatus {
        match self.phase {
            Phase::Idle { .. } | Phase::Maintenance { .. } => AgentStatus::Idle,
            Phase::Running { .. } => AgentStatus::Running,
        }
    }

    /// Current phase reservation.
    #[must_use]
    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    /// Queue `message` on next-turn and latch a wake. Does not start a driver.
    ///
    /// # Errors
    ///
    /// [`LoopError::Invalid`] or [`LoopError::Session`] from the inbox splice.
    pub fn followup(&mut self, message: Message) -> Result<(), LoopError> {
        self.send(message, InboxTarget::NextTurn, true)
    }

    /// Queue `message` on next-step and latch a wake. Does not start a driver.
    ///
    /// # Errors
    ///
    /// [`LoopError::Invalid`] or [`LoopError::Session`] from the inbox splice.
    pub fn steer(&mut self, message: Message) -> Result<(), LoopError> {
        self.send(message, InboxTarget::NextStep, true)
    }

    /// Queue `message` on next-step without latching a wake.
    ///
    /// # Errors
    ///
    /// [`LoopError::Invalid`] or [`LoopError::Session`] from the inbox splice.
    pub fn inject(&mut self, message: Message) -> Result<(), LoopError> {
        self.send(message, InboxTarget::NextStep, false)
    }

    /// Abort the current reservation. Clears the inbox unless [`CancelOptions::keep_inbox`].
    ///
    /// # Errors
    ///
    /// [`LoopError::Invalid`] or [`LoopError::Session`] from inbox `clear`.
    pub fn cancel(&mut self, cause: CancelCause, options: CancelOptions) -> Result<(), LoopError> {
        self.cancel_cause = Some(cause);
        if !options.keep_inbox {
            self.inbox.clear(&mut self.session)?;
            if let Phase::Maintenance { wake_requested, .. }
            | Phase::Running { wake_requested, .. } = &mut self.phase
            {
                *wake_requested = false;
            }
        }
        match &self.phase {
            Phase::Idle { .. } => {}
            Phase::Maintenance { abort, .. } | Phase::Running { abort, .. } => abort.abort(),
        }
        Ok(())
    }

    /// Register a kernel waterfall listener on [`EVENT_AGENT_PRE_STEP`].
    ///
    /// A listener that returns without calling `next` short-circuits. An empty
    /// listener list keeps claimed messages plus an optional snapshot.
    pub fn on_pre_step<F, Fut>(&self, listener: F)
    where
        F: Fn(PreStepDecision, Next<PreStepDecision>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = PreStepDecision> + Send + 'static,
    {
        let _ = self.ctx.on_waterfall(EVENT_AGENT_PRE_STEP, listener);
    }

    /// Append a request-config listener. Listeners run in registration order over the seed config.
    pub fn on_request(
        &mut self,
        listener: impl Fn(&LlmCallConfig) -> LlmCallConfig + Send + Sync + 'static,
    ) {
        self.on_request.push(Box::new(listener));
    }

    /// Register a kernel waterfall listener on [`EVENT_AGENT_REQUEST_ERROR`].
    ///
    /// A listener that returns without calling `next` owns recovery. The default
    /// seed is [`RequestErrorAction::Fail`].
    pub fn on_request_error<F, Fut>(&self, listener: F)
    where
        F: Fn(RequestErrorAction, Next<RequestErrorAction>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = RequestErrorAction> + Send + 'static,
    {
        let _ = self.ctx.on_waterfall(EVENT_AGENT_REQUEST_ERROR, listener);
    }

    /// Run `job` while the agent is reserved as maintenance. A latched wake is consumed on return
    /// when `wake_requested && inbox.has_pending()`.
    ///
    /// # Errors
    ///
    /// [`LoopError::Invalid`] when the agent is not idle.
    pub async fn run_maintenance<F, Fut, T>(&mut self, job: F) -> Result<T, LoopError>
    where
        F: FnOnce(AbortFlag) -> Fut,
        Fut: Future<Output = T>,
    {
        let Phase::Idle { last_turn } = self.phase else {
            return Err(self.active_work_error());
        };
        let abort = AbortFlag::new();
        self.phase = Phase::Maintenance {
            abort: abort.clone(),
            last_turn,
            wake_requested: false,
        };
        let result = job(abort).await;
        let (last_turn, wake_requested) = match &self.phase {
            Phase::Maintenance {
                last_turn,
                wake_requested,
                ..
            } => (*last_turn, *wake_requested),
            Phase::Idle { last_turn } => (*last_turn, false),
            Phase::Running { turn, .. } => (*turn, false),
        };
        self.phase = Phase::Idle { last_turn };
        if wake_requested && self.inbox.has_pending() {
            self.wake_latched = true;
        }
        Ok(result)
    }

    /// Drive turns until idle. Queue methods never spawn this driver.
    ///
    /// Starts from idle when next-turn has work or a wake is latched. An empty first enter still
    /// logs `turn/start` and `turn/end` completed without a step.
    ///
    /// # Errors
    ///
    /// [`LoopError::Invalid`] when a driver is already reserved, or from inbox/step failures.
    /// [`LoopError::Session`] when a log append fails.
    /// [`LoopError::Prompt`] when prompt assembly or render fails.
    pub async fn run_until_idle(&mut self) -> Result<(), LoopError> {
        drive(&mut DriveTarget::Exclusive(self)).await
    }

    /// Drive turns while locking `state` only around synchronous session mutations.
    ///
    /// Releases `state` across the pre-step waterfall, the LLM stream, the
    /// request-error waterfall, and tool-body `.await` points so a concurrent
    /// [`Self::followup`] can splice the inbox.
    ///
    /// # Errors
    ///
    /// Same as [`Self::run_until_idle`].
    pub async fn run_until_idle_locked(state: &Mutex<Self>) -> Result<(), LoopError> {
        drive(&mut DriveTarget::Shared(state)).await
    }

    fn send(
        &mut self,
        message: Message,
        target: InboxTarget,
        wakeup: bool,
    ) -> Result<(), LoopError> {
        let waking_after_abort = wakeup && self.is_aborted();
        let resolved = if waking_after_abort {
            InboxTarget::NextTurn
        } else {
            target
        };
        let start = match resolved {
            InboxTarget::NextTurn => self.inbox.next_turn().len(),
            InboxTarget::NextStep => self.inbox.next_step().len(),
        };
        self.inbox
            .splice(&mut self.session, resolved, start, 0, vec![message], true)?;
        if wakeup {
            self.latch_wake(waking_after_abort);
        }
        Ok(())
    }

    fn latch_wake(&mut self, wake_after_abort: bool) {
        let disposed = self.cancel_cause == Some(CancelCause::Disposed);
        match &mut self.phase {
            Phase::Idle { .. } => {
                self.wake_latched = true;
            }
            Phase::Maintenance { wake_requested, .. } => {
                if !disposed {
                    *wake_requested = true;
                }
            }
            Phase::Running {
                abort,
                wake_requested,
                ..
            } => {
                if abort.is_aborted() && wake_after_abort && !disposed {
                    *wake_requested = true;
                }
            }
        }
    }

    fn is_aborted(&self) -> bool {
        match &self.phase {
            Phase::Idle { .. } => false,
            Phase::Maintenance { abort, .. } | Phase::Running { abort, .. } => abort.is_aborted(),
        }
    }

    fn active_work_error(&self) -> LoopError {
        LoopError::Invalid(format!(
            "agent \"{}\" already has active work",
            self.id.as_str()
        ))
    }

    fn begin_turn(&mut self) -> Result<u64, LoopError> {
        let Phase::Running { turn, abort, .. } = &self.phase else {
            return Err(LoopError::Invalid(format!(
                "agent \"{}\": turn without driver reservation",
                self.id.as_str()
            )));
        };
        if abort.is_aborted() {
            return Err(LoopError::Invalid(format!(
                "agent \"{}\" aborted",
                self.id.as_str()
            )));
        }
        let turn = *turn + 1;
        if let Phase::Running { turn: slot, .. } = &mut self.phase {
            *slot = turn;
        }
        self.push_event(|seq| SessionEvent::TurnStart {
            seq,
            time: seq as i64,
            data: TurnStartData { turn },
            ignorable: None,
        })?;
        Ok(turn)
    }

    fn claim_pre_step(
        &mut self,
        target: InboxTarget,
    ) -> Result<(Context, PreStepDecision, PromptAssembly), LoopError> {
        let turn = match &self.phase {
            Phase::Running { turn, .. } => *turn,
            _ => {
                return Err(LoopError::Invalid(format!(
                    "agent \"{}\": pre-step outside running phase",
                    self.id.as_str()
                )));
            }
        };
        let claimed = self.inbox.claim(&mut self.session, target, turn)?;
        let assembly = self
            .prompt
            .assemble(&AssembleContext::default())
            .map_err(|error| LoopError::Prompt(error.to_string()))?;
        let sections = render_context_sections(&assembly)
            .map_err(|error| LoopError::Prompt(error.to_string()))?;
        let current = join_context_sections(&sections);
        let snapshot = self.runtime_context.project(&current, &sections);
        let mut messages = claimed;
        if let Some(snapshot) = snapshot {
            messages.push(snapshot);
        }
        Ok((
            self.ctx.clone(),
            PreStepDecision::Enter { messages },
            assembly,
        ))
    }

    fn open_step(&mut self, turn: u64, messages: Vec<Message>) -> Result<u64, LoopError> {
        let step = match &mut self.phase {
            Phase::Running { step, .. } => {
                *step += 1;
                *step
            }
            _ => {
                return Err(LoopError::Invalid(format!(
                    "agent \"{}\": step outside running phase",
                    self.id.as_str()
                )));
            }
        };
        self.push_event(|seq| SessionEvent::StepStart {
            seq,
            time: seq as i64,
            data: StepBoundaryData { turn, step },
            ignorable: None,
        })?;
        for message in messages {
            self.push_event(|seq| SessionEvent::UserMessage {
                seq,
                time: seq as i64,
                data: message,
                surface_op: Some(SurfaceOp::Append),
                source_event_seqs: None,
                ignorable: None,
            })?;
        }
        Ok(step)
    }

    fn ingest_chunks(
        &mut self,
        turn: u64,
        step: u64,
        chunks: Vec<StreamChunk>,
    ) -> Result<StreamIngest, LoopError> {
        let mut assembler = BlockAssembler::new();
        let mut chunk_seqs = Vec::new();
        for chunk in chunks {
            let seq = self.push_event(|seq| SessionEvent::AssistantChunk {
                seq,
                time: seq as i64,
                data: AssistantChunkData {
                    turn,
                    step,
                    chunk: chunk.clone(),
                },
                ignorable: None,
            })?;
            chunk_seqs.push(seq);
            assembler.push(chunk);
        }
        match assembler.finish() {
            FinishReason::Error { failure } | FinishReason::Aborted { failure } => {
                Ok(StreamIngest::Failed(failure))
            }
            finish => Ok(StreamIngest::Ready {
                assembler,
                chunk_seqs,
                finish,
            }),
        }
    }

    fn reset_running_for_next_turn(&mut self) {
        if let Phase::Running {
            abort,
            step,
            wake_requested,
            ..
        } = &mut self.phase
        {
            *abort = AbortFlag::new();
            *step = 0;
            *wake_requested = false;
        }
    }

    fn settle_driver(&mut self) -> (u64, bool) {
        match &self.phase {
            Phase::Running {
                turn,
                wake_requested,
                ..
            } => (*turn, *wake_requested),
            Phase::Idle { last_turn } => (*last_turn, false),
            Phase::Maintenance { last_turn, .. } => (*last_turn, false),
        }
    }

    /// Compose one frozen request from step-boundary messages and the header fold.
    fn build_request(
        &mut self,
        turn: u64,
        step: u64,
        tools: &[dsh_llm::ToolSchema],
        system: &str,
        boundary_messages: Vec<Message>,
    ) -> Result<GenerateOptions, LoopError> {
        let _ = (turn, step);
        let mut config = if self.request_header_logged {
            request_proposal(&self.session.request_header().unwrap())
        } else {
            let persisted = self.session.request_header();
            let reasoning_effort = persisted.as_ref().and_then(|header| {
                if header.config.provider == self.options.provider
                    && header.config.model == self.options.model
                    && header
                        .adapter_defaults
                        .as_ref()
                        .and_then(|defaults| defaults.reasoning_effort)
                        != Some(true)
                {
                    header.config.reasoning_effort.clone()
                } else {
                    None
                }
            });
            LlmCallConfig {
                provider: self.options.provider.clone(),
                model: self.options.model.clone(),
                reasoning_effort,
                temperature: None,
                max_tokens: self.options.max_tokens,
                stop: None,
            }
        };
        for listener in &self.on_request {
            config = listener(&config);
        }
        let prepared = self
            .llm
            .lock()
            .expect("llm")
            .prepare_call(&config)
            .map_err(|error| LoopError::Invalid(error.to_string()))?;
        let header = canonical_header(&EpochHeader {
            config: prepared.config.clone(),
            adapter_defaults: Some(prepared.adapter_defaults),
            system: (!system.is_empty()).then(|| system.to_string()),
            tools: if tools.is_empty() {
                None
            } else {
                serde_json::to_value(tools).ok()
            },
        });
        let baseline = self.session.request_header();
        if !self.request_header_logged {
            let reason = if baseline.is_none() {
                RequestHeaderReason::Initial
            } else {
                RequestHeaderReason::Resume
            };
            self.push_event(|seq| SessionEvent::RequestHeader {
                seq,
                time: seq as i64,
                data: RequestHeaderData {
                    header: header.clone(),
                    reason,
                },
                ignorable: None,
            })?;
            self.request_header_logged = true;
        } else if baseline
            .as_ref()
            .is_none_or(|baseline| !header_equals(baseline, &header))
        {
            self.push_event(|seq| SessionEvent::RequestHeader {
                seq,
                time: seq as i64,
                data: RequestHeaderData {
                    header: header.clone(),
                    reason: RequestHeaderReason::Change,
                },
                ignorable: None,
            })?;
        }
        let signal = match &self.phase {
            Phase::Running { abort, .. } | Phase::Maintenance { abort, .. } => abort.clone(),
            Phase::Idle { .. } => AbortFlag::new(),
        };
        Ok(GenerateOptions {
            provider: header.config.provider.clone(),
            model: header.config.model.clone(),
            reasoning_effort: header.config.reasoning_effort.clone(),
            messages: boundary_messages,
            system: header.system.clone(),
            tools: (!tools.is_empty()).then(|| tools.to_vec()),
            temperature: header.config.temperature,
            max_tokens: header.config.max_tokens,
            stop: header.config.stop.clone(),
            signal,
            session_id: Some(self.session.id().clone()),
            purpose: None,
        })
    }

    fn append_assistant(
        &mut self,
        turn: u64,
        step: u64,
        assembler: &BlockAssembler,
        chunk_seqs: &[u64],
        provider: &str,
        model: &str,
    ) -> Result<(), LoopError> {
        let message = assembler.message(dsh_session::MessageSource::Model {
            provider: provider.to_string(),
            model: model.to_string(),
            replay_state: assembler.replay_state().cloned(),
        });
        self.push_event(|seq| SessionEvent::AssistantMessage {
            seq,
            time: seq as i64,
            data: AssistantMessageData {
                turn,
                step,
                message,
                usage: assembler.usage().cloned(),
            },
            surface_op: Some(SurfaceOp::Append),
            source_event_seqs: Some(chunk_seqs.to_vec()),
            ignorable: None,
        })?;
        Ok(())
    }

    fn cancel_reason_json(&self) -> serde_json::Value {
        match self.cancel_cause {
            Some(CancelCause::Disposed) => serde_json::json!({"kind": "disposed"}),
            Some(CancelCause::User) | None => serde_json::json!({"kind": "user"}),
        }
    }

    fn push_event(&mut self, build: impl FnOnce(u64) -> SessionEvent) -> Result<u64, LoopError> {
        crate::append_event(&mut self.session, build)?;
        let seq = crate::next_seq(&self.session).saturating_sub(1);
        if let Some(LogEvent::Known(event)) = self.session.events().last() {
            let event = event.clone();
            self.runtime_context.observe(&event);
        }
        Ok(seq)
    }
}

enum DriveTarget<'a> {
    Exclusive(&'a mut LoopAgent),
    Shared(&'a Mutex<LoopAgent>),
}

enum StreamIngest {
    Failed(LlmFailure),
    Ready {
        assembler: BlockAssembler,
        chunk_seqs: Vec<u64>,
        finish: FinishReason,
    },
}

struct SharedHost<'a> {
    state: &'a Mutex<LoopAgent>,
    tools: Arc<Mutex<ToolRuntime>>,
}

impl ToolCallHost for SharedHost<'_> {
    fn with_session<R>(&mut self, f: impl FnOnce(&mut Session) -> R) -> R {
        let mut agent = self.state.lock().expect("loop agent state");
        f(&mut agent.session)
    }

    fn tools(&self) -> &Arc<Mutex<ToolRuntime>> {
        &self.tools
    }
}

impl DriveTarget<'_> {
    fn with<R>(&mut self, f: impl FnOnce(&mut LoopAgent) -> R) -> R {
        match self {
            DriveTarget::Exclusive(agent) => f(agent),
            DriveTarget::Shared(state) => {
                let mut agent = state.lock().expect("loop agent state");
                f(&mut agent)
            }
        }
    }

    fn compaction_scope(&mut self) -> CompactionScope {
        match self {
            DriveTarget::Exclusive(agent) => {
                let abort = running_abort(agent);
                let options = agent.options.clone();
                CompactionScope::exclusive(&mut agent.session, abort, options)
            }
            DriveTarget::Shared(state) => {
                let state: &Mutex<LoopAgent> = state;
                let (abort, options) = {
                    let agent = state.lock().expect("loop agent state");
                    (running_abort(&agent), agent.options.clone())
                };
                CompactionScope::shared(state, abort, options)
            }
        }
    }

    async fn execute_tools(
        &mut self,
        turn: u64,
        step: u64,
        tool_calls: &[ContentBlock],
        signal: &AbortFlag,
        max_parallel: usize,
        extra: &mut Vec<Message>,
    ) -> Result<ToolCallsOutcome, LoopError> {
        match self {
            DriveTarget::Exclusive(agent) => {
                crate::tool_calls::execute_tool_calls(
                    &mut crate::tool_calls::DirectHost {
                        session: &mut agent.session,
                        tools: &agent.tools,
                    },
                    turn,
                    step,
                    tool_calls,
                    signal,
                    max_parallel,
                    &mut |message| extra.push(message),
                )
                .await
            }
            DriveTarget::Shared(state) => {
                let tools = {
                    let agent = state.lock().expect("loop agent state");
                    Arc::clone(&agent.tools)
                };
                crate::tool_calls::execute_tool_calls(
                    &mut SharedHost {
                        state: *state,
                        tools,
                    },
                    turn,
                    step,
                    tool_calls,
                    signal,
                    max_parallel,
                    &mut |message| extra.push(message),
                )
                .await
            }
        }
    }
}

async fn drive(target: &mut DriveTarget<'_>) -> Result<(), LoopError> {
    if target.with(|agent| !matches!(agent.phase, Phase::Idle { .. })) {
        return Err(target.with(|agent| agent.active_work_error()));
    }
    loop {
        let should_stop = target.with(|agent| -> Result<bool, LoopError> {
            let Phase::Idle { last_turn } = agent.phase else {
                return Err(agent.active_work_error());
            };
            if agent.inbox.next_turn().is_empty() && !agent.wake_latched {
                return Ok(true);
            }
            agent.wake_latched = false;
            agent.phase = Phase::Running {
                abort: AbortFlag::new(),
                turn: last_turn,
                step: 0,
                wake_requested: false,
            };
            Ok(false)
        })?;
        if should_stop {
            return Ok(());
        }
        let running = drive_running(target).await;
        let should_continue = target.with(|agent| {
            let (last_turn, wake_requested) = agent.settle_driver();
            agent.phase = Phase::Idle { last_turn };
            wake_requested && agent.inbox.has_pending()
        });
        running?;
        if should_continue {
            target.with(|agent| agent.wake_latched = true);
            continue;
        }
        return Ok(());
    }
}

async fn drive_running(target: &mut DriveTarget<'_>) -> Result<(), LoopError> {
    while turn(target).await? {}
    Ok(())
}

async fn turn(target: &mut DriveTarget<'_>) -> Result<bool, LoopError> {
    let started = target.with(LoopAgent::begin_turn);
    if let Err(error) = &started {
        if target.with(|agent| agent.is_aborted()) {
            return Err(LoopError::Invalid(error.to_string()));
        }
    }
    let turn = started?;
    let mut turn_ends: Option<TurnEndReason> = None;
    let body = run_turn_steps(target, turn, &mut turn_ends).await;
    if let Err(error) = &body {
        if turn_ends.is_none() {
            turn_ends = Some(if target.with(|agent| agent.is_aborted()) {
                TurnEndReason::Aborted {
                    reason: target.with(|agent| agent.cancel_reason_json()),
                }
            } else {
                TurnEndReason::Error {
                    error: llm_failure_from_loop_error(error),
                }
            });
        }
    }
    let reason = turn_ends.unwrap_or(TurnEndReason::Completed);
    let aborted_turn =
        target.with(|agent| agent.is_aborted()) || matches!(reason, TurnEndReason::Aborted { .. });
    let end_result = target.with(|agent| {
        agent.push_event(|seq| SessionEvent::TurnEnd {
            seq,
            time: seq as i64,
            data: TurnEndData { turn, reason },
            ignorable: None,
        })
    });
    body?;
    end_result?;
    if aborted_turn {
        return Ok(false);
    }
    if !target.with(|agent| agent.inbox.has_pending()) {
        return Ok(false);
    }
    target.with(|agent| agent.reset_running_for_next_turn());
    Ok(true)
}

async fn run_turn_steps(
    target: &mut DriveTarget<'_>,
    turn: u64,
    turn_ends: &mut Option<TurnEndReason>,
) -> Result<(), LoopError> {
    let mut inbox_target = InboxTarget::NextTurn;
    loop {
        if target.with(|agent| agent.is_aborted()) {
            *turn_ends = Some(TurnEndReason::Aborted {
                reason: target.with(|agent| agent.cancel_reason_json()),
            });
            return Err(LoopError::Invalid(
                target.with(|agent| format!("agent \"{}\" aborted", agent.id.as_str())),
            ));
        }
        let (ctx, seed, assembly) = target.with(|agent| agent.claim_pre_step(inbox_target))?;
        let decision = {
            let compaction = target.compaction_scope();
            compaction
                .run(ctx.waterfall(EVENT_AGENT_PRE_STEP, seed))
                .await
        };
        match decision {
            PreStepDecision::Reject => {
                *turn_ends = Some(TurnEndReason::Blocked);
                return Ok(());
            }
            PreStepDecision::Enter { messages } => {
                let step_zero =
                    target.with(|agent| matches!(agent.phase, Phase::Running { step: 0, .. }));
                if step_zero && messages.is_empty() {
                    *turn_ends = Some(TurnEndReason::Completed);
                    return Ok(());
                }
                if turn_ends.is_some() && messages.is_empty() {
                    break;
                }
                let step = target.with(|agent| agent.open_step(turn, messages))?;
                let step_result = execute_step(target, &assembly, turn, step).await;
                let end_result = target.with(|agent| {
                    agent.push_event(|seq| SessionEvent::StepEnd {
                        seq,
                        time: seq as i64,
                        data: StepBoundaryData { turn, step },
                        ignorable: None,
                    })
                });
                let step_end = step_result?;
                end_result?;
                if turn_ends
                    .as_ref()
                    .map(|reason| !matches!(reason, TurnEndReason::MaxTokens))
                    .unwrap_or(true)
                {
                    if let Some(ref end) = step_end {
                        *turn_ends = Some(end.clone());
                    }
                }
                if target.with(|agent| agent.is_aborted()) {
                    *turn_ends = Some(TurnEndReason::Aborted {
                        reason: target.with(|agent| agent.cancel_reason_json()),
                    });
                    return Ok(());
                }
                if turn_ends.is_some()
                    && target.with(|agent| agent.inbox.next_step().is_empty())
                    && !matches!(step_end, Some(TurnEndReason::MaxTokens))
                {
                    break;
                }
                inbox_target = InboxTarget::NextStep;
            }
        }
    }
    Ok(())
}

async fn execute_step(
    target: &mut DriveTarget<'_>,
    assembly: &PromptAssembly,
    turn: u64,
    step: u64,
) -> Result<Option<TurnEndReason>, LoopError> {
    loop {
        if target.with(|agent| agent.is_aborted()) {
            return Ok(Some(TurnEndReason::Aborted {
                reason: target.with(|agent| agent.cancel_reason_json()),
            }));
        }
        let prepared = target.with(|agent| -> Result<_, LoopError> {
            let system =
                render_prompt(assembly).map_err(|error| LoopError::Prompt(error.to_string()))?;
            let boundary_messages = agent.session.derive_messages();
            let request =
                agent.build_request(turn, step, &assembly.tools, &system, boundary_messages)?;
            let llm = agent.llm.lock().expect("llm").clone();
            Ok((request, llm, agent.ctx.clone()))
        })?;
        let (request, llm, ctx) = prepared;
        let provider = request.provider.clone();
        let model = request.model.clone();
        let chunks = collect_stream_from(llm, request).await;
        let ingest = target.with(|agent| agent.ingest_chunks(turn, step, chunks))?;
        match ingest {
            StreamIngest::Failed(failure) => {
                let (policy, prior, abort) = target.with(|agent| {
                    let policy = agent
                        .llm
                        .lock()
                        .expect("llm")
                        .provider_retry_policy(&provider);
                    (
                        policy,
                        llm_retry_payloads(&agent.session),
                        running_abort(agent),
                    )
                });
                let scope = RetryScope::new(
                    failure.clone(),
                    turn,
                    step,
                    provider.clone(),
                    policy,
                    prior,
                    abort,
                );
                let (action, audit) = {
                    let compaction = target.compaction_scope();
                    compaction
                        .run(scope.run(
                            ctx.waterfall(EVENT_AGENT_REQUEST_ERROR, RequestErrorAction::Fail),
                        ))
                        .await
                };
                target.with(|agent| -> Result<(), LoopError> {
                    for entry in audit {
                        agent.push_event(|seq| entry.into_session_event(seq))?;
                    }
                    Ok(())
                })?;
                if target.with(|agent| agent.is_aborted()) {
                    return Ok(Some(TurnEndReason::Aborted {
                        reason: target.with(|agent| agent.cancel_reason_json()),
                    }));
                }
                match action {
                    RequestErrorAction::Retry => continue,
                    RequestErrorAction::Fail => {
                        return Ok(Some(TurnEndReason::Error { error: failure }));
                    }
                }
            }
            StreamIngest::Ready {
                assembler,
                chunk_seqs,
                finish,
            } => {
                target.with(|agent| {
                    agent.append_assistant(turn, step, &assembler, &chunk_seqs, &provider, &model)
                })?;
                if matches!(finish, FinishReason::MaxTokens) {
                    return Ok(Some(TurnEndReason::MaxTokens));
                }
                let tool_calls: Vec<ContentBlock> = assembler
                    .blocks()
                    .into_iter()
                    .filter(|block| matches!(block, ContentBlock::ToolCall { .. }))
                    .collect();
                if tool_calls.is_empty() {
                    return Ok(Some(TurnEndReason::Completed));
                }
                let (signal, max_parallel) = target.with(|agent| {
                    let signal = match &agent.phase {
                        Phase::Running { abort, .. } | Phase::Maintenance { abort, .. } => {
                            abort.clone()
                        }
                        Phase::Idle { .. } => AbortFlag::new(),
                    };
                    (signal, agent.options.max_parallel_tool_calls)
                });
                let mut extra = Vec::new();
                let outcome = target
                    .execute_tools(turn, step, &tool_calls, &signal, max_parallel, &mut extra)
                    .await?;
                target.with(|agent| -> Result<(), LoopError> {
                    for message in extra {
                        let start = agent.inbox.next_step().len();
                        agent.inbox.splice(
                            &mut agent.session,
                            InboxTarget::NextStep,
                            start,
                            0,
                            vec![message],
                            true,
                        )?;
                    }
                    Ok(())
                })?;
                if outcome.aborted || target.with(|agent| agent.is_aborted()) {
                    return Ok(Some(TurnEndReason::Aborted {
                        reason: target.with(|agent| agent.cancel_reason_json()),
                    }));
                }
                return Ok(if outcome.concluded {
                    Some(TurnEndReason::Completed)
                } else {
                    None
                });
            }
        }
    }
}

async fn collect_stream_from(llm: LlmRuntime, request: GenerateOptions) -> Vec<StreamChunk> {
    let mut stream = llm.stream(request);
    let mut chunks = Vec::new();
    while let Some(chunk) = stream.next().await {
        chunks.push(chunk);
    }
    chunks
}

fn llm_failure_from_loop_error(error: &LoopError) -> LlmFailure {
    LlmFailure {
        message: error.to_string(),
        code: "UNKNOWN".into(),
        status: None,
        provider_retry_after_ms: None,
        request_id: None,
    }
}

fn last_turn_from(session: &Session) -> u64 {
    session
        .events()
        .iter()
        .rev()
        .find_map(|event| match event {
            LogEvent::Known(SessionEvent::TurnStart { data, .. }) => Some(data.turn),
            _ => None,
        })
        .unwrap_or(0)
}

fn llm_retry_payloads(session: &Session) -> Vec<serde_json::Value> {
    session
        .events()
        .iter()
        .filter_map(|event| match event {
            LogEvent::Known(SessionEvent::LlmRetry { data, .. }) => Some(data.clone()),
            _ => None,
        })
        .collect()
}

fn running_abort(agent: &LoopAgent) -> AbortFlag {
    match &agent.phase {
        Phase::Running { abort, .. } | Phase::Maintenance { abort, .. } => abort.clone(),
        Phase::Idle { .. } => AbortFlag::new(),
    }
}

#[cfg(test)]
pub(crate) use crate::{event_types, test_header, user_text};

#[cfg(test)]
mod tests {
    use super::{
        AgentStatus, CancelCause, CancelOptions, EVENT_AGENT_PRE_STEP, EVENT_AGENT_REQUEST_ERROR,
        LoopAgent, LoopOptions, Phase, PreStepDecision, RequestErrorAction, event_types,
        test_header, user_text,
    };
    use dsh_llm::{LlmRuntime, MockAdapter, MockScript, max_tokens_response, text_response};
    use dsh_session::{LlmCallConfig, MessageSource, Session, TurnEndReason};
    use dsh_system_prompt::{SystemPrompt, SystemPromptConfig};
    use dsh_tools::ToolRuntime;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn harness(script: Vec<MockScript>) -> (LoopAgent, Arc<MockAdapter>) {
        let adapter = Arc::new(MockAdapter::new(script));
        let mut llm = LlmRuntime::new();
        llm.register_adapter("mock", adapter.clone());
        let agent = LoopAgent::new(
            dsh_kernel::Context::new(),
            Session::new(test_header("loop-1")),
            LoopOptions {
                provider: "mock".into(),
                model: "mock".into(),
                max_tokens: None,
                max_parallel_tool_calls: 10,
            },
            Arc::new(std::sync::Mutex::new(ToolRuntime::new(
                dsh_tools::ToolPresentationMode::Native,
            ))),
            SystemPrompt::new(SystemPromptConfig::default()).unwrap(),
            Arc::new(std::sync::Mutex::new(llm)),
        )
        .unwrap();
        (agent, adapter)
    }

    #[tokio::test]
    async fn starts_idle_and_maintenance_reports_idle() {
        let (mut agent, _) = harness(vec![]);
        assert!(matches!(agent.phase(), Phase::Idle { last_turn: 0 }));
        assert_eq!(agent.status(), AgentStatus::Idle);
        let status = agent
            .run_maintenance(|abort| async move {
                assert!(!abort.is_aborted());
                AgentStatus::Idle
            })
            .await
            .unwrap();
        assert_eq!(status, AgentStatus::Idle);
        assert!(matches!(agent.phase(), Phase::Idle { last_turn: 0 }));
    }

    #[tokio::test]
    async fn turn_start_is_appended_before_claim() {
        let (mut agent, adapter) = harness(vec![MockScript::Chunks(text_response("ok"))]);
        let saw_turn_before_claim = Arc::new(AtomicBool::new(false));
        let saw = saw_turn_before_claim.clone();
        agent.on_pre_step(move |decision, next| {
            let saw = saw.clone();
            async move {
                saw.store(true, Ordering::SeqCst);
                next(decision).await
            }
        });
        agent.followup(user_text("m1", "hi")).unwrap();
        agent.run_until_idle().await.unwrap();
        assert!(saw_turn_before_claim.load(Ordering::SeqCst));
        let types = event_types(&agent.session);
        let turn = types.iter().position(|t| t.as_str() == "turn/start");
        let step = types.iter().position(|t| t.as_str() == "step/start");
        assert!(turn.unwrap() < step.unwrap(), "{types:?}");
        assert_eq!(adapter.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn empty_first_claim_logs_a_turn_without_a_step() {
        let (mut agent, adapter) = harness(vec![MockScript::Chunks(text_response("must not run"))]);
        agent.on_pre_step(|_decision, _next| async { PreStepDecision::Enter { messages: vec![] } });
        agent.followup(user_text("m1", "go")).unwrap();
        agent.run_until_idle().await.unwrap();
        let types: Vec<_> = event_types(&agent.session)
            .into_iter()
            .filter(|t| matches!(t.as_str(), "turn/start" | "step/start" | "turn/end"))
            .collect();
        assert_eq!(types, vec!["turn/start", "turn/end"]);
        assert!(adapter.requests.lock().unwrap().is_empty());
        let end = agent
            .session
            .events()
            .iter()
            .rev()
            .find_map(|e| match e {
                dsh_session::LogEvent::Known(dsh_session::SessionEvent::TurnEnd {
                    data, ..
                }) => Some(data),
                _ => None,
            })
            .unwrap();
        assert_eq!(end.turn, 1);
        assert!(matches!(end.reason, TurnEndReason::Completed));
        assert!(agent.inbox.next_turn().is_empty());
    }

    #[tokio::test]
    async fn simple_text_turn_event_order() {
        let (mut agent, _) = harness(vec![MockScript::Chunks(text_response("ok"))]);
        agent.followup(user_text("m1", "hi")).unwrap();
        agent.run_until_idle().await.unwrap();
        let types = event_types(&agent.session);
        let interesting: Vec<_> = types
            .iter()
            .filter(|t| {
                matches!(
                    t.as_str(),
                    "agent/inbox/spliced"
                        | "turn/start"
                        | "step/start"
                        | "user/message"
                        | "request/header"
                        | "assistant/message"
                        | "step/end"
                        | "turn/end"
                )
            })
            .cloned()
            .collect();
        assert_eq!(
            interesting,
            vec![
                "agent/inbox/spliced",
                "turn/start",
                "agent/inbox/spliced",
                "step/start",
                "user/message",
                "request/header",
                "assistant/message",
                "step/end",
                "turn/end",
            ]
        );
        assert!(types.iter().any(|t| t == "assistant/chunk"));
        assert_eq!(agent.status(), AgentStatus::Idle);
    }

    #[tokio::test]
    async fn snapshot_identity_skips_duplicate_runtime_context() {
        let mut prompt = SystemPrompt::new(SystemPromptConfig::default()).unwrap();
        prompt
            .context(dsh_system_prompt::PromptContext {
                name: "cwd".into(),
                order: 0,
                text: dsh_system_prompt::SectionText::Static("cwd=/tmp".into()),
            })
            .unwrap();
        let adapter = Arc::new(MockAdapter::new(vec![
            MockScript::Chunks(text_response("one")),
            MockScript::Chunks(text_response("two")),
        ]));
        let mut llm = LlmRuntime::new();
        llm.register_adapter("mock", adapter.clone());
        let mut agent = LoopAgent::new(
            dsh_kernel::Context::new(),
            Session::new(test_header("snap")),
            LoopOptions {
                provider: "mock".into(),
                model: "mock".into(),
                max_tokens: None,
                max_parallel_tool_calls: 10,
            },
            Arc::new(std::sync::Mutex::new(ToolRuntime::new(
                dsh_tools::ToolPresentationMode::Native,
            ))),
            prompt,
            Arc::new(std::sync::Mutex::new(llm)),
        )
        .unwrap();
        agent.followup(user_text("m1", "first")).unwrap();
        agent.run_until_idle().await.unwrap();
        agent.followup(user_text("m2", "second")).unwrap();
        agent.run_until_idle().await.unwrap();
        let snapshots = agent
            .session
            .events()
            .iter()
            .filter(|e| match e {
                dsh_session::LogEvent::Known(dsh_session::SessionEvent::UserMessage {
                    data,
                    ..
                }) => matches!(
                    &data.source,
                    dsh_session::MessageSource::Plugin { plugin, .. }
                        if plugin == super::RUNTIME_CONTEXT_SOURCE
                ),
                _ => false,
            })
            .count();
        assert_eq!(snapshots, 1);
    }

    #[tokio::test]
    async fn assistant_source_uses_on_request_route() {
        let (mut agent, adapter) = harness(vec![MockScript::Chunks(text_response("ok"))]);
        agent.on_request(|config| LlmCallConfig {
            model: "rerouted".into(),
            ..config.clone()
        });
        agent.followup(user_text("m1", "hi")).unwrap();
        agent.run_until_idle().await.unwrap();
        assert_eq!(adapter.requests.lock().unwrap()[0].model, "rerouted");
        let source = agent
            .session
            .events()
            .iter()
            .rev()
            .find_map(|e| match e {
                dsh_session::LogEvent::Known(dsh_session::SessionEvent::AssistantMessage {
                    data,
                    ..
                }) => Some(&data.message.source),
                _ => None,
            })
            .unwrap();
        assert!(matches!(
            source,
            MessageSource::Model { provider, model, .. }
                if provider == "mock" && model == "rerouted"
        ));
    }

    #[tokio::test]
    async fn seeds_max_tokens_on_the_first_request() {
        let (mut agent, adapter) = harness(vec![MockScript::Chunks(text_response("bounded"))]);
        agent.options.max_tokens = Some(256);
        agent
            .followup(user_text("m1", "use the configured output limit"))
            .unwrap();
        agent.run_until_idle().await.unwrap();
        assert_eq!(adapter.requests.lock().unwrap()[0].max_tokens, Some(256));
    }

    #[tokio::test]
    async fn sticky_max_tokens_survives_a_later_completed_step() {
        let (mut agent, _) = harness(vec![
            MockScript::Chunks(max_tokens_response("cut")),
            MockScript::Chunks(text_response("continued")),
        ]);
        let n = Arc::new(AtomicUsize::new(0));
        agent.on_pre_step({
            let n = n.clone();
            move |decision, _next| {
                let n = n.clone();
                async move {
                    let step = n.fetch_add(1, Ordering::SeqCst) + 1;
                    if step == 1 {
                        decision
                    } else {
                        PreStepDecision::Enter {
                            messages: vec![user_text("cont", "continue after truncation")],
                        }
                    }
                }
            }
        });
        agent.followup(user_text("m1", "go")).unwrap();
        agent.run_until_idle().await.unwrap();
        assert_eq!(n.load(Ordering::SeqCst), 2);
        let reasons: Vec<_> = agent
            .session
            .events()
            .iter()
            .filter_map(|e| match e {
                dsh_session::LogEvent::Known(dsh_session::SessionEvent::TurnEnd {
                    data, ..
                }) => Some(&data.reason),
                _ => None,
            })
            .collect();
        assert!(matches!(reasons.as_slice(), [TurnEndReason::MaxTokens]));
    }

    #[tokio::test]
    async fn max_tokens_does_not_leak_across_turns() {
        let (mut agent, _) = harness(vec![
            MockScript::Chunks(max_tokens_response("cut")),
            MockScript::Chunks(text_response("fresh")),
        ]);
        agent.followup(user_text("m1", "one")).unwrap();
        agent.run_until_idle().await.unwrap();
        agent.followup(user_text("m2", "two")).unwrap();
        agent.run_until_idle().await.unwrap();
        let reasons: Vec<_> = agent
            .session
            .events()
            .iter()
            .filter_map(|e| match e {
                dsh_session::LogEvent::Known(dsh_session::SessionEvent::TurnEnd {
                    data, ..
                }) => Some(&data.reason),
                _ => None,
            })
            .collect();
        assert!(matches!(
            reasons.as_slice(),
            [TurnEndReason::MaxTokens, TurnEndReason::Completed]
        ));
    }

    #[tokio::test]
    async fn idle_cancel_is_noop_and_next_prompt_runs() {
        let (mut agent, adapter) = harness(vec![MockScript::Chunks(text_response("reply"))]);
        agent
            .cancel(CancelCause::User, CancelOptions::default())
            .unwrap();
        agent.followup(user_text("m1", "real prompt")).unwrap();
        agent.run_until_idle().await.unwrap();
        assert_eq!(adapter.requests.lock().unwrap().len(), 1);
        assert!(event_types(&agent.session).iter().any(|t| t == "turn/end"));
    }

    #[tokio::test]
    async fn abort_drain_synthesizes_aborted_before_dispatch() {
        use dsh_session::CallId;
        use dsh_tools::{ToolDefinition, ToolPresentationMode};
        use serde_json::json;
        let started = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(tokio::sync::Notify::new());
        let started_c = started.clone();
        let release_c = release.clone();
        let chunks = {
            use dsh_session::StreamChunk;
            vec![
                StreamChunk::BlockStart {
                    index: 0,
                    block_type: "tool-call".into(),
                },
                StreamChunk::BlockEnd {
                    index: 0,
                    block: dsh_session::ContentBlock::ToolCall {
                        id: CallId::new("c1"),
                        name: "slow".into(),
                        arguments: json!({"id":"a"}).to_string(),
                    },
                },
                StreamChunk::BlockStart {
                    index: 1,
                    block_type: "tool-call".into(),
                },
                StreamChunk::BlockEnd {
                    index: 1,
                    block: dsh_session::ContentBlock::ToolCall {
                        id: CallId::new("c2"),
                        name: "slow".into(),
                        arguments: json!({"id":"b"}).to_string(),
                    },
                },
                StreamChunk::Usage {
                    usage: dsh_session::TokenUsage {
                        input_tokens: 5,
                        output_tokens: 5,
                        cache_read_tokens: None,
                        cache_write_tokens: None,
                        reasoning_tokens: None,
                    },
                },
                StreamChunk::Finish {
                    reason: dsh_session::FinishReason::ToolCalls,
                    replay_state: None,
                },
            ]
        };
        let adapter = Arc::new(MockAdapter::new(vec![MockScript::Chunks(chunks)]));
        let mut llm = LlmRuntime::new();
        llm.register_adapter("mock", adapter);
        let mut tools = ToolRuntime::new(ToolPresentationMode::Native);
        tools.register(ToolDefinition {
            name: "slow".into(),
            description: "gated".into(),
            parameters: json!({"type": "object"}),
            execute: Box::new(move |args, exec| {
                let started_c = started_c.clone();
                let release_c = release_c.clone();
                Box::pin(async move {
                    started_c.fetch_add(1, Ordering::SeqCst);
                    exec.signal.abort();
                    release_c.notified().await;
                    Ok(args)
                })
            }),
            render: Box::new(|_args, value| {
                vec![dsh_session::ContentBlock::Text {
                    text: value.to_string(),
                }]
            }),
            is_concurrency_safe: None,
        });
        let mut agent = LoopAgent::new(
            dsh_kernel::Context::new(),
            Session::new(test_header("abort-drain")),
            LoopOptions {
                provider: "mock".into(),
                model: "mock".into(),
                max_tokens: None,
                max_parallel_tool_calls: 1,
            },
            Arc::new(std::sync::Mutex::new(tools)),
            SystemPrompt::new(SystemPromptConfig::default()).unwrap(),
            Arc::new(std::sync::Mutex::new(llm)),
        )
        .unwrap();
        agent.followup(user_text("m1", "go")).unwrap();
        {
            let run = agent.run_until_idle();
            tokio::pin!(run);
            loop {
                tokio::select! {
                    result = &mut run => {
                        result.unwrap();
                        break;
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_millis(5)) => {
                        if started.load(Ordering::SeqCst) >= 1 {
                            release.notify_waiters();
                        }
                    }
                }
            }
        }
        let results: Vec<_> = agent
            .session
            .events()
            .iter()
            .filter_map(|e| match e {
                dsh_session::LogEvent::Known(dsh_session::SessionEvent::ToolResult {
                    data,
                    ..
                }) => data.error.clone(),
                _ => None,
            })
            .collect();
        assert!(
            results.iter().any(
                |e| e.code == dsh_tools::TOOL_ABORTED_BEFORE_DISPATCH && e.name == "AbortError"
            )
        );
    }

    #[tokio::test]
    async fn parallel_siblings_start_together_exclusive_is_a_barrier() {
        use dsh_session::CallId;
        use dsh_tools::{ToolDefinition, ToolPresentationMode};
        use serde_json::json;
        let live = Arc::new(AtomicUsize::new(0));
        let max_live = Arc::new(AtomicUsize::new(0));
        let live_c = live.clone();
        let max_c = max_live.clone();
        let chunks = vec![
            dsh_session::StreamChunk::BlockStart {
                index: 0,
                block_type: "tool-call".into(),
            },
            dsh_session::StreamChunk::BlockEnd {
                index: 0,
                block: dsh_session::ContentBlock::ToolCall {
                    id: CallId::new("p1"),
                    name: "par".into(),
                    arguments: json!({"id":"1"}).to_string(),
                },
            },
            dsh_session::StreamChunk::BlockStart {
                index: 1,
                block_type: "tool-call".into(),
            },
            dsh_session::StreamChunk::BlockEnd {
                index: 1,
                block: dsh_session::ContentBlock::ToolCall {
                    id: CallId::new("p2"),
                    name: "par".into(),
                    arguments: json!({"id":"2"}).to_string(),
                },
            },
            dsh_session::StreamChunk::Usage {
                usage: dsh_session::TokenUsage {
                    input_tokens: 1,
                    output_tokens: 1,
                    cache_read_tokens: None,
                    cache_write_tokens: None,
                    reasoning_tokens: None,
                },
            },
            dsh_session::StreamChunk::Finish {
                reason: dsh_session::FinishReason::ToolCalls,
                replay_state: None,
            },
        ];
        let adapter = Arc::new(MockAdapter::new(vec![
            MockScript::Chunks(chunks),
            MockScript::Chunks(text_response("done")),
        ]));
        let mut llm = LlmRuntime::new();
        llm.register_adapter("mock", adapter);
        let mut tools = ToolRuntime::new(ToolPresentationMode::Native);
        tools.register(ToolDefinition {
            name: "par".into(),
            description: "parallel".into(),
            parameters: json!({"type": "object"}),
            execute: Box::new(move |args, _exec| {
                let live_c = live_c.clone();
                let max_c = max_c.clone();
                Box::pin(async move {
                    let n = live_c.fetch_add(1, Ordering::SeqCst) + 1;
                    max_c.fetch_max(n, Ordering::SeqCst);
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    live_c.fetch_sub(1, Ordering::SeqCst);
                    Ok(args)
                })
            }),
            render: Box::new(|_, v| {
                vec![dsh_session::ContentBlock::Text {
                    text: v.to_string(),
                }]
            }),
            is_concurrency_safe: Some(Box::new(|_| true)),
        });
        let mut agent = LoopAgent::new(
            dsh_kernel::Context::new(),
            Session::new(test_header("par")),
            LoopOptions {
                provider: "mock".into(),
                model: "mock".into(),
                max_tokens: None,
                max_parallel_tool_calls: 10,
            },
            Arc::new(std::sync::Mutex::new(tools)),
            SystemPrompt::new(SystemPromptConfig::default()).unwrap(),
            Arc::new(std::sync::Mutex::new(llm)),
        )
        .unwrap();
        agent.followup(user_text("m1", "go")).unwrap();
        agent.run_until_idle().await.unwrap();
        assert!(
            max_live.load(Ordering::SeqCst) >= 2,
            "parallel siblings must overlap"
        );
    }

    fn loop_agent_with_ctx(ctx: dsh_kernel::Context, _text: &str) -> LoopAgent {
        let adapter = Arc::new(MockAdapter::new(vec![MockScript::Chunks(text_response(
            "ok",
        ))]));
        let mut llm = LlmRuntime::new();
        llm.register_adapter("mock", adapter);
        LoopAgent::new(
            ctx,
            Session::new(test_header("loop-ctx")),
            LoopOptions {
                provider: "mock".into(),
                model: "mock".into(),
                max_tokens: None,
                max_parallel_tool_calls: 10,
            },
            Arc::new(std::sync::Mutex::new(ToolRuntime::new(
                dsh_tools::ToolPresentationMode::Native,
            ))),
            SystemPrompt::new(SystemPromptConfig::default()).unwrap(),
            Arc::new(std::sync::Mutex::new(llm)),
        )
        .unwrap()
    }

    fn last_turn_end(agent: &LoopAgent) -> Option<&dsh_session::TurnEndReason> {
        agent
            .session
            .events()
            .iter()
            .rev()
            .find_map(|event| match event {
                dsh_session::LogEvent::Known(dsh_session::SessionEvent::TurnEnd {
                    data, ..
                }) => Some(&data.reason),
                _ => None,
            })
    }

    fn last_assistant_text(agent: &LoopAgent) -> String {
        agent
            .session
            .events()
            .iter()
            .rev()
            .find_map(|event| match event {
                dsh_session::LogEvent::Known(dsh_session::SessionEvent::AssistantMessage {
                    data,
                    ..
                }) => data.message.content.iter().find_map(|block| match block {
                    dsh_session::ContentBlock::Text { text } => Some(text.clone()),
                    _ => None,
                }),
                _ => None,
            })
            .unwrap_or_default()
    }

    fn loop_agent_fail_then_text(
        ctx: dsh_kernel::Context,
        code: &str,
        recovered: &str,
    ) -> LoopAgent {
        let adapter = Arc::new(MockAdapter::new(vec![
            MockScript::Fail(dsh_llm::LlmError::new("busy", code)),
            MockScript::Chunks(text_response(recovered)),
        ]));
        let mut llm = LlmRuntime::new();
        llm.register_adapter("mock", adapter);
        LoopAgent::new(
            ctx,
            Session::new(test_header("loop-retry")),
            LoopOptions {
                provider: "mock".into(),
                model: "mock".into(),
                max_tokens: None,
                max_parallel_tool_calls: 10,
            },
            Arc::new(std::sync::Mutex::new(ToolRuntime::new(
                dsh_tools::ToolPresentationMode::Native,
            ))),
            SystemPrompt::new(SystemPromptConfig::default()).unwrap(),
            Arc::new(std::sync::Mutex::new(llm)),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn pre_step_waterfall_must_call_next_to_keep_claimed_messages() {
        let ctx = dsh_kernel::Context::new();
        ctx.on_waterfall::<PreStepDecision, _, _>(
            EVENT_AGENT_PRE_STEP,
            |decision, next| async move { next(decision).await },
        )
        .unwrap();
        let mut agent = loop_agent_with_ctx(ctx, "keep me");
        agent.followup(user_text("keep-me", "keep me")).unwrap();
        agent.run_until_idle().await.unwrap();
        assert!(agent.session.events().iter().any(|e| matches!(
            e,
            dsh_session::LogEvent::Known(dsh_session::SessionEvent::UserMessage { data, .. })
                if data.content.iter().any(|block| matches!(
                    block,
                    dsh_session::ContentBlock::Text { text } if text == "keep me"
                ))
        )));
    }

    #[tokio::test]
    async fn pre_step_without_next_rejects_the_step() {
        let ctx = dsh_kernel::Context::new();
        ctx.on_waterfall::<PreStepDecision, _, _>(
            EVENT_AGENT_PRE_STEP,
            |_decision, _next| async move { PreStepDecision::Reject },
        )
        .unwrap();
        let mut agent = loop_agent_with_ctx(ctx, "blocked");
        agent.followup(user_text("blocked", "blocked")).unwrap();
        agent.run_until_idle().await.unwrap();
        assert!(matches!(
            last_turn_end(&agent),
            Some(dsh_session::TurnEndReason::Blocked)
        ));
    }

    #[tokio::test]
    async fn request_error_retry_without_next_owns_recovery() {
        let ctx = dsh_kernel::Context::new();
        ctx.on_waterfall::<RequestErrorAction, _, _>(
            EVENT_AGENT_REQUEST_ERROR,
            |_action, _next| async { RequestErrorAction::Retry },
        )
        .unwrap();
        let mut agent = loop_agent_fail_then_text(ctx, "RATE_LIMIT", "recovered");
        agent.followup(user_text("hi", "hi")).unwrap();
        agent.run_until_idle().await.unwrap();
        assert_eq!(last_assistant_text(&agent), "recovered");
    }

    fn find_event(agent: &LoopAgent, event_type: &str) -> serde_json::Value {
        agent
            .session
            .events()
            .iter()
            .find_map(|event| match event {
                dsh_session::LogEvent::Known(dsh_session::SessionEvent::LlmRetry {
                    data, ..
                }) if event_type == "llm/retry" => Some(data.clone()),
                dsh_session::LogEvent::Known(dsh_session::SessionEvent::LlmRetryStarted {
                    data,
                    ..
                }) if event_type == "llm/retry-started" => Some(data.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing {event_type} event"))
    }

    #[tokio::test]
    async fn retry_plugin_retries_rate_limit_once_and_logs_llm_retry() {
        let ctx = dsh_kernel::Context::new();
        let mut llm = LlmRuntime::new();
        llm.register_adapter(
            "deepseek-official",
            Arc::new(dsh_llm::FailThenOk::rate_limit()),
        );
        ctx.provide("llm", std::sync::Mutex::new(llm)).unwrap();
        dsh_llm::retry::install(&ctx);
        let llm = ctx.get::<std::sync::Mutex<LlmRuntime>>("llm").expect("llm");
        let mut agent = LoopAgent::new(
            ctx,
            Session::new(test_header("retry-plugin")),
            LoopOptions {
                provider: "deepseek-official".into(),
                model: "mock".into(),
                max_tokens: None,
                max_parallel_tool_calls: 10,
            },
            Arc::new(std::sync::Mutex::new(ToolRuntime::new(
                dsh_tools::ToolPresentationMode::Native,
            ))),
            SystemPrompt::new(SystemPromptConfig::default()).unwrap(),
            llm,
        )
        .unwrap();
        agent
            .followup(user_text("retry", "retry the transient provider failure"))
            .unwrap();
        agent.run_until_idle().await.unwrap();
        assert_eq!(last_assistant_text(&agent), "RETRY_OK");
        let retry = find_event(&agent, "llm/retry");
        assert_eq!(retry["failure"]["code"], "RATE_LIMIT");
        assert_eq!(retry["maxRetries"], 1);
        assert_eq!(retry["failure"]["status"], 429);
        assert_eq!(retry["policyKey"], r#"["normal",1,["RATE_LIMIT"],1,1,0]"#);
        assert_eq!(retry["delayMs"], 1);
    }

    mod reconstruction {
        use super::{LoopAgent, LoopOptions, harness, test_header, user_text};
        use dsh_llm::{
            GenerateOptions, LlmRuntime, MockAdapter, MockScript, text_response, tool_call_response,
        };
        use dsh_session::{RequestHeaderReason, Session};
        use dsh_system_prompt::{SystemPrompt, SystemPromptConfig};
        use dsh_tools::ToolRuntime;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        fn expect_prefix_extension(previous: &GenerateOptions, current: &GenerateOptions) {
            assert!(current.messages.len() > previous.messages.len());
            assert_eq!(
                &current.messages[..previous.messages.len()],
                previous.messages.as_slice()
            );
            assert_eq!(current.system, previous.system);
            assert_eq!(current.tools, previous.tools);
        }

        #[tokio::test]
        async fn each_step_request_append_extends_the_previous() {
            use dsh_tools::{ToolDefinition, ToolPresentationMode};
            use serde_json::json;
            let adapter = Arc::new(MockAdapter::new(vec![
                MockScript::Chunks(tool_call_response(
                    "c1",
                    "echo",
                    &json!({"text":"one"}),
                    Some("first"),
                )),
                MockScript::Chunks(tool_call_response(
                    "c2",
                    "echo",
                    &json!({"text":"two"}),
                    Some("second"),
                )),
                MockScript::Chunks(text_response("done")),
            ]));
            let mut llm = LlmRuntime::new();
            llm.register_adapter("mock", adapter.clone());
            let mut tools = ToolRuntime::new(ToolPresentationMode::Native);
            tools.register(ToolDefinition {
                name: "echo".into(),
                description: "echo back".into(),
                parameters: json!({"type": "object"}),
                execute: Box::new(|args, _exec| {
                    Box::pin(async move { Ok(args.get("text").cloned().unwrap_or(json!(""))) })
                }),
                render: Box::new(|_args, value| {
                    vec![dsh_session::ContentBlock::Text {
                        text: format!("echo: {}", value.as_str().unwrap_or("")),
                    }]
                }),
                is_concurrency_safe: None,
            });
            let mut agent = LoopAgent::new(
                dsh_kernel::Context::new(),
                Session::new(test_header("a1")),
                LoopOptions {
                    provider: "mock".into(),
                    model: "mock".into(),
                    max_tokens: None,
                    max_parallel_tool_calls: 10,
                },
                Arc::new(std::sync::Mutex::new(tools)),
                SystemPrompt::new(SystemPromptConfig {
                    persona: "stable base".into(),
                    ..SystemPromptConfig::default()
                })
                .unwrap(),
                Arc::new(std::sync::Mutex::new(llm)),
            )
            .unwrap();
            agent.followup(user_text("m1", "go")).unwrap();
            agent.run_until_idle().await.unwrap();
            let requests = adapter.requests.lock().unwrap().clone();
            assert_eq!(requests.len(), 3);
            expect_prefix_extension(&requests[0], &requests[1]);
            expect_prefix_extension(&requests[1], &requests[2]);
            let headers: Vec<_> = agent
                .session
                .events()
                .iter()
                .filter_map(|e| match e {
                    dsh_session::LogEvent::Known(dsh_session::SessionEvent::RequestHeader {
                        data,
                        ..
                    }) => Some(data.reason.clone()),
                    _ => None,
                })
                .collect();
            assert_eq!(headers, vec![RequestHeaderReason::Initial]);
            assert!(requests[1].messages.iter().any(|m| {
                m.content
                    .iter()
                    .any(|b| matches!(b, dsh_session::ContentBlock::ToolResult { .. }))
            }));
        }

        #[tokio::test]
        async fn later_turn_append_extends_the_previous_turn() {
            let (mut agent, adapter) = harness(vec![
                MockScript::Chunks(text_response("one")),
                MockScript::Chunks(text_response("two")),
            ]);
            agent.followup(user_text("m1", "first")).unwrap();
            agent.run_until_idle().await.unwrap();
            agent.followup(user_text("m2", "second")).unwrap();
            agent.run_until_idle().await.unwrap();
            let requests = adapter.requests.lock().unwrap().clone();
            assert_eq!(requests.len(), 2);
            expect_prefix_extension(&requests[0], &requests[1]);
        }

        #[tokio::test]
        async fn header_change_is_logged_when_the_canonical_header_differs() {
            let (mut agent, adapter) = harness(vec![
                MockScript::Chunks(text_response("one")),
                MockScript::Chunks(text_response("two")),
            ]);
            let n = Arc::new(AtomicUsize::new(0));
            agent.on_request({
                let n = n.clone();
                move |config| {
                    let count = n.fetch_add(1, Ordering::SeqCst) + 1;
                    let mut next = config.clone();
                    if count >= 2 {
                        next.model = "other".into();
                    }
                    next
                }
            });
            agent.followup(user_text("m1", "first")).unwrap();
            agent.run_until_idle().await.unwrap();
            agent.followup(user_text("m2", "second")).unwrap();
            agent.run_until_idle().await.unwrap();
            let reasons: Vec<_> = agent
                .session
                .events()
                .iter()
                .filter_map(|e| match e {
                    dsh_session::LogEvent::Known(dsh_session::SessionEvent::RequestHeader {
                        data,
                        ..
                    }) => Some(data.reason.clone()),
                    _ => None,
                })
                .collect();
            assert_eq!(
                reasons,
                vec![RequestHeaderReason::Initial, RequestHeaderReason::Change]
            );
            assert_eq!(adapter.requests.lock().unwrap()[1].model, "other");
        }

        #[tokio::test]
        async fn adapter_default_max_tokens_is_marked_and_stripped_from_the_next_proposal() {
            let adapter = Arc::new(
                MockAdapter::new(vec![
                    MockScript::Chunks(text_response("one")),
                    MockScript::Chunks(text_response("two")),
                ])
                .with_defaults(Some(256), None),
            );
            let mut llm = LlmRuntime::new();
            llm.register_adapter("mock", adapter.clone());
            let mut agent = LoopAgent::new(
                dsh_kernel::Context::new(),
                Session::new(test_header("adapter-max-tokens")),
                LoopOptions {
                    provider: "mock".into(),
                    model: "mock".into(),
                    max_tokens: None,
                    max_parallel_tool_calls: 10,
                },
                Arc::new(std::sync::Mutex::new(ToolRuntime::new(
                    dsh_tools::ToolPresentationMode::Native,
                ))),
                SystemPrompt::new(SystemPromptConfig::default()).unwrap(),
                Arc::new(std::sync::Mutex::new(llm)),
            )
            .unwrap();
            agent.followup(user_text("m1", "first")).unwrap();
            agent.run_until_idle().await.unwrap();
            agent.followup(user_text("m2", "second")).unwrap();
            agent.run_until_idle().await.unwrap();
            let requests = adapter.requests.lock().unwrap();
            assert_eq!(requests[0].max_tokens, Some(256));
            let header = agent.session.request_header().unwrap();
            assert_eq!(header.adapter_defaults.unwrap().max_tokens, Some(true));
            // Second seed strips the adapter default; prepare_call re-materializes 256.
            assert_eq!(requests[1].max_tokens, Some(256));
        }
    }
}
