//! Model-facing full-file `write` tool.

use dsh_fs::FsObservation;
use dsh_session::ContentBlock;
use dsh_tools::{ToolDefinition, ToolError};
use serde_json::{Value, json};

use crate::FsToolContext;
use crate::args::{check_escalation, optional_string, parse_file_path};
use crate::error::coded_fs_error;

/// Confirmation envelope; the file body is not echoed.
fn format_write_output(display_path: &str, operation: &str) -> String {
    let verb = if operation == "create" {
        "Created"
    } else {
        "Updated"
    };
    format!("<path>{display_path}</path>\n<type>file</type>\n<content>\n{verb} file\n</content>")
}

/// Registerable `write` definition. Exclusive: `is_concurrency_safe` is [`None`].
pub(crate) fn definition(ctx: FsToolContext) -> ToolDefinition {
    ToolDefinition {
        name: "write".into(),
        description: "Create or fully replace a UTF-8 text file.".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Path to write, resolved by the filesystem backend."
                },
                "content": {
                    "type": "string",
                    "description": "Full UTF-8 text content to write."
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
            "required": ["file_path", "content"]
        }),
        execute: Box::new(move |args, exec| {
            let ctx = ctx.clone();
            Box::pin(async move { execute(&ctx, args, exec.signal).await })
        }),
        render: Box::new(|_args, value| {
            let path = value.get("path").and_then(Value::as_str).unwrap_or("");
            let operation = value
                .get("operation")
                .and_then(Value::as_str)
                .unwrap_or("update");
            vec![ContentBlock::Text {
                text: format_write_output(path, operation),
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
    let content = match args.get("content") {
        Some(Value::String(text)) => text.clone(),
        _ => return Err(ToolError::Other("content must be a string".into())),
    };
    let sandbox_permissions = optional_string(&args, "sandbox_permissions")?;
    let justification = optional_string(&args, "justification")?;
    check_escalation(ctx, sandbox_permissions, justification)?;
    let target = ctx
        .fs
        .resolve(&file_path, None, Some(&signal))
        .await
        .map_err(coded_fs_error)?;
    let intent = ctx.gate.write_intent(&target, Some(ctx.owner));
    let outcome = ctx
        .fs
        .write_text(
            &target,
            &content,
            Some(intent),
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
        "operation": outcome.operation,
        "before": outcome.before,
        "after": outcome.after,
    }))
}
