//! Tool registry, execution pipeline, and argument freeze for the Rust host.

mod error;
mod freeze;
mod pipeline;
pub mod plugin;
mod types;

pub use error::{TOOL_ABORTED, TOOL_ABORTED_BEFORE_DISPATCH, ToolError};
pub use freeze::{freeze_args, freeze_args_from_raw};
pub use pipeline::{
    ApprovalOutcome, Approver, PrepareSnapshot, ScheduledToolDispatch, ScheduledToolPreparation,
    ToolDefinition, ToolExecutionMode, ToolRuntime,
};
pub use types::{
    AbortFlag, PostToolDecision, PreToolDecision, RUN_CODE_NAME, ToolErrorInfo, ToolExecution,
    ToolExecutionInput, ToolExecutionResult, ToolExecutionToken, ToolFailure, ToolPresentationMode,
};
