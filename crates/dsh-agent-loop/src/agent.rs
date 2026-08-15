//! Idle / maintenance / running phase machine, sticky turn reasons, and tool dispatch.

use std::future::Future;

use dsh_llm::{BlockAssembler, GenerateOptions, LlmRuntime, ToolSchema};
use dsh_session::{
    AssistantChunkData, AssistantMessageData, ContentBlock, EpochHeader, FinishReason, InboxTarget,
    LlmCallConfig, LlmFailure, LogEvent, Message, RequestHeaderData, RequestHeaderReason, Session,
    SessionEvent, SessionId, StepBoundaryData, StreamChunk, SurfaceOp, TurnEndData, TurnEndReason,
    TurnStartData, canonical_header,
};
use dsh_system_prompt::{
    AssembleContext, PromptAssembly, SystemPrompt, join_context_sections, render_context_sections,
    render_prompt,
};
use dsh_tools::{AbortFlag, ToolRuntime};
use futures::StreamExt;

use crate::error::LoopError;
use crate::inbox::Inbox;
use crate::runtime_context::RuntimeContextProjection;

#[cfg(test)]
pub use crate::RUNTIME_CONTEXT_SOURCE;

/// Default maximum in-flight parallel-safe tool calls per agent step.
pub const DEFAULT_MAX_PARALLEL_TOOL_CALLS: usize = 10;

type PreStepListener = Box<dyn Fn(&LoopAgent, Vec<Message>) -> PreStepDecision + Send + Sync>;
type RequestListener = Box<dyn Fn(&LlmCallConfig) -> LlmCallConfig + Send + Sync>;
type RequestErrorListener = Box<dyn Fn(&LlmFailure) -> RequestErrorAction + Send + Sync>;

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

/// Recovery choice after a terminal model-request error or abort finish.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestErrorAction {
    /// Repeat `build_request` and the adapter stream.
    Retry,
    /// End the turn as an error.
    Fail,
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
    /// Tool registry used by later tool-call scheduling.
    pub tools: ToolRuntime,
    /// System prompt and runtime-context assembly.
    pub prompt: SystemPrompt,
    /// Adapter registry used to stream model output.
    pub llm: LlmRuntime,
    phase: Phase,
    request_header_logged: bool,
    runtime_context: RuntimeContextProjection,
    pre_step: Vec<PreStepListener>,
    on_request: Vec<RequestListener>,
    on_request_error: Vec<RequestErrorListener>,
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
        session: Session,
        options: LoopOptions,
        tools: ToolRuntime,
        prompt: SystemPrompt,
        llm: LlmRuntime,
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
            phase: Phase::Idle { last_turn },
            request_header_logged: false,
            runtime_context,
            pre_step: Vec::new(),
            on_request: Vec::new(),
            on_request_error: Vec::new(),
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

    /// Append a pre-step listener. An empty listener list uses claimed messages plus an optional snapshot.
    pub fn on_pre_step(
        &mut self,
        listener: impl Fn(&LoopAgent, Vec<Message>) -> PreStepDecision + Send + Sync + 'static,
    ) {
        self.pre_step.push(Box::new(listener));
    }

    /// Append a request-config listener. Listeners run in registration order over the seed config.
    pub fn on_request(
        &mut self,
        listener: impl Fn(&LlmCallConfig) -> LlmCallConfig + Send + Sync + 'static,
    ) {
        self.on_request.push(Box::new(listener));
    }

    /// Append a request-error listener. The last listener's action is used; default is fail.
    pub fn on_request_error(
        &mut self,
        listener: impl Fn(&LlmFailure) -> RequestErrorAction + Send + Sync + 'static,
    ) {
        self.on_request_error.push(Box::new(listener));
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
        if !matches!(self.phase, Phase::Idle { .. }) {
            return Err(self.active_work_error());
        }
        loop {
            let Phase::Idle { last_turn } = self.phase else {
                return Err(self.active_work_error());
            };
            if self.inbox.next_turn().is_empty() && !self.wake_latched {
                return Ok(());
            }
            self.wake_latched = false;
            self.phase = Phase::Running {
                abort: AbortFlag::new(),
                turn: last_turn,
                step: 0,
                wake_requested: false,
            };
            let drive = self.drive_running().await;
            let (last_turn, wake_requested) = match &self.phase {
                Phase::Running {
                    turn,
                    wake_requested,
                    ..
                } => (*turn, *wake_requested),
                Phase::Idle { last_turn } => (*last_turn, false),
                Phase::Maintenance { last_turn, .. } => (*last_turn, false),
            };
            self.phase = Phase::Idle { last_turn };
            drive?;
            if wake_requested && self.inbox.has_pending() {
                self.wake_latched = true;
                continue;
            }
            return Ok(());
        }
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

    async fn drive_running(&mut self) -> Result<(), LoopError> {
        while self.turn().await? {}
        Ok(())
    }

    async fn turn(&mut self) -> Result<bool, LoopError> {
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

        let mut turn_ends: Option<TurnEndReason> = None;
        let body = self.run_turn_steps(turn, &mut turn_ends).await;
        if let Err(error) = &body {
            if turn_ends.is_none() {
                turn_ends = Some(if self.is_aborted() {
                    TurnEndReason::Aborted {
                        reason: self.cancel_reason_json(),
                    }
                } else {
                    TurnEndReason::Error {
                        error: llm_failure_from_loop_error(error),
                    }
                });
            }
        }
        let reason = turn_ends.unwrap_or(TurnEndReason::Completed);
        let aborted_turn = self.is_aborted() || matches!(reason, TurnEndReason::Aborted { .. });
        let end_result = self.push_event(|seq| SessionEvent::TurnEnd {
            seq,
            time: seq as i64,
            data: TurnEndData { turn, reason },
            ignorable: None,
        });
        body?;
        end_result?;
        if aborted_turn {
            return Ok(false);
        }
        if !self.inbox.has_pending() {
            return Ok(false);
        }
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
        Ok(true)
    }

    async fn run_turn_steps(
        &mut self,
        turn: u64,
        turn_ends: &mut Option<TurnEndReason>,
    ) -> Result<(), LoopError> {
        let mut target = InboxTarget::NextTurn;
        loop {
            if self.is_aborted() {
                *turn_ends = Some(TurnEndReason::Aborted {
                    reason: self.cancel_reason_json(),
                });
                return Err(LoopError::Invalid(format!(
                    "agent \"{}\" aborted",
                    self.id.as_str()
                )));
            }
            let (decision, assembly) = self.pre_step(target)?;
            match decision {
                PreStepDecision::Reject => {
                    *turn_ends = Some(TurnEndReason::Blocked);
                    return Ok(());
                }
                PreStepDecision::Enter { messages } => {
                    let step_zero = matches!(self.phase, Phase::Running { step: 0, .. });
                    if step_zero && messages.is_empty() {
                        *turn_ends = Some(TurnEndReason::Completed);
                        return Ok(());
                    }
                    if turn_ends.is_some() && messages.is_empty() {
                        break;
                    }
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
                    let step_result = self.execute_step(&assembly).await;
                    let end_result = self.push_event(|seq| SessionEvent::StepEnd {
                        seq,
                        time: seq as i64,
                        data: StepBoundaryData { turn, step },
                        ignorable: None,
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
                    if self.is_aborted() {
                        *turn_ends = Some(TurnEndReason::Aborted {
                            reason: self.cancel_reason_json(),
                        });
                        return Ok(());
                    }
                    if turn_ends.is_some()
                        && self.inbox.next_step().is_empty()
                        && !matches!(step_end, Some(TurnEndReason::MaxTokens))
                    {
                        break;
                    }
                    target = InboxTarget::NextStep;
                }
            }
        }
        Ok(())
    }

    fn pre_step(
        &mut self,
        target: InboxTarget,
    ) -> Result<(PreStepDecision, PromptAssembly), LoopError> {
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
        let decision = if self.pre_step.is_empty() {
            let mut messages = claimed;
            if let Some(snapshot) = snapshot {
                messages.push(snapshot);
            }
            PreStepDecision::Enter { messages }
        } else {
            let listeners = std::mem::take(&mut self.pre_step);
            let mut messages = claimed;
            let mut reject = false;
            for listener in &listeners {
                match listener(self, std::mem::take(&mut messages)) {
                    PreStepDecision::Reject => {
                        reject = true;
                        break;
                    }
                    PreStepDecision::Enter { messages: next } => messages = next,
                }
            }
            self.pre_step = listeners;
            if reject {
                PreStepDecision::Reject
            } else {
                PreStepDecision::Enter { messages }
            }
        };
        Ok((decision, assembly))
    }

    async fn execute_step(
        &mut self,
        assembly: &PromptAssembly,
    ) -> Result<Option<TurnEndReason>, LoopError> {
        let (turn, step) = match &self.phase {
            Phase::Running { turn, step, .. } => (*turn, *step),
            _ => {
                return Err(LoopError::Invalid(format!(
                    "agent \"{}\": step outside running phase",
                    self.id.as_str()
                )));
            }
        };
        loop {
            if self.is_aborted() {
                return Ok(Some(TurnEndReason::Aborted {
                    reason: self.cancel_reason_json(),
                }));
            }
            let request = self.build_request(assembly)?;
            let provider = request.provider.clone();
            let model = request.model.clone();
            let chunks = self.collect_stream(request).await;
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
                    match self.request_error_action(&failure) {
                        RequestErrorAction::Retry => continue,
                        RequestErrorAction::Fail => {
                            return Ok(Some(TurnEndReason::Error { error: failure }));
                        }
                    }
                }
                finish => {
                    self.append_assistant(turn, step, &assembler, &chunk_seqs, &provider, &model)?;
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
                    let signal = match &self.phase {
                        Phase::Running { abort, .. } | Phase::Maintenance { abort, .. } => {
                            abort.clone()
                        }
                        Phase::Idle { .. } => AbortFlag::new(),
                    };
                    let max_parallel = self.options.max_parallel_tool_calls;
                    let mut extra = Vec::new();
                    let outcome = crate::tool_calls::execute_tool_calls(
                        &mut self.session,
                        &mut self.tools,
                        turn,
                        step,
                        &tool_calls,
                        &signal,
                        max_parallel,
                        &mut |message| extra.push(message),
                    )
                    .await?;
                    for message in extra {
                        let start = self.inbox.next_step().len();
                        self.inbox.splice(
                            &mut self.session,
                            InboxTarget::NextStep,
                            start,
                            0,
                            vec![message],
                            true,
                        )?;
                    }
                    if outcome.aborted || self.is_aborted() {
                        return Ok(Some(TurnEndReason::Aborted {
                            reason: self.cancel_reason_json(),
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

    fn build_request(&mut self, assembly: &PromptAssembly) -> Result<GenerateOptions, LoopError> {
        let system =
            render_prompt(assembly).map_err(|error| LoopError::Prompt(error.to_string()))?;
        let messages = self.session.derive_messages();
        let mut config = LlmCallConfig {
            provider: self.options.provider.clone(),
            model: self.options.model.clone(),
            reasoning_effort: None,
            temperature: None,
            max_tokens: self.options.max_tokens,
            stop: None,
        };
        for listener in &self.on_request {
            config = listener(&config);
        }
        let (config, adapter_defaults) = match self.llm.prepare_call(&config) {
            Ok(prepared) => (prepared.config, Some(prepared.adapter_defaults)),
            Err(_) => (config, None),
        };
        let header = canonical_header(&EpochHeader {
            config: config.clone(),
            adapter_defaults,
            system: (!system.is_empty()).then_some(system),
            tools: tools_json(&assembly.tools),
        });
        if !self.request_header_logged {
            let reason = if self.session.request_header().is_some() {
                RequestHeaderReason::Resume
            } else {
                RequestHeaderReason::Initial
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
        }
        let signal = match &self.phase {
            Phase::Running { abort, .. } | Phase::Maintenance { abort, .. } => abort.clone(),
            Phase::Idle { .. } => AbortFlag::new(),
        };
        Ok(GenerateOptions {
            provider: header.config.provider.clone(),
            model: header.config.model.clone(),
            reasoning_effort: header.config.reasoning_effort.clone(),
            messages,
            system: header.system.clone(),
            tools: (!assembly.tools.is_empty()).then(|| assembly.tools.clone()),
            temperature: header.config.temperature,
            max_tokens: header.config.max_tokens,
            stop: header.config.stop.clone(),
            signal,
            session_id: Some(self.id.clone()),
            purpose: None,
        })
    }

    async fn collect_stream(&mut self, request: GenerateOptions) -> Vec<StreamChunk> {
        let llm = std::mem::replace(&mut self.llm, LlmRuntime::new());
        let mut stream = llm.stream(request);
        let mut chunks = Vec::new();
        while let Some(chunk) = stream.next().await {
            chunks.push(chunk);
        }
        drop(stream);
        self.llm = llm;
        chunks
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

    fn request_error_action(&self, failure: &LlmFailure) -> RequestErrorAction {
        let mut action = RequestErrorAction::Fail;
        for listener in &self.on_request_error {
            action = listener(failure);
        }
        action
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

fn tools_json(tools: &[ToolSchema]) -> Option<serde_json::Value> {
    if tools.is_empty() {
        return None;
    }
    Some(serde_json::Value::Array(
        tools
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.parameters,
                })
            })
            .collect(),
    ))
}

#[cfg(test)]
pub(crate) use crate::{event_types, test_header, user_text};

#[cfg(test)]
mod tests {
    use super::{
        AgentStatus, CancelCause, CancelOptions, LoopAgent, LoopOptions, Phase, PreStepDecision,
        event_types, test_header, user_text,
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
            Session::new(test_header("loop-1")),
            LoopOptions {
                provider: "mock".into(),
                model: "mock".into(),
                max_tokens: None,
                max_parallel_tool_calls: 10,
            },
            ToolRuntime::new(dsh_tools::ToolPresentationMode::Native),
            SystemPrompt::new(SystemPromptConfig::default()).unwrap(),
            llm,
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
        agent.on_pre_step(move |agent, claimed| {
            let types = event_types(&agent.session);
            assert!(types.iter().any(|t| t == "turn/start"));
            assert!(!types.iter().any(|t| t == "step/start"));
            saw.store(true, Ordering::SeqCst);
            assert_eq!(claimed.len(), 1);
            PreStepDecision::Enter { messages: claimed }
        });
        agent.followup(user_text("m1", "hi")).unwrap();
        agent.run_until_idle().await.unwrap();
        assert!(saw_turn_before_claim.load(Ordering::SeqCst));
        assert_eq!(adapter.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn empty_first_claim_logs_a_turn_without_a_step() {
        let (mut agent, adapter) = harness(vec![MockScript::Chunks(text_response("must not run"))]);
        agent.on_pre_step(|_, _| PreStepDecision::Enter { messages: vec![] });
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
            Session::new(test_header("snap")),
            LoopOptions {
                provider: "mock".into(),
                model: "mock".into(),
                max_tokens: None,
                max_parallel_tool_calls: 10,
            },
            ToolRuntime::new(dsh_tools::ToolPresentationMode::Native),
            prompt,
            llm,
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
            move |_, claimed| {
                let step = n.fetch_add(1, Ordering::SeqCst) + 1;
                if step == 1 {
                    PreStepDecision::Enter { messages: claimed }
                } else {
                    PreStepDecision::Enter {
                        messages: vec![user_text("cont", "continue after truncation")],
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
            Session::new(test_header("abort-drain")),
            LoopOptions {
                provider: "mock".into(),
                model: "mock".into(),
                max_tokens: None,
                max_parallel_tool_calls: 1,
            },
            tools,
            SystemPrompt::new(SystemPromptConfig::default()).unwrap(),
            llm,
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
            Session::new(test_header("par")),
            LoopOptions {
                provider: "mock".into(),
                model: "mock".into(),
                max_tokens: None,
                max_parallel_tool_calls: 10,
            },
            tools,
            SystemPrompt::new(SystemPromptConfig::default()).unwrap(),
            llm,
        )
        .unwrap();
        agent.followup(user_text("m1", "go")).unwrap();
        agent.run_until_idle().await.unwrap();
        assert!(
            max_live.load(Ordering::SeqCst) >= 2,
            "parallel siblings must overlap"
        );
    }
}
