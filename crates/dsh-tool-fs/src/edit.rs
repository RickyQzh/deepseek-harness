//! Model-facing literal `edit` tool.

use dsh_fs::{FsEditRequest, FsObservation};
use dsh_session::ContentBlock;
use dsh_tools::{ToolDefinition, ToolError};
use serde_json::{Value, json};

use crate::FsToolContext;
use crate::args::{check_escalation, optional_string, parse_file_path};
use crate::error::coded_fs_error;

/// Confirmation sentence; the file body is not echoed.
fn format_edit_output(display_path: &str, replace_all: bool) -> String {
    if replace_all {
        format!(
            "The file {display_path} has been updated. All occurrences were successfully replaced."
        )
    } else {
        format!("The file {display_path} has been updated successfully.")
    }
}

/// Registerable `edit` definition. Exclusive: `is_concurrency_safe` is [`None`].
pub(crate) fn definition(ctx: FsToolContext) -> ToolDefinition {
    ToolDefinition {
        name: "edit".into(),
        description: "Edit an existing UTF-8 text file by replacing literal text.".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Path to edit, resolved by the filesystem backend."
                },
                "old_string": {
                    "type": "string",
                    "description": "Literal text to replace. Must match exactly."
                },
                "new_string": {
                    "type": "string",
                    "description": "Literal replacement text. Use an empty string to delete the match."
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace all matches. Defaults to false; when false, old_string must appear exactly once."
                },
                "sandbox_permissions": {
                    "type": "string",
                    "description": "Wider sandbox mode for a one-shot retry of a denied mutation."
                },
                "justification": {
                    "type": "string",
                    "description": "Required with sandbox_permissions: one sentence for the user."
                }
            },
            "required": ["file_path", "old_string", "new_string"]
        }),
        execute: Box::new(move |args, exec| {
            let ctx = ctx.clone();
            Box::pin(async move { execute(&ctx, args, exec.signal).await })
        }),
        render: Box::new(|args, value| {
            let path = value.get("path").and_then(Value::as_str).unwrap_or("");
            let replace_all = args
                .get("replace_all")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            vec![ContentBlock::Text {
                text: format_edit_output(path, replace_all),
            }]
        }),
        is_concurrency_safe: None,
    }
}

async fn execute(
    ctx: &FsToolContext,
    args: Value,
    signal: dsh_tools::AbortFlag,
) -> Result<Value, ToolError> {
    let file_path = parse_file_path(&args)?;
    let old_string = match args.get("old_string") {
        Some(Value::String(text)) if !text.is_empty() => text.clone(),
        _ => {
            return Err(ToolError::Other(
                "old_string must be a non-empty string".into(),
            ));
        }
    };
    let new_string = match args.get("new_string") {
        Some(Value::String(text)) => text.clone(),
        _ => return Err(ToolError::Other("new_string must be a string".into())),
    };
    if old_string == new_string {
        return Err(ToolError::Other(
            "old_string and new_string must differ".into(),
        ));
    }
    let replace_all = match args.get("replace_all") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(_) => return Err(ToolError::Other("replace_all must be a boolean".into())),
    };
    let sandbox_permissions = optional_string(&args, "sandbox_permissions")?;
    let justification = optional_string(&args, "justification")?;
    check_escalation(ctx, sandbox_permissions, justification)?;
    let target = ctx
        .fs
        .resolve(&file_path, None, Some(&signal))
        .await
        .map_err(coded_fs_error)?;
    let version = ctx
        .gate
        .edit_intent(&target, Some(ctx.owner))
        .map_err(coded_fs_error)?;
    let outcome = ctx
        .fs
        .edit_text(
            &target,
            &FsEditRequest {
                old_string,
                new_string,
                replace_all,
            },
            Some(&version),
            Some(&signal),
            ctx.sandbox.as_ref(),
        )
        .await
        .map_err(coded_fs_error)?;
    ctx.gate.observe(
        &target,
        FsObservation::Present {
            version: outcome.version,
        },
        Some(ctx.owner),
    );
    Ok(json!({
        "path": target.display_path,
        "before": outcome.before,
        "after": outcome.after,
    }))
}
