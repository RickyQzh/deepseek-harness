//! Tool registry types and argument freeze for the Rust host.

mod error;
mod freeze;
mod types;

pub use error::{TOOL_ABORTED, TOOL_ABORTED_BEFORE_DISPATCH, ToolError};
pub use freeze::{freeze_args, freeze_args_from_raw};
pub use types::{
    AbortFlag, PostToolDecision, PreToolDecision, RUN_CODE_NAME, ToolErrorInfo, ToolExecution,
    ToolExecutionInput, ToolExecutionResult, ToolExecutionToken, ToolFailure, ToolPresentationMode,
};
