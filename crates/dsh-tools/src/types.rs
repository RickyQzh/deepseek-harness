//! Tool execution types shared by the pipeline.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use dsh_session::{CallId, ContentBlock, Message, SessionId};
use tokio::sync::Notify;

/// Tool name reserved for Code Mode `run_code`.
pub const RUN_CODE_NAME: &str = "run_code";

/// Cooperative cancellation signal shared across a tool execution.
#[derive(Clone, Debug)]
pub struct AbortFlag {
    aborted: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl AbortFlag {
    /// Create a flag that is not aborted.
    #[must_use]
    pub fn new() -> Self {
        Self {
            aborted: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(Notify::new()),
        }
    }

    /// Mark the flag aborted and wake every waiter.
    pub fn abort(&self) {
        self.aborted.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }

    /// Whether [`abort`](Self::abort) has been called.
    #[must_use]
    pub fn is_aborted(&self) -> bool {
        self.aborted.load(Ordering::Acquire)
    }

    /// Wait until [`abort`](Self::abort) has been called.
    ///
    /// Returns immediately when the flag is already aborted.
    pub async fn cancelled(&self) {
        loop {
            let notified = self.notify.notified();
            if self.is_aborted() {
                return;
            }
            notified.await;
        }
    }
}

impl Default for AbortFlag {
    fn default() -> Self {
        Self::new()
    }
}

/// Opaque token identifying one in-flight tool execution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolExecutionToken(
    /// Monotonic execution token.
    pub u64,
);

/// How a tool is presented to the model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolPresentationMode {
    /// Native tool-calling only.
    Native,
    /// Code Mode only.
    Code,
    /// Native tool-calling and Code Mode.
    Both,
}

/// Decision from a pre-execute policy listener.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreToolDecision {
    /// Continue execution.
    Allow,
    /// Refuse the call with this reason.
    Deny {
        /// Reason shown to the model.
        reason: String,
    },
    /// Ask the user before continuing.
    Ask {
        /// Optional reason shown while asking.
        reason: Option<String>,
    },
}

/// Decision from a post-execute policy listener.
#[derive(Clone, Debug, PartialEq)]
pub enum PostToolDecision {
    /// Accept the tool outcome, optionally replacing content or value.
    Accept {
        /// Replacement content blocks.
        content: Option<Vec<ContentBlock>>,
        /// Replacement JSON value.
        value: Option<serde_json::Value>,
        /// Extra model-visible messages appended after the result.
        additional_contexts: Vec<Message>,
    },
    /// Block the outcome and return this feedback instead.
    Block {
        /// Feedback content shown to the model.
        feedback: Vec<ContentBlock>,
        /// Extra model-visible messages appended after the block.
        additional_contexts: Vec<Message>,
    },
}

/// Structured tool-error identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolErrorInfo {
    /// Error name, such as `AbortError`.
    pub name: String,
    /// Stable error code, such as [`crate::TOOL_ABORTED`].
    pub code: String,
}

/// Failure payload on [`ToolExecutionResult::Failure`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolFailure {
    /// Human-readable failure text.
    pub message: String,
    /// Optional structured identity.
    pub info: Option<ToolErrorInfo>,
}

/// Materialized outcome of one tool body.
#[derive(Clone, Debug, PartialEq)]
pub enum ToolExecutionResult {
    /// The tool completed without reporting failure.
    Success {
        /// JSON value returned to the model.
        value: serde_json::Value,
        /// Content blocks returned to the model.
        content: Vec<ContentBlock>,
        /// Optional metadata that is not model-visible by default.
        meta: Option<serde_json::Value>,
        /// Extra model-visible messages appended after the result.
        additional_contexts: Vec<Message>,
        /// Whether this result ends the current turn.
        concludes_turn: bool,
    },
    /// The tool reported failure.
    Failure {
        /// Failure text and optional structured identity.
        error: ToolFailure,
        /// Content blocks returned to the model.
        content: Vec<ContentBlock>,
        /// Optional metadata that is not model-visible by default.
        meta: Option<serde_json::Value>,
        /// Extra model-visible messages appended after the result.
        additional_contexts: Vec<Message>,
    },
}

impl ToolExecutionResult {
    /// Whether this result is a tool-reported failure.
    #[must_use]
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Failure { .. })
    }

    /// Content blocks attached to the result.
    #[must_use]
    pub fn content(&self) -> &[ContentBlock] {
        match self {
            Self::Success { content, .. } | Self::Failure { content, .. } => content,
        }
    }
}

/// Request handed to the tool pipeline before a token is assigned.
#[derive(Clone, Debug)]
pub struct ToolExecutionInput {
    /// Provider-issued call id.
    pub call_id: CallId,
    /// Root call id when this execution is nested.
    pub root_call_id: Option<CallId>,
    /// Tool name.
    pub name: String,
    /// Frozen arguments.
    pub arguments: serde_json::Value,
    /// Parent execution when this call is nested.
    pub parent: Option<ToolExecutionToken>,
    /// Calling session for owner-fenced tools. `None` when the caller has no session.
    pub session_id: Option<SessionId>,
    /// Cancellation signal for this execution.
    pub signal: AbortFlag,
}

/// In-flight tool execution after a token is assigned.
#[derive(Clone, Debug)]
pub struct ToolExecution {
    /// Token for this execution.
    pub token: ToolExecutionToken,
    /// Provider-issued call id.
    pub call_id: CallId,
    /// Root call id for the outermost execution.
    pub root_call_id: CallId,
    /// Tool name.
    pub name: String,
    /// Frozen arguments.
    pub arguments: serde_json::Value,
    /// Parent execution when this call is nested.
    pub parent: Option<ToolExecutionToken>,
    /// Calling session copied from [`ToolExecutionInput::session_id`].
    pub session_id: Option<SessionId>,
    /// Cancellation signal for this execution.
    pub signal: AbortFlag,
}

#[cfg(test)]
mod tests {
    use super::AbortFlag;

    #[tokio::test]
    async fn abort_flag_notifies_waiters() {
        let flag = AbortFlag::new();
        assert!(!flag.is_aborted());
        let waiter = flag.clone();
        let task = tokio::spawn(async move {
            waiter.cancelled().await;
        });
        flag.abort();
        task.await.expect("join");
        assert!(flag.is_aborted());
    }
}
