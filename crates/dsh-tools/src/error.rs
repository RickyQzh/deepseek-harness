//! Tool error codes and the pipeline error type.

/// Cancellation after the tool body was invoked.
pub const TOOL_ABORTED: &str = "ABORTED";
/// Cancellation before the tool body was invoked.
pub const TOOL_ABORTED_BEFORE_DISPATCH: &str = "ABORTED_BEFORE_DISPATCH";

/// Pipeline failure that is not a materialized `ToolExecutionResult`.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    /// Arguments were not lossless JSON.
    #[error("tool execution arguments must be losslessly JSON-serializable")]
    ArgsNotJson,
    /// No visible tool with this name.
    #[error("unknown tool \"{0}\"")]
    UnknownTool(String),
    /// Visible name denied by Code Mode collapse, with the route the model must take.
    #[error("unknown tool \"{name}\": {hint}")]
    UnknownToolHint {
        /// Denied tool name.
        name: String,
        /// How to reach the tool instead.
        hint: String,
    },
    /// Other pipeline failure.
    #[error("{0}")]
    Other(String),
    /// Structured failure whose `name` and `code` ride on [`crate::ToolErrorInfo`].
    #[error("{message}")]
    Coded {
        /// Model-visible failure text.
        message: String,
        /// Structured error name, such as `FsError`.
        name: String,
        /// Stable routing code, such as `FS_NOT_OBSERVED`.
        code: String,
    },
}

#[cfg(test)]
mod tests {
    use super::{TOOL_ABORTED, ToolError};

    #[test]
    fn coded_displays_message() {
        let err = ToolError::Coded {
            message: "edit requires reading \"a.txt\" first — read the file, then retry".into(),
            name: "FsError".into(),
            code: "FS_NOT_OBSERVED".into(),
        };
        assert!(err.to_string().contains("read the file"));
        let _ = TOOL_ABORTED;
    }
}
