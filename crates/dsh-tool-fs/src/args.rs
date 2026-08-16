//! Shared argument parsing for filesystem tools.

use dsh_sandbox::validate_escalation_args;
use dsh_tools::ToolError;
use serde_json::Value;

use crate::FsToolContext;

const UNCONFINED_ESCALATION: &str = "sandbox_permissions is not available in this composition (no sandboxing filesystem to escalate)";

/// Require `file_path` as a string whose trim is non-empty. Returns the original string.
pub(crate) fn parse_file_path(args: &Value) -> Result<String, ToolError> {
    let Some(Value::String(path)) = args.get("file_path") else {
        return Err(ToolError::Other(
            "file_path must be a non-empty string".into(),
        ));
    };
    if path.trim().is_empty() {
        return Err(ToolError::Other(
            "file_path must be a non-empty string".into(),
        ));
    }
    Ok(path.clone())
}

/// Borrow an optional string argument. Absent or JSON null is [`None`].
pub(crate) fn optional_string<'a>(
    args: &'a Value,
    key: &str,
) -> Result<Option<&'a str>, ToolError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.as_str())),
        Some(_) => Err(ToolError::Other(format!("{key} must be a string"))),
    }
}

/// Refuse escalation fields when this composition has no confining filesystem.
///
/// Otherwise require `sandbox_permissions` and `justification` to travel together.
pub(crate) fn check_escalation(
    ctx: &FsToolContext,
    sandbox_permissions: Option<&str>,
    justification: Option<&str>,
) -> Result<(), ToolError> {
    if (sandbox_permissions.is_some() || justification.is_some())
        && (ctx.sandbox.is_none() || ctx.fs.sandbox_mode().is_none())
    {
        return Err(ToolError::Coded {
            message: UNCONFINED_ESCALATION.into(),
            name: "Invalid".into(),
            code: "Invalid".into(),
        });
    }
    validate_escalation_args(sandbox_permissions, justification)
        .map_err(|err| ToolError::Other(err.to_string()))
}
