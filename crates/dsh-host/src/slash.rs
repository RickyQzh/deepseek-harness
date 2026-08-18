//! Slash Typert remotes: `commands/list` and `commands/execute` only.

use dsh_commands::{CommandRegistry, CommandResult};
use dsh_rpc::{RpcError, RpcErrorCode, RpcResult};
use serde_json::{Value, json};

/// Handle an installed slash remote. Unknown slash names return [`None`] (HTTP 404).
pub(crate) async fn handle_slash(
    commands: Option<&CommandRegistry>,
    method: &str,
    payload: &Value,
) -> Option<RpcResult> {
    match method {
        "commands/list" => Some(list_commands(commands)),
        "commands/execute" => Some(execute_command(commands, payload).await),
        _ => None,
    }
}

fn args_of(payload: &Value) -> &Value {
    match payload.get("args") {
        Some(args) => args,
        None => &Value::Null,
    }
}

fn list_commands(commands: Option<&CommandRegistry>) -> RpcResult {
    let descriptors = match commands {
        Some(registry) => registry.list(),
        None => Vec::new(),
    };
    let commands: Vec<Value> = descriptors
        .into_iter()
        .map(|descriptor| {
            json!({
                "name": descriptor.name(),
                "description": descriptor.description(),
            })
        })
        .collect();
    RpcResult::ok(json!({ "commands": commands }))
}

async fn execute_command(commands: Option<&CommandRegistry>, payload: &Value) -> RpcResult {
    let Some(line) = args_of(payload).get("line").and_then(Value::as_str) else {
        return unknown_command();
    };
    let Some(registry) = commands else {
        return unknown_command();
    };
    match registry.execute(line).await {
        None => unknown_command(),
        Some(execution) => {
            RpcResult::ok(json!({ "result": command_result_json(execution.result()) }))
        }
    }
}

fn command_result_json(result: &CommandResult) -> Value {
    match result {
        CommandResult::Success { text } => {
            let mut value = json!({ "kind": "success" });
            if let Some(text) = text {
                value["text"] = json!(text);
            }
            value
        }
        CommandResult::Error { text } => json!({ "kind": "error", "text": text }),
    }
}

fn unknown_command() -> RpcResult {
    RpcResult::err(RpcError::with_code(
        RpcErrorCode::UnknownCommand,
        "unknown-command",
        json!({}),
    ))
}
