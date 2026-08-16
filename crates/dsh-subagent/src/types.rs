//! Request, result, capability, and lifecycle types for one-shot subagent runs.

use std::future::Future;
use std::pin::Pin;

use dsh_agent::AgentHandle;
use dsh_brand::Branded;
use dsh_session::{ContentBlock, LogEvent, SessionId};
use dsh_tools::AbortFlag;

/// Kernel event name emitted before the child turn.
pub const EVENT_SUBAGENT_START: &str = "subagent/start";
/// Kernel event name emitted after the child turn.
pub const EVENT_SUBAGENT_END: &str = "subagent/end";

/// Tag for [`SubagentRunId`].
pub struct SubagentRunIdTag;

/// Identifies one accepted subagent run across its lifecycle event pair.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct SubagentRunId(Branded<SubagentRunIdTag>);

impl SubagentRunId {
    /// Brand `value` as a run id.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(Branded::new(value))
    }

    /// Borrow the inner string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// Start-time features a provider supports. [`SubagentRuntime::start`] rejects a
/// request that needs a flag the named provider does not advertise.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SubagentCapabilities {
    /// Structured final result.
    pub output_schema: bool,
    /// Absolute delegation-depth cap (`max_depth`).
    pub depth_limit: bool,
    /// Child tool restriction.
    pub tool_filter: bool,
    /// Per-child persona.
    pub persona: bool,
}

impl SubagentCapabilities {
    /// Every start-time flag enabled (in-process spawn and fork).
    #[must_use]
    pub const fn all() -> Self {
        Self {
            output_schema: true,
            depth_limit: true,
            tool_filter: true,
            persona: true,
        }
    }
}

/// Caller request for one one-shot subagent run.
#[derive(Clone, Debug)]
pub struct SubagentStartRequest {
    /// Optional short display label persisted on the child descriptor.
    pub label: Option<String>,
    /// Content delivered as the child's user message.
    pub prompt: Vec<ContentBlock>,
    /// Delegating parent session id (lineage); the live handle is passed to `start`.
    pub parent_id: SessionId,
    /// Cancellation flag from the spawning context.
    pub signal: AbortFlag,
    /// Optional absolute cap for the child's delegation depth. Requires [`SubagentCapabilities::depth_limit`].
    pub max_depth: Option<u32>,
}

/// Why a one-shot subagent run ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubagentStopReason {
    /// The child finished its turn normally.
    Completed,
    /// Cancelled through the request signal or disposal.
    Aborted,
    /// Model, transport, or interrupted-turn failure.
    Error,
    /// The child hit its token ceiling before finishing.
    MaxTokens,
    /// The child declined the task (`TurnEndReason::Blocked`).
    Refusal,
}

/// Terminal outcome of a one-shot subagent run.
#[derive(Clone, Debug, PartialEq)]
pub struct SubagentResult {
    /// Why the run ended.
    pub stop_reason: SubagentStopReason,
    /// Assistant text blocks from the child's own suffix (after any fork seed).
    pub output: Vec<ContentBlock>,
}

/// Observe-only identifying detail for `subagent/start`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubagentRunInfo {
    /// Unique identity shared with the paired terminal event.
    pub run_id: SubagentRunId,
    /// Provider name recorded when the child was created.
    pub provider: String,
    /// The child agent's session id.
    pub id: SessionId,
}

/// Observe-only outcome detail for `subagent/end`.
#[derive(Clone, Debug, PartialEq)]
pub struct SubagentRunEndInfo {
    /// Unique identity shared with the paired start event.
    pub run_id: SubagentRunId,
    /// The same provider name carried by the paired start event.
    pub provider: String,
    /// The child agent's session id.
    pub id: SessionId,
    /// The terminal stop reason.
    pub stop_reason: SubagentStopReason,
}

/// Detached creation inputs for a continuable child. One-shot `start` must not call [`SubagentProvider::prepare_continuable`].
#[derive(Clone, Debug, Default)]
pub struct ContinuableCreateSpec {
    /// Completed-turn prefix of the parent log, or `None` for a fresh child.
    pub seed: Option<Vec<LogEvent>>,
}

/// Typed failure for the subagent registry and one-shot start path.
#[derive(Debug, thiserror::Error)]
pub enum SubagentError {
    /// No provider is registered under this name.
    #[error("unknown subagent provider `{0}`")]
    UnknownProvider(String),
    /// A provider with this name is already registered.
    #[error("a subagent provider named \"{0}\" is already registered")]
    DuplicateProvider(String),
    /// The request needs a start-time capability the provider does not advertise.
    #[error("subagent provider `{0}` does not support {1}")]
    UnsupportedCapability(String, String),
    /// Starting a child would exceed the requested or default depth cap.
    #[error("subagent depth {0} exceeds maxDepth {1}")]
    Depth(u64, u32),
    /// Provider, session, or agent failure.
    #[error("{0}")]
    Other(String),
}

impl SubagentError {
    /// Wrap an infrastructure failure that is not a stop reason.
    #[must_use]
    pub fn other(message: impl Into<String>) -> Self {
        Self::Other(message.into())
    }
}

/// One registered transport for running child agents.
pub trait SubagentProvider: Send + Sync {
    /// Unique registry name (for example `spawn` or `fork`).
    fn name(&self) -> &str;
    /// Start-time features this provider supports.
    fn capabilities(&self) -> SubagentCapabilities;
    /// Whether the child sees the parent's completed-turn prefix.
    fn inherits_parent_context(&self) -> bool;
    /// Establish a one-shot child and drive it to a terminal [`SubagentResult`].
    fn start(
        &self,
        request: SubagentStartRequest,
        parent: AgentHandle,
    ) -> Pin<Box<dyn Future<Output = Result<SubagentResult, SubagentError>> + Send + '_>>;
    /// Data stub for continuable creation. One-shot [`Self::start`] must not call this.
    fn prepare_continuable(&self, parent: &AgentHandle) -> ContinuableCreateSpec;
}
