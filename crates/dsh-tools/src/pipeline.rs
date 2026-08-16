//! Pre-execute, approval, guard, execute, and post-execute pipeline.
//!
//! Under [`ToolPresentationMode::Code`], a model-direct call to a registered
//! name other than [`RUN_CODE_NAME`] is denied before pre-execute.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use dsh_session::{ContentBlock, Session};

use crate::freeze::freeze_args;
use crate::{
    PostToolDecision, PreToolDecision, RUN_CODE_NAME, TOOL_ABORTED, TOOL_ABORTED_BEFORE_DISPATCH,
    ToolError, ToolErrorInfo, ToolExecution, ToolExecutionInput, ToolExecutionResult,
    ToolExecutionToken, ToolFailure, ToolPresentationMode,
};

/// Guard that may deny a call after pre-execute [`PreToolDecision::Allow`].
pub type ToolGuard = Box<dyn Fn(&ToolExecution) -> Option<String> + Send + Sync>;

/// Tool body. Receives frozen arguments and the in-flight execution.
pub type ToolBody = Box<
    dyn Fn(
            serde_json::Value,
            ToolExecution,
        ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, ToolError>> + Send>>
        + Send
        + Sync,
>;

/// Renders a successful value into model-visible content blocks.
pub type ToolRender =
    Box<dyn Fn(&serde_json::Value, &serde_json::Value) -> Vec<ContentBlock> + Send + Sync>;

/// Returns whether this call may overlap other parallel calls.
pub type ToolConcurrencySafe = Box<dyn Fn(&serde_json::Value) -> bool + Send + Sync>;

type StoredBody = Arc<
    dyn Fn(
            serde_json::Value,
            ToolExecution,
        ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, ToolError>> + Send>>
        + Send
        + Sync,
>;
type StoredRender =
    Arc<dyn Fn(&serde_json::Value, &serde_json::Value) -> Vec<ContentBlock> + Send + Sync>;
type StoredClassify = Arc<dyn Fn(&serde_json::Value) -> bool + Send + Sync>;

type PreNext = Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = PreToolDecision> + Send>> + Send>;
type PreFn = Arc<
    dyn Fn(ToolExecution, PreNext) -> Pin<Box<dyn Future<Output = PreToolDecision> + Send>>
        + Send
        + Sync,
>;
type PostNext = Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = PostToolDecision> + Send>> + Send>;
type PostFn = Arc<
    dyn Fn(
            ToolExecution,
            ToolExecutionResult,
            PostNext,
        ) -> Pin<Box<dyn Future<Output = PostToolDecision> + Send>>
        + Send
        + Sync,
>;
type ApprovalHook = Box<
    dyn Fn(ToolExecution, Option<String>) -> Pin<Box<dyn Future<Output = ApprovalOutcome> + Send>>
        + Send
        + Sync,
>;
type StoredGuard = Arc<dyn Fn(&ToolExecution) -> Option<String> + Send + Sync>;

/// Decides a pre-execute [`PreToolDecision::Ask`] using the asking session log.
pub trait Approver: Send + Sync {
    /// Return one closed outcome for `exec`. The future may borrow `self` and `session`.
    fn decide<'a>(
        &'a self,
        session: &'a mut Session,
        exec: &'a ToolExecution,
        reason: Option<String>,
    ) -> Pin<Box<dyn Future<Output = ApprovalOutcome> + Send + 'a>>;
}

struct HookApprover {
    hook: ApprovalHook,
}

impl Approver for HookApprover {
    fn decide<'a>(
        &'a self,
        _session: &'a mut Session,
        exec: &'a ToolExecution,
        reason: Option<String>,
    ) -> Pin<Box<dyn Future<Output = ApprovalOutcome> + Send + 'a>> {
        (self.hook)(exec.clone(), reason)
    }
}

/// Registered tool: schema, body, render, and optional concurrency classifier.
pub struct ToolDefinition {
    /// Tool name as the model would call it.
    pub name: String,
    /// Model-facing description.
    pub description: String,
    /// JSON Schema for arguments.
    pub parameters: serde_json::Value,
    /// Body invoked after policy allows the call.
    pub execute: ToolBody,
    /// Renders a successful body value.
    pub render: ToolRender,
    /// When present, `true` is the only value that schedules the call as parallel.
    pub is_concurrency_safe: Option<ToolConcurrencySafe>,
}

/// How the scheduler may overlap this call with others.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolExecutionMode {
    /// The visible tool's classifier returned exactly `true`.
    Parallel,
    /// Unknown, collapsed, undeclared, or not exactly `true`.
    Exclusive,
}

/// Outcome of [`ToolRuntime::prepare`].
#[derive(Clone, Debug)]
pub enum ScheduledToolPreparation {
    /// Policy allowed the call; [`ToolRuntime::dispatch`] may run the body.
    Dispatch {
        /// Tokenized execution passed to dispatch and finalize.
        exec: ToolExecution,
    },
    /// Denied or aborted after policy; still runs [`ToolRuntime::finalize`].
    PostResult {
        /// Tokenized execution for post-execute.
        exec: ToolExecution,
        /// Result before post-execute.
        result: ToolExecutionResult,
    },
    /// Collapse, freeze failure, or abort before policy; skip post-execute.
    FinalResult {
        /// Tokenized execution that never reached the body.
        exec: ToolExecution,
        /// Final result.
        result: ToolExecutionResult,
    },
}

/// Outcome of [`ToolRuntime::dispatch`].
#[derive(Clone, Debug)]
pub enum ScheduledToolDispatch {
    /// Body finished; still runs [`ToolRuntime::finalize`].
    PostResult {
        /// Result before post-execute.
        result: ToolExecutionResult,
    },
    /// Pipeline failure that skips post-execute.
    FinalResult {
        /// Final result.
        result: ToolExecutionResult,
    },
}

#[derive(Clone)]
struct RegisteredTool {
    execute: StoredBody,
    render: StoredRender,
    is_concurrency_safe: Option<StoredClassify>,
}

/// Outcome of an approval hook after pre-execute [`PreToolDecision::Ask`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalOutcome {
    /// Continue this call once.
    AllowedOnce,
    /// The user refused the call.
    Rejected,
    /// The approval request was cancelled.
    Cancelled,
    /// No approval channel is available.
    Unavailable,
}

impl ApprovalOutcome {
    /// Session-log and TypeScript wire string.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AllowedOnce => "allowed-once",
            Self::Rejected => "rejected",
            Self::Cancelled => "cancelled",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Frozen prepare state that can finish without holding [`ToolRuntime`].
pub struct PrepareSnapshot {
    exec: ToolExecution,
    early: Option<ScheduledToolPreparation>,
    pre: Vec<PreFn>,
    approver: Option<Arc<dyn Approver>>,
    guards: Vec<StoredGuard>,
}

impl PrepareSnapshot {
    fn early(result: ScheduledToolPreparation) -> Self {
        Self {
            exec: match &result {
                ScheduledToolPreparation::Dispatch { exec }
                | ScheduledToolPreparation::PostResult { exec, .. }
                | ScheduledToolPreparation::FinalResult { exec, .. } => exec.clone(),
            },
            early: Some(result),
            pre: Vec::new(),
            approver: None,
            guards: Vec::new(),
        }
    }

    /// Run pre-execute, approval, and guards on the Tokio worker.
    pub async fn finish(self, session: Option<&mut Session>) -> ScheduledToolPreparation {
        if let Some(early) = self.early {
            return early;
        }
        let exec = self.exec;
        let pre = run_pre(self.pre, 0, exec.clone()).await;
        let (decision, approval_cancelled) = match pre {
            PreToolDecision::Ask { reason } => {
                resolve_ask(self.approver.as_ref(), session, &exec, reason).await
            }
            other => (other, false),
        };
        if approval_cancelled && exec.signal.is_aborted() {
            return ScheduledToolPreparation::PostResult {
                exec,
                result: aborted_before_dispatch(),
            };
        }
        let denial = match decision {
            PreToolDecision::Allow => self.guards.iter().find_map(|guard| guard(&exec)),
            PreToolDecision::Deny { reason } => Some(reason),
            PreToolDecision::Ask { reason } => {
                Some(reason.unwrap_or_else(|| unavailable_reason(&exec.name)))
            }
        };
        if let Some(reason) = denial {
            return ScheduledToolPreparation::PostResult {
                exec,
                result: deny_result(reason),
            };
        }
        if exec.signal.is_aborted() {
            return ScheduledToolPreparation::PostResult {
                exec,
                result: aborted_before_dispatch(),
            };
        }
        ScheduledToolPreparation::Dispatch { exec }
    }
}

/// Flat-map tool registry that runs the policy and dispatch pipeline.
pub struct ToolRuntime {
    mode: ToolPresentationMode,
    next_token: u64,
    tools: HashMap<String, RegisteredTool>,
    pre: Vec<PreFn>,
    post: Vec<PostFn>,
    guards: Vec<StoredGuard>,
    approver: Option<Arc<dyn Approver>>,
}

impl Clone for ToolRuntime {
    fn clone(&self) -> Self {
        Self {
            mode: self.mode,
            next_token: self.next_token,
            tools: self.tools.clone(),
            pre: self.pre.clone(),
            post: self.post.clone(),
            guards: Vec::new(),
            approver: None,
        }
    }
}

impl ToolRuntime {
    /// Create an empty runtime that presents tools in `mode`.
    #[must_use]
    pub fn new(mode: ToolPresentationMode) -> Self {
        Self {
            mode,
            next_token: 1,
            tools: HashMap::new(),
            pre: Vec::new(),
            post: Vec::new(),
            guards: Vec::new(),
            approver: None,
        }
    }

    /// Register `definition`. A later registration of the same name replaces the earlier one.
    ///
    /// Does not reserve [`RUN_CODE_NAME`]; collapse only denies model-direct native names.
    pub fn register(&mut self, definition: ToolDefinition) {
        self.tools.insert(
            definition.name.clone(),
            RegisteredTool {
                execute: Arc::from(definition.execute),
                render: Arc::from(definition.render),
                is_concurrency_safe: definition.is_concurrency_safe.map(Arc::from),
            },
        );
    }

    /// Model-facing names currently registered.
    #[must_use]
    pub fn registered_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.tools.keys().cloned().collect();
        names.sort();
        names
    }

    /// Append a pre-execute waterfall listener.
    ///
    /// The listener must call `next()` to delegate. Returning
    /// [`PreToolDecision::Deny`] or [`PreToolDecision::Ask`] without `next()`
    /// short-circuits. The terminal default is [`PreToolDecision::Allow`].
    pub fn on_pre<F>(&mut self, listener: F)
    where
        F: Fn(ToolExecution, PreNext) -> Pin<Box<dyn Future<Output = PreToolDecision> + Send>>
            + Send
            + Sync
            + 'static,
    {
        self.pre.push(Arc::new(listener));
    }

    /// Append a post-execute waterfall listener.
    ///
    /// The listener must call `next()` to delegate. Returning
    /// [`PostToolDecision::Block`] without `next()` short-circuits. The terminal
    /// default is [`PostToolDecision::Accept`] with empty replacements.
    pub fn on_post<F>(&mut self, listener: F)
    where
        F: Fn(
                ToolExecution,
                ToolExecutionResult,
                PostNext,
            ) -> Pin<Box<dyn Future<Output = PostToolDecision> + Send>>
            + Send
            + Sync
            + 'static,
    {
        self.post.push(Arc::new(listener));
    }

    /// Append a monotonic guard evaluated only after [`PreToolDecision::Allow`].
    ///
    /// The first `Some(reason)` denies the call with content `Error: {reason}`.
    pub fn guard(&mut self, guard: ToolGuard) {
        self.guards.push(Arc::from(guard));
    }

    /// Install or clear the [`Approver`] used when pre-execute returns Ask.
    ///
    /// `None` degrades Ask to [`ApprovalOutcome::Unavailable`].
    pub fn set_approver(&mut self, approver: Option<Arc<dyn Approver>>) {
        self.approver = approver;
    }

    /// Install or clear a session-free approval hook. Thin wrapper over [`set_approver`].
    ///
    /// `None` degrades Ask to deny.
    pub fn set_approval(&mut self, approval: Option<ApprovalHook>) {
        self.approver = approval.map(|hook| Arc::new(HookApprover { hook }) as Arc<dyn Approver>);
    }

    /// Parallel only when the visible, non-collapsed tool's classifier returns exactly `true`.
    #[must_use]
    pub fn execution_mode(&self, input: &ToolExecutionInput) -> ToolExecutionMode {
        let visible = self.tools.contains_key(&input.name);
        if self.collapses(visible, &input.name, input.parent.is_none()) {
            return ToolExecutionMode::Exclusive;
        }
        match self
            .tools
            .get(&input.name)
            .and_then(|tool| tool.is_concurrency_safe.as_ref())
        {
            Some(classify) if classify(&input.arguments) => ToolExecutionMode::Parallel,
            _ => ToolExecutionMode::Exclusive,
        }
    }

    /// Freeze, collapse, and abort-before-policy without `.await`.
    ///
    /// Callers that hold a tools mutex must drop it before
    /// [`PrepareSnapshot::finish`].
    pub fn begin_prepare(&mut self, input: ToolExecutionInput) -> PrepareSnapshot {
        let token = ToolExecutionToken(self.next_token);
        self.next_token += 1;
        let root_call_id = input.root_call_id.unwrap_or_else(|| input.call_id.clone());
        let arguments = match freeze_args(&input.arguments) {
            Ok(args) => args,
            Err(error) => {
                let exec = ToolExecution {
                    token,
                    call_id: input.call_id,
                    root_call_id,
                    name: input.name,
                    arguments: input.arguments,
                    parent: input.parent,
                    session_id: input.session_id,
                    signal: input.signal,
                };
                return PrepareSnapshot::early(ScheduledToolPreparation::FinalResult {
                    exec,
                    result: result_from_tool_error(&error),
                });
            }
        };
        let exec = ToolExecution {
            token,
            call_id: input.call_id,
            root_call_id,
            name: input.name,
            arguments,
            parent: input.parent,
            session_id: input.session_id,
            signal: input.signal,
        };
        let visible = self.tools.contains_key(&exec.name);
        let collapsed = self.collapses(visible, &exec.name, exec.parent.is_none());
        if collapsed && exec.signal.is_aborted() {
            return PrepareSnapshot::early(ScheduledToolPreparation::FinalResult {
                exec,
                result: aborted_before_dispatch(),
            });
        }
        if collapsed {
            return PrepareSnapshot::early(ScheduledToolPreparation::FinalResult {
                exec: exec.clone(),
                result: result_from_tool_error(&ToolError::UnknownToolHint {
                    name: exec.name.clone(),
                    hint: collapse_hint(&exec.name),
                }),
            });
        }
        if exec.signal.is_aborted() {
            return PrepareSnapshot::early(ScheduledToolPreparation::FinalResult {
                exec,
                result: aborted_before_dispatch(),
            });
        }
        PrepareSnapshot {
            exec,
            early: None,
            pre: self.pre.clone(),
            approver: self.approver.clone(),
            guards: self.guards.clone(),
        }
    }

    /// Freeze, collapse, abort-before-body, pre-execute, approval, and guards.
    ///
    /// Assigns a real execution token. A deny, unknown-tool failure after policy, or
    /// abort after policy is [`ScheduledToolPreparation::PostResult`]. Collapse, freeze
    /// failure, and abort before policy are [`ScheduledToolPreparation::FinalResult`].
    /// Ask with no [`Approver`] uses the Unavailable deny text.
    pub async fn prepare(&mut self, input: ToolExecutionInput) -> ScheduledToolPreparation {
        self.begin_prepare(input).finish(None).await
    }

    /// [`prepare`](Self::prepare) with a session for [`Approver::decide`].
    pub async fn prepare_with_session(
        &mut self,
        session: &mut Session,
        input: ToolExecutionInput,
    ) -> ScheduledToolPreparation {
        self.begin_prepare(input).finish(Some(session)).await
    }

    /// Run `definition.execute` and render. The future does not borrow `self`.
    ///
    /// Call only after [`prepare`](Self::prepare) returned
    /// [`ScheduledToolPreparation::Dispatch`]. Unknown names fail as a post-result so
    /// [`finalize`](Self::finalize) still runs. Overlapping callers join these futures
    /// without holding `&mut ToolRuntime` across the body `.await`.
    #[must_use]
    pub fn dispatch(
        &self,
        exec: &ToolExecution,
    ) -> Pin<Box<dyn Future<Output = ScheduledToolDispatch> + Send + 'static>> {
        let exec = exec.clone();
        let Some(tool) = self.tools.get(&exec.name) else {
            let result = result_from_tool_error(&ToolError::UnknownTool(exec.name.clone()));
            return Box::pin(async move { ScheduledToolDispatch::PostResult { result } });
        };
        let execute = Arc::clone(&tool.execute);
        let render = Arc::clone(&tool.render);
        let body = execute(exec.arguments.clone(), exec.clone());
        Box::pin(async move {
            match body.await {
                Ok(value) => {
                    if exec.signal.is_aborted() {
                        ScheduledToolDispatch::PostResult {
                            result: aborted_after_body(),
                        }
                    } else {
                        let content = render(&exec.arguments, &value);
                        ScheduledToolDispatch::PostResult {
                            result: ToolExecutionResult::Success {
                                value,
                                content,
                                meta: None,
                                additional_contexts: vec![],
                                concludes_turn: false,
                            },
                        }
                    }
                }
                Err(error) => ScheduledToolDispatch::PostResult {
                    result: result_from_tool_error(&error),
                },
            }
        })
    }

    /// Run post-execute listeners over `result`.
    pub async fn finalize(
        &self,
        exec: &ToolExecution,
        result: ToolExecutionResult,
    ) -> ToolExecutionResult {
        self.finish_post(exec, result).await
    }

    /// Run freeze → collapse → pre → approval → guards → body → post.
    ///
    /// Composes [`prepare`](Self::prepare), [`dispatch`](Self::dispatch), and
    /// [`finalize`](Self::finalize). A collapsed model-direct call never reaches
    /// pre-execute. Exclusive callers may use this for the whole pipeline.
    pub async fn execute(&mut self, input: ToolExecutionInput) -> ToolExecutionResult {
        let prepared = self.prepare(input).await;
        self.continue_after_prepare(prepared).await
    }

    /// [`execute`](Self::execute) with a session for [`Approver::decide`].
    pub async fn execute_with_session(
        &mut self,
        session: &mut Session,
        input: ToolExecutionInput,
    ) -> ToolExecutionResult {
        let prepared = self.prepare_with_session(session, input).await;
        self.continue_after_prepare(prepared).await
    }

    async fn continue_after_prepare(
        &mut self,
        prepared: ScheduledToolPreparation,
    ) -> ToolExecutionResult {
        match prepared {
            ScheduledToolPreparation::Dispatch { exec } => {
                let dispatched = self.dispatch(&exec).await;
                match dispatched {
                    ScheduledToolDispatch::PostResult { result } => {
                        self.finalize(&exec, result).await
                    }
                    ScheduledToolDispatch::FinalResult { result } => result,
                }
            }
            ScheduledToolPreparation::PostResult { exec, result } => {
                self.finalize(&exec, result).await
            }
            ScheduledToolPreparation::FinalResult { result, .. } => result,
        }
    }

    fn collapses(&self, visible: bool, name: &str, model_direct: bool) -> bool {
        visible && model_direct && self.mode == ToolPresentationMode::Code && name != RUN_CODE_NAME
    }

    async fn finish_post(
        &self,
        exec: &ToolExecution,
        result: ToolExecutionResult,
    ) -> ToolExecutionResult {
        let decision = run_post(self.post.clone(), 0, exec.clone(), result.clone()).await;
        apply_post_decision(result, decision)
    }
}

fn unavailable_reason(name: &str) -> String {
    format!("tool \"{name}\" requires approval, but no approval channel is available")
}

async fn resolve_ask(
    approver: Option<&Arc<dyn Approver>>,
    session: Option<&mut Session>,
    exec: &ToolExecution,
    reason: Option<String>,
) -> (PreToolDecision, bool) {
    let Some(approver) = approver else {
        return (
            PreToolDecision::Deny {
                reason: unavailable_reason(&exec.name),
            },
            false,
        );
    };
    let Some(session) = session else {
        return (
            PreToolDecision::Deny {
                reason: unavailable_reason(&exec.name),
            },
            false,
        );
    };
    let outcome = approver.decide(session, exec, reason).await;
    match outcome {
        ApprovalOutcome::AllowedOnce => (PreToolDecision::Allow, false),
        ApprovalOutcome::Rejected => (
            PreToolDecision::Deny {
                reason: format!("the user rejected tool \"{}\"", exec.name),
            },
            false,
        ),
        ApprovalOutcome::Cancelled => (
            PreToolDecision::Deny {
                reason: format!("approval for tool \"{}\" was cancelled", exec.name),
            },
            true,
        ),
        ApprovalOutcome::Unavailable => (
            PreToolDecision::Deny {
                reason: unavailable_reason(&exec.name),
            },
            false,
        ),
    }
}

fn run_pre(
    listeners: Vec<PreFn>,
    index: usize,
    exec: ToolExecution,
) -> Pin<Box<dyn Future<Output = PreToolDecision> + Send>> {
    Box::pin(async move {
        if index >= listeners.len() {
            return PreToolDecision::Allow;
        }
        let current = Arc::clone(&listeners[index]);
        let next_exec = exec.clone();
        let next_listeners = listeners.clone();
        let next: PreNext = Box::new(move || run_pre(next_listeners, index + 1, next_exec));
        current(exec, next).await
    })
}

fn run_post(
    listeners: Vec<PostFn>,
    index: usize,
    exec: ToolExecution,
    result: ToolExecutionResult,
) -> Pin<Box<dyn Future<Output = PostToolDecision> + Send>> {
    Box::pin(async move {
        if index >= listeners.len() {
            return PostToolDecision::Accept {
                content: None,
                value: None,
                additional_contexts: vec![],
            };
        }
        let current = Arc::clone(&listeners[index]);
        let next_exec = exec.clone();
        let next_result = result.clone();
        let next_listeners = listeners.clone();
        let next: PostNext =
            Box::new(move || run_post(next_listeners, index + 1, next_exec, next_result));
        current(exec, result, next).await
    })
}

fn apply_post_decision(
    result: ToolExecutionResult,
    decision: PostToolDecision,
) -> ToolExecutionResult {
    match decision {
        PostToolDecision::Block {
            feedback,
            additional_contexts,
        } => ToolExecutionResult::Failure {
            error: ToolFailure {
                message: failure_message_from_content(&feedback),
                info: None,
            },
            content: feedback,
            meta: None,
            additional_contexts,
        },
        PostToolDecision::Accept {
            content,
            value,
            additional_contexts,
        } => {
            if content.is_some() && value.is_some() {
                return result_from_tool_error(&ToolError::Other(
                    "tools/post-execute accept decision cannot replace both value and content"
                        .into(),
                ));
            }
            apply_accept(result, content, value, additional_contexts)
        }
    }
}

fn apply_accept(
    result: ToolExecutionResult,
    content: Option<Vec<ContentBlock>>,
    value: Option<serde_json::Value>,
    extra: Vec<dsh_session::Message>,
) -> ToolExecutionResult {
    match result {
        ToolExecutionResult::Success {
            value: old_value,
            content: old_content,
            meta,
            mut additional_contexts,
            concludes_turn,
        } => {
            additional_contexts.extend(extra);
            ToolExecutionResult::Success {
                value: value.unwrap_or(old_value),
                content: content.unwrap_or(old_content),
                meta,
                additional_contexts,
                concludes_turn,
            }
        }
        ToolExecutionResult::Failure {
            error,
            content: old_content,
            meta,
            mut additional_contexts,
        } => {
            if value.is_some() {
                return result_from_tool_error(&ToolError::Other(
                    "tools/post-execute cannot replace the value of a failed result".into(),
                ));
            }
            additional_contexts.extend(extra);
            ToolExecutionResult::Failure {
                error,
                content: content.unwrap_or(old_content),
                meta,
                additional_contexts,
            }
        }
    }
}

fn failure_message_from_content(content: &[ContentBlock]) -> String {
    let text = content
        .iter()
        .map(|block| match block {
            ContentBlock::Text { text } => text.clone(),
            ContentBlock::Reasoning { .. } => "[reasoning content]".into(),
            ContentBlock::Image { .. } => "[image content]".into(),
            ContentBlock::ToolCall { .. } => "[tool-call content]".into(),
            ContentBlock::ToolResult { .. } => "[tool-result content]".into(),
        })
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() {
        "tool result blocked by post-execute policy".into()
    } else {
        text
    }
}

fn collapse_hint(name: &str) -> String {
    format!(
        "only `{run}` is callable directly — call `{name}` from inside a `{run}` program instead",
        run = RUN_CODE_NAME,
    )
}

fn deny_result(reason: String) -> ToolExecutionResult {
    ToolExecutionResult::Failure {
        error: ToolFailure {
            message: reason.clone(),
            info: None,
        },
        content: vec![ContentBlock::Text {
            text: format!("Error: {reason}"),
        }],
        meta: None,
        additional_contexts: vec![],
    }
}

fn result_from_tool_error(error: &ToolError) -> ToolExecutionResult {
    let message = error.to_string();
    let info = match error {
        ToolError::UnknownTool(_) | ToolError::UnknownToolHint { .. } => Some(ToolErrorInfo {
            name: "ToolNotFoundError".into(),
            code: "UNKNOWN_TOOL".into(),
        }),
        ToolError::Coded { name, code, .. } => Some(ToolErrorInfo {
            name: name.clone(),
            code: code.clone(),
        }),
        ToolError::ArgsNotJson | ToolError::Other(_) => None,
    };
    ToolExecutionResult::Failure {
        error: ToolFailure {
            message: message.clone(),
            info,
        },
        content: vec![ContentBlock::Text {
            text: format!("Error: {message}"),
        }],
        meta: None,
        additional_contexts: vec![],
    }
}

fn aborted_before_dispatch() -> ToolExecutionResult {
    ToolExecutionResult::Failure {
        error: ToolFailure {
            message: "tool call aborted before dispatch".into(),
            info: Some(ToolErrorInfo {
                name: "AbortError".into(),
                code: TOOL_ABORTED_BEFORE_DISPATCH.into(),
            }),
        },
        content: vec![ContentBlock::Text {
            text: "Error: tool call aborted before dispatch".into(),
        }],
        meta: None,
        additional_contexts: vec![],
    }
}

fn aborted_after_body() -> ToolExecutionResult {
    ToolExecutionResult::Failure {
        error: ToolFailure {
            message: "tool call aborted".into(),
            info: Some(ToolErrorInfo {
                name: "AbortError".into(),
                code: TOOL_ABORTED.into(),
            }),
        },
        content: vec![ContentBlock::Text {
            text: "Error: tool call aborted".into(),
        }],
        meta: None,
        additional_contexts: vec![],
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        AbortFlag, ApprovalOutcome, Approver, PostToolDecision, PreToolDecision, RUN_CODE_NAME,
        TOOL_ABORTED, TOOL_ABORTED_BEFORE_DISPATCH, ToolDefinition, ToolError, ToolExecutionInput,
        ToolExecutionMode, ToolExecutionResult, ToolPresentationMode, ToolRuntime,
    };
    use dsh_session::{
        CallId, ContentBlock, SESSION_FORMAT_VERSION, Session, SessionHeader, SessionId,
    };
    use serde_json::json;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn echo_tool() -> ToolDefinition {
        ToolDefinition {
            name: "echo".into(),
            description: "echo back".into(),
            parameters: json!({"type": "object"}),
            execute: Box::new(|args, _exec| {
                Box::pin(async move { Ok(args.get("text").cloned().unwrap_or(json!(""))) })
            }),
            render: Box::new(|_args, value| {
                vec![ContentBlock::Text {
                    text: format!("echo: {}", value.as_str().unwrap_or("")),
                }]
            }),
            is_concurrency_safe: None,
        }
    }

    fn input(name: &str, args: serde_json::Value, signal: AbortFlag) -> ToolExecutionInput {
        ToolExecutionInput {
            call_id: CallId::new("c1"),
            root_call_id: None,
            name: name.into(),
            arguments: args,
            parent: None,
            session_id: None,
            signal,
        }
    }

    #[tokio::test]
    async fn execute_echo_returns_rendered_success() {
        let mut tools = ToolRuntime::new(ToolPresentationMode::Native);
        tools.register(echo_tool());
        let result = tools
            .execute(input("echo", json!({"text": "ping"}), AbortFlag::new()))
            .await;
        assert!(!result.is_error());
        assert_eq!(
            result.content(),
            &[ContentBlock::Text {
                text: "echo: ping".into()
            }]
        );
    }

    #[tokio::test]
    async fn pre_execute_deny_skips_the_body() {
        // overwrite: register last-wins for this test crate (Phase 3 flat map).
        let mut tools = ToolRuntime::new(ToolPresentationMode::Native);
        let ran = Arc::new(AtomicUsize::new(0));
        let ran_body = ran.clone();
        tools.register(ToolDefinition {
            name: "echo".into(),
            description: "echo".into(),
            parameters: json!({}),
            execute: Box::new(move |args, _| {
                ran_body.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move { Ok(args) })
            }),
            render: Box::new(|_, v| {
                vec![ContentBlock::Text {
                    text: v.as_str().unwrap_or("").into(),
                }]
            }),
            is_concurrency_safe: None,
        });
        tools.on_pre(|_exec, _next| {
            Box::pin(async {
                PreToolDecision::Deny {
                    reason: "denied by policy".into(),
                }
            })
        });
        let result = tools
            .execute(input("echo", json!({"text": "hi"}), AbortFlag::new()))
            .await;
        assert!(result.is_error());
        assert_eq!(
            result.content(),
            &[ContentBlock::Text {
                text: "Error: denied by policy".into()
            }]
        );
        assert_eq!(ran.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn ask_without_approval_degrades_to_deny() {
        let mut tools = ToolRuntime::new(ToolPresentationMode::Native);
        tools.register(echo_tool());
        tools.on_pre(|_exec, _next| {
            Box::pin(async {
                PreToolDecision::Ask {
                    reason: Some("needs approval".into()),
                }
            })
        });
        let result = tools
            .execute(input("echo", json!({"text": "hi"}), AbortFlag::new()))
            .await;
        assert!(result.is_error());
        assert_eq!(
            result.content(),
            &[ContentBlock::Text {
                text:
                    "Error: tool \"echo\" requires approval, but no approval channel is available"
                        .into()
            }]
        );
    }

    #[tokio::test]
    async fn ask_without_reason_uses_unavailable_channel_text() {
        let mut tools = ToolRuntime::new(ToolPresentationMode::Native);
        tools.register(echo_tool());
        tools.on_pre(|_exec, _next| Box::pin(async { PreToolDecision::Ask { reason: None } }));
        let result = tools
            .execute(input("echo", json!({"text": "hi"}), AbortFlag::new()))
            .await;
        assert_eq!(
            result.content(),
            &[ContentBlock::Text {
                text:
                    "Error: tool \"echo\" requires approval, but no approval channel is available"
                        .into()
            }]
        );
    }

    #[tokio::test]
    async fn ask_with_approver_allowed_once_dispatches() {
        struct AllowAll;
        impl Approver for AllowAll {
            fn decide<'a>(
                &'a self,
                _session: &'a mut Session,
                _exec: &'a crate::ToolExecution,
                _reason: Option<String>,
            ) -> Pin<Box<dyn Future<Output = ApprovalOutcome> + Send + 'a>> {
                Box::pin(async { ApprovalOutcome::AllowedOnce })
            }
        }
        let mut tools = ToolRuntime::new(ToolPresentationMode::Native);
        tools.register(echo_tool());
        tools.set_approver(Some(Arc::new(AllowAll)));
        tools.on_pre(|_exec, _next| Box::pin(async { PreToolDecision::Ask { reason: None } }));
        let mut session = Session::new(SessionHeader {
            version: SESSION_FORMAT_VERSION,
            id: SessionId::new("ask-approver"),
            created_at: 1,
            cwd: None,
            parent_session: None,
            seed_length: None,
            origin: None,
            delegation_depth: None,
            agent_preset: None,
        });
        let result = tools
            .execute_with_session(
                &mut session,
                input("echo", json!({"text": "hi"}), AbortFlag::new()),
            )
            .await;
        assert!(!result.is_error());
        assert_eq!(
            result.content(),
            &[ContentBlock::Text {
                text: "echo: hi".into()
            }]
        );
    }

    #[tokio::test]
    async fn guard_denies_after_pre_allow() {
        let mut tools = ToolRuntime::new(ToolPresentationMode::Native);
        tools.register(echo_tool());
        tools.guard(Box::new(|exec| {
            if exec.name == "echo" {
                Some("guarded".into())
            } else {
                None
            }
        }));
        let result = tools
            .execute(input("echo", json!({"text": "hi"}), AbortFlag::new()))
            .await;
        assert_eq!(
            result.content(),
            &[ContentBlock::Text {
                text: "Error: guarded".into()
            }]
        );
    }

    #[tokio::test]
    async fn code_mode_collapse_skips_pre_execute() {
        let mut tools = ToolRuntime::new(ToolPresentationMode::Code);
        tools.register(echo_tool());
        let pre = Arc::new(AtomicUsize::new(0));
        let pre_count = pre.clone();
        tools.on_pre(move |_exec, next| {
            pre_count.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move { next().await })
        });
        let result = tools
            .execute(input("echo", json!({"text": "hi"}), AbortFlag::new()))
            .await;
        assert_eq!(pre.load(Ordering::SeqCst), 0);
        match result {
            ToolExecutionResult::Failure { error, .. } => {
                assert_eq!(
                    error.info.as_ref().map(|i| i.code.as_str()),
                    Some("UNKNOWN_TOOL")
                );
                assert!(
                    error
                        .message
                        .contains("only `run_code` is callable directly")
                );
                assert!(error.message.contains("echo"));
            }
            ToolExecutionResult::Success { .. } => panic!("collapsed call must fail"),
        }
    }

    #[tokio::test]
    async fn nested_parent_bypasses_code_collapse() {
        let mut tools = ToolRuntime::new(ToolPresentationMode::Code);
        tools.register(echo_tool());
        let mut input = input("echo", json!({"text": "hi"}), AbortFlag::new());
        input.parent = Some(crate::ToolExecutionToken(1));
        let result = tools.execute(input).await;
        assert!(!result.is_error());
    }

    #[tokio::test]
    async fn collapsed_aborted_call_is_aborted_before_dispatch_not_unknown_tool() {
        let mut tools = ToolRuntime::new(ToolPresentationMode::Code);
        tools.register(echo_tool());
        let signal = AbortFlag::new();
        signal.abort();
        let result = tools
            .execute(input("echo", json!({"text": "hi"}), signal))
            .await;
        match result {
            ToolExecutionResult::Failure { error, content, .. } => {
                assert_eq!(
                    error.info.as_ref().map(|i| i.code.as_str()),
                    Some(TOOL_ABORTED_BEFORE_DISPATCH)
                );
                assert_eq!(error.message, "tool call aborted before dispatch");
                assert_eq!(
                    content,
                    vec![ContentBlock::Text {
                        text: "Error: tool call aborted before dispatch".into()
                    }]
                );
            }
            ToolExecutionResult::Success { .. } => panic!("expected abort"),
        }
    }

    #[tokio::test]
    async fn abort_after_body_uses_aborted_code() {
        let mut tools = ToolRuntime::new(ToolPresentationMode::Native);
        let signal = AbortFlag::new();
        let abort = signal.clone();
        tools.register(ToolDefinition {
            name: "echo".into(),
            description: "echo".into(),
            parameters: json!({}),
            execute: Box::new(move |args, _| {
                abort.abort();
                Box::pin(async move { Ok(args) })
            }),
            render: Box::new(|_, _| vec![ContentBlock::Text { text: "ok".into() }]),
            is_concurrency_safe: None,
        });
        let result = tools
            .execute(input("echo", json!({"text": "hi"}), signal))
            .await;
        match result {
            ToolExecutionResult::Failure { error, .. } => {
                assert_eq!(
                    error.info.as_ref().map(|i| i.code.as_str()),
                    Some(TOOL_ABORTED)
                );
                assert_eq!(error.message, "tool call aborted");
            }
            ToolExecutionResult::Success { .. } => panic!("expected abort"),
        }
    }

    #[tokio::test]
    async fn unknown_tool_still_runs_pre_execute() {
        let mut tools = ToolRuntime::new(ToolPresentationMode::Native);
        let pre = Arc::new(AtomicUsize::new(0));
        let pre_count = pre.clone();
        tools.on_pre(move |_exec, next| {
            pre_count.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move { next().await })
        });
        let result = tools
            .execute(input("ghost", json!({}), AbortFlag::new()))
            .await;
        assert_eq!(pre.load(Ordering::SeqCst), 1);
        match result {
            ToolExecutionResult::Failure { error, .. } => {
                assert_eq!(error.message, "unknown tool \"ghost\"");
            }
            ToolExecutionResult::Success { .. } => panic!("expected unknown"),
        }
    }

    #[tokio::test]
    async fn prepare_deny_skips_dispatch_and_assigns_a_real_token() {
        let mut tools = ToolRuntime::new(ToolPresentationMode::Native);
        let ran = Arc::new(AtomicUsize::new(0));
        let ran_body = ran.clone();
        tools.register(ToolDefinition {
            name: "echo".into(),
            description: "echo".into(),
            parameters: json!({}),
            execute: Box::new(move |args, _| {
                ran_body.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move { Ok(args) })
            }),
            render: Box::new(|_, _| vec![ContentBlock::Text { text: "ok".into() }]),
            is_concurrency_safe: Some(Box::new(|_| true)),
        });
        let seen_token = Arc::new(AtomicUsize::new(0));
        let token_slot = seen_token.clone();
        tools.on_pre(move |exec, _next| {
            token_slot.store(exec.token.0 as usize, Ordering::SeqCst);
            Box::pin(async {
                PreToolDecision::Deny {
                    reason: "denied by policy".into(),
                }
            })
        });
        let prepared = tools
            .prepare(input("echo", json!({"text": "hi"}), AbortFlag::new()))
            .await;
        match prepared {
            crate::ScheduledToolPreparation::PostResult { exec, result } => {
                assert!(exec.token.0 >= 1);
                assert_eq!(seen_token.load(Ordering::SeqCst), exec.token.0 as usize);
                assert!(result.is_error());
            }
            other => panic!("expected post-result deny, got {other:?}"),
        }
        assert_eq!(ran.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn dispatch_futures_overlap_without_mut_runtime() {
        let mut tools = ToolRuntime::new(ToolPresentationMode::Native);
        let live = Arc::new(AtomicUsize::new(0));
        let max_live = Arc::new(AtomicUsize::new(0));
        let live_c = live.clone();
        let max_c = max_live.clone();
        tools.register(ToolDefinition {
            name: "p".into(),
            description: "p".into(),
            parameters: json!({}),
            execute: Box::new(move |args, _| {
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
            render: Box::new(|_, _| vec![ContentBlock::Text { text: "ok".into() }]),
            is_concurrency_safe: Some(Box::new(|_| true)),
        });
        let a = tools
            .prepare(input("p", json!({"id": 1}), AbortFlag::new()))
            .await;
        let b = tools
            .prepare(input("p", json!({"id": 2}), AbortFlag::new()))
            .await;
        let crate::ScheduledToolPreparation::Dispatch { exec: ea } = a else {
            panic!("expected dispatch");
        };
        let crate::ScheduledToolPreparation::Dispatch { exec: eb } = b else {
            panic!("expected dispatch");
        };
        let fa = tools.dispatch(&ea);
        let fb = tools.dispatch(&eb);
        let (ra, rb) = tokio::join!(fa, fb);
        assert!(!matches!(
            ra,
            crate::ScheduledToolDispatch::PostResult { result } if result.is_error()
        ));
        assert!(!matches!(
            rb,
            crate::ScheduledToolDispatch::PostResult { result } if result.is_error()
        ));
        assert!(max_live.load(Ordering::SeqCst) >= 2);
    }

    #[tokio::test]
    async fn concurrency_safe_true_is_parallel() {
        let mut tools = ToolRuntime::new(ToolPresentationMode::Native);
        tools.register(ToolDefinition {
            name: "p".into(),
            description: "p".into(),
            parameters: json!({}),
            execute: Box::new(|args, _| Box::pin(async move { Ok(args) })),
            render: Box::new(|_, _| vec![]),
            is_concurrency_safe: Some(Box::new(|_| true)),
        });
        let mode = tools.execution_mode(&input("p", json!({}), AbortFlag::new()));
        assert_eq!(mode, ToolExecutionMode::Parallel);
    }

    #[tokio::test]
    async fn coded_error_sets_info_name_and_code() {
        let mut tools = ToolRuntime::new(ToolPresentationMode::Native);
        tools.register(ToolDefinition {
            name: "boom".into(),
            description: "boom".into(),
            parameters: json!({}),
            execute: Box::new(|_, _| {
                Box::pin(async {
                    Err(ToolError::Coded {
                        message:
                            "edit requires reading \"a.txt\" first — read the file, then retry"
                                .into(),
                        name: "FsError".into(),
                        code: "FS_NOT_OBSERVED".into(),
                    })
                })
            }),
            render: Box::new(|_, _| vec![ContentBlock::Text { text: "ok".into() }]),
            is_concurrency_safe: None,
        });
        let result = tools
            .execute(input("boom", json!({}), AbortFlag::new()))
            .await;
        match result {
            ToolExecutionResult::Failure { error, content, .. } => {
                assert_eq!(error.info.as_ref().unwrap().name, "FsError");
                assert_eq!(error.info.as_ref().unwrap().code, "FS_NOT_OBSERVED");
                assert_eq!(
                    error.message,
                    "edit requires reading \"a.txt\" first — read the file, then retry"
                );
                assert_eq!(
                    content,
                    vec![ContentBlock::Text {
                        text: "Error: edit requires reading \"a.txt\" first — read the file, then retry"
                            .into()
                    }]
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn run_code_name_is_reserved_token() {
        let _ = ApprovalOutcome::AllowedOnce;
        let _ = PostToolDecision::Accept {
            content: None,
            value: None,
            additional_contexts: vec![],
        };
        let _ = ToolError::UnknownTool("ghost".into());
        assert_eq!(RUN_CODE_NAME, "run_code");
    }
}
