//! Model-facing UTF-8 `read` tool.

use dsh_fs::{FsErrorCode, FsInfoType, FsObservation};
use dsh_session::ContentBlock;
use dsh_tools::{ToolDefinition, ToolError};
use serde_json::{Value, json};

use crate::FsToolContext;
use crate::args::parse_file_path;
use crate::error::coded_fs_error;

/// Numbered-line render of a successful read value's `text` field.
fn format_numbered_text(text: &str) -> String {
    text.split('\n')
        .enumerate()
        .map(|(i, line)| format!("{:>6}|{line}", i + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Registerable `read` definition. Concurrently schedulable: observation races fail closed.
pub(crate) fn definition(ctx: FsToolContext) -> ToolDefinition {
    ToolDefinition {
        name: "read".into(),
        description: "Read a UTF-8 text file and return line-numbered content.".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Path to read, resolved by the filesystem backend."
                }
            },
            "required": ["file_path"]
        }),
        execute: Box::new(move |args, exec| {
            let ctx = ctx.clone();
            Box::pin(async move { execute(&ctx, args, exec.signal).await })
        }),
        render: Box::new(|_args, value| {
            let text = value.get("text").and_then(Value::as_str).unwrap_or("");
            vec![ContentBlock::Text {
                text: format_numbered_text(text),
            }]
        }),
        is_concurrency_safe: Some(Box::new(|_| true)),
    }
}

async fn execute(
    ctx: &FsToolContext,
    args: Value,
    signal: dsh_tools::AbortFlag,
) -> Result<Value, ToolError> {
    let file_path = parse_file_path(&args)?;
    let target = ctx
        .fs
        .resolve(&file_path, None, Some(&signal))
        .await
        .map_err(coded_fs_error)?;
    let info = match ctx.fs.stat(&target, Some(&signal)).await {
        Ok(Some(info)) => info,
        Ok(None) => {
            ctx.gate
                .observe(&target, FsObservation::Absent, Some(ctx.owner));
            return Err(ToolError::Coded {
                message: format!("cannot read \"{}\": not found", target.display_path),
                name: "FsError".into(),
                code: FsErrorCode::NotFound.as_str().into(),
            });
        }
        Err(err) => return Err(coded_fs_error(err)),
    };
    if info.kind != FsInfoType::File {
        return Err(ToolError::Coded {
            message: format!(
                "cannot read \"{}\": not a regular file",
                target.display_path
            ),
            name: "FsError".into(),
            code: FsErrorCode::NotRegularFile.as_str().into(),
        });
    }
    let text = ctx
        .fs
        .read_text(&target, Some(&signal))
        .await
        .map_err(coded_fs_error)?;
    ctx.gate.observe(
        &target,
        FsObservation::Present {
            version: info.version,
        },
        Some(ctx.owner),
    );
    Ok(json!({
        "path": target.display_path,
        "text": text,
    }))
}
