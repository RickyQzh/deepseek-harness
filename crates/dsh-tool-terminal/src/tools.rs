//! Execute bodies for the six `terminal_*` tools.

use dsh_session::{ContentBlock, SessionId};
use dsh_terminal::{
    TerminalReadRequest, TerminalSendRequest, TerminalSessionId, TerminalSessionService,
    TerminalSessionSnapshot, TerminalSessionStatus, TerminalSignal, TerminalSpawnRequest,
    TerminalSpawnResult, TerminalWaitReason,
};
use dsh_tools::{ToolDefinition, ToolError, ToolExecution, ToolRuntime};
use serde_json::{Map, Value, json};

use crate::plugin::ToolTerminalConfig;
use crate::render::{
    list_from_json, read_from_json, render_list, render_read, render_send, render_spawn,
    send_from_json, spawn_from_json,
};

const MISSING_SESSION: &str = "terminal tools require an initiating session";
const JOBS_REQUIRED: &str =
    "background terminal sends require @deepseek-ai/dsh-jobs and @deepseek-ai/dsh-tool-jobs";
const BACKGROUND_DISABLED: &str =
    "background terminal sends are disabled by tool-terminal configuration";
const SEND_ABORTED: &str = "terminal send aborted";
const OPEN_DESCRIPTION: &str = "Create a persistent, owner-isolated terminal session from a registered backend type. Use this for shell or REPL state that must survive across tool calls.";
const SEND_DESCRIPTION_BASE: &str = "Send text to a persistent terminal. By default Enter is submitted and the call waits for a prompt, stdin wait, output silence, timeout, or session exit.";
const SEND_DESCRIPTION_BACKGROUND: &str =
    " Background mode returns a job id for job_output/job_kill.";
const READ_DESCRIPTION: &str =
    "Read a bounded page of retained output from a persistent terminal without sending input.";
const SIGNAL_DESCRIPTION: &str =
    "Send an allowed signal to the current foreground process group of a persistent terminal.";
const CLOSE_DESCRIPTION: &str =
    "Close one persistent terminal and wait until its captured owned process tree is gone.";
const LIST_DESCRIPTION: &str = "List persistent terminal sessions owned by the current agent.";

fn tool_err(error: impl ToString) -> ToolError {
    ToolError::Other(error.to_string())
}

fn require_owner(exec: &ToolExecution) -> Result<SessionId, ToolError> {
    exec.session_id
        .clone()
        .ok_or_else(|| tool_err(MISSING_SESSION))
}

fn session_id_from_args(args: &Value) -> Result<TerminalSessionId, ToolError> {
    let value = args.get("sessionId").and_then(Value::as_str).unwrap_or("");
    TerminalSessionId::from_arg(value).map_err(tool_err)
}

fn required_nonempty(args: &Value, key: &str, message: &str) -> Result<String, ToolError> {
    let value = args.get(key).and_then(Value::as_str).unwrap_or("");
    if value.is_empty() {
        return Err(tool_err(message));
    }
    Ok(value.to_string())
}

fn optional_string(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_string)
}

fn optional_usize(args: &Value, key: &str) -> Result<Option<usize>, ToolError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .map(Some)
            .ok_or_else(|| tool_err(format!("{key} must be a non-negative integer"))),
        Some(_) => Err(tool_err(format!("{key} must be a non-negative integer"))),
    }
}

fn parse_signal(args: &Value) -> Result<TerminalSignal, ToolError> {
    let name = args.get("signal").and_then(Value::as_str).unwrap_or("");
    match name {
        "SIGINT" => Ok(TerminalSignal::Sigint),
        "SIGTERM" => Ok(TerminalSignal::Sigterm),
        "SIGKILL" => Ok(TerminalSignal::Sigkill),
        "SIGTSTP" => Ok(TerminalSignal::Sigtstp),
        "SIGHUP" => Ok(TerminalSignal::Sighup),
        _ => Err(tool_err(format!("invalid terminal signal {name}"))),
    }
}

fn wait_reason_str(reason: TerminalWaitReason) -> &'static str {
    match reason {
        TerminalWaitReason::StdinRead => "stdin_read",
        TerminalWaitReason::InferredIdle => "inferred_idle",
        TerminalWaitReason::Timeout => "timeout",
        TerminalWaitReason::SessionExit => "session_exit",
    }
}

fn status_to_json(status: &TerminalSessionStatus) -> Value {
    match status {
        TerminalSessionStatus::Running => json!({ "kind": "running" }),
        TerminalSessionStatus::Exited { exit_code, signal } => json!({
            "kind": "exited",
            "exitCode": exit_code,
            "signal": signal,
        }),
    }
}

fn snapshot_fields(
    session_id: &str,
    name: Option<&str>,
    backend_type: &str,
    pid: Option<i32>,
    status: &TerminalSessionStatus,
) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert("sessionId".into(), json!(session_id));
    if let Some(name) = name {
        map.insert("name".into(), json!(name));
    }
    map.insert("type".into(), json!(backend_type));
    if let Some(pid) = pid {
        map.insert("pid".into(), json!(pid));
    }
    map.insert("status".into(), status_to_json(status));
    map
}

fn spawn_to_json(result: &TerminalSpawnResult) -> Value {
    let mut map = snapshot_fields(
        result.session_id().as_str(),
        result.name(),
        result.backend_type(),
        result.pid(),
        &result.status(),
    );
    map.insert("motd".into(), json!(result.motd()));
    Value::Object(map)
}

fn list_item_to_json(snapshot: &TerminalSessionSnapshot) -> Value {
    Value::Object(snapshot_fields(
        snapshot.session_id().as_str(),
        snapshot.name(),
        snapshot.backend_type(),
        snapshot.pid(),
        &snapshot.status(),
    ))
}

fn text_block(text: String) -> Vec<ContentBlock> {
    vec![ContentBlock::Text { text }]
}

fn send_parameters(enable_run_in_background: bool) -> Value {
    let mut properties = json!({
        "sessionId": {
            "type": "string",
            "description": "Terminal session id returned by terminal_open or terminal_list."
        },
        "text": {
            "type": "string",
            "description": "UTF-8 text to write to the terminal."
        },
        "submit": {
            "type": "boolean",
            "description": "Submit Enter after text (default true). Set false for control characters or incomplete REPL input."
        }
    });
    if enable_run_in_background {
        properties["run_in_background"] = json!({
            "type": "boolean",
            "description": "Return a job id immediately; collect with job_output or stop with job_kill."
        });
    }
    json!({
        "type": "object",
        "properties": properties,
        "required": ["sessionId", "text"]
    })
}

fn send_description(enable_run_in_background: bool) -> String {
    if enable_run_in_background {
        format!("{SEND_DESCRIPTION_BASE}{SEND_DESCRIPTION_BACKGROUND}")
    } else {
        SEND_DESCRIPTION_BASE.to_string()
    }
}

/// Register the six `terminal_*` tools. Clone `terminals` into each closure.
///
/// # Parameters
///
/// * `runtime` - Tool registry that receives the six definitions.
/// * `terminals` - Cloned session service captured by execute closures.
/// * `config` - Validated `enableRunInBackground` and `maxResultBytes`.
///
/// # Returns
///
/// Nothing. The six tools are registered on `runtime`.
pub(crate) fn register_terminal_tools(
    runtime: &mut ToolRuntime,
    terminals: TerminalSessionService,
    config: ToolTerminalConfig,
) {
    let max_result_bytes = config.max_result_bytes;
    let enable_run_in_background = config.enable_run_in_background;

    {
        let terminals = terminals.clone();
        runtime.register(ToolDefinition {
            name: "terminal_open".into(),
            description: OPEN_DESCRIPTION.into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "type": {
                        "type": "string",
                        "description": "Registered terminal backend type, usually \"shell\"."
                    },
                    "name": {
                        "type": "string",
                        "description": "Optional owner-local display name such as \"main\" or \"gdb\"."
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Initial working directory. Defaults to the deployment workspace root."
                    }
                },
                "required": ["type"]
            }),
            execute: Box::new(move |args, exec| {
                let terminals = terminals.clone();
                Box::pin(async move { execute_open(terminals, args, exec).await })
            }),
            render: Box::new(move |_args, value| {
                text_block(render_spawn(&spawn_from_json(value), max_result_bytes))
            }),
            is_concurrency_safe: None,
        });
    }

    {
        let terminals = terminals.clone();
        runtime.register(ToolDefinition {
            name: "terminal_send".into(),
            description: send_description(enable_run_in_background),
            parameters: send_parameters(enable_run_in_background),
            execute: Box::new(move |args, exec| {
                let terminals = terminals.clone();
                Box::pin(async move {
                    execute_send(terminals, args, exec, enable_run_in_background).await
                })
            }),
            render: Box::new(move |_args, value| {
                text_block(render_send(&send_from_json(value), max_result_bytes))
            }),
            is_concurrency_safe: None,
        });
    }

    {
        let terminals = terminals.clone();
        runtime.register(ToolDefinition {
            name: "terminal_read".into(),
            description: READ_DESCRIPTION.into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "sessionId": {
                        "type": "string",
                        "description": "Terminal session id."
                    },
                    "offset": {
                        "type": "number",
                        "description": "Newest-relative line offset (default 0)."
                    },
                    "count": {
                        "type": "number",
                        "description": "Requested line count (default 500; backend caps apply)."
                    }
                },
                "required": ["sessionId"]
            }),
            execute: Box::new(move |args, exec| {
                let terminals = terminals.clone();
                Box::pin(async move { execute_read(terminals, args, exec) })
            }),
            render: Box::new(move |_args, value| {
                text_block(render_read(&read_from_json(value), max_result_bytes))
            }),
            is_concurrency_safe: None,
        });
    }

    {
        let terminals = terminals.clone();
        runtime.register(ToolDefinition {
            name: "terminal_signal".into(),
            description: SIGNAL_DESCRIPTION.into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "sessionId": {
                        "type": "string",
                        "description": "Terminal session id."
                    },
                    "signal": {
                        "type": "string",
                        "enum": ["SIGINT", "SIGTERM", "SIGKILL", "SIGTSTP", "SIGHUP"],
                        "description": "Signal to deliver. Shell-targeted SIGKILL is rejected; use terminal_close."
                    }
                },
                "required": ["sessionId", "signal"]
            }),
            execute: Box::new(move |args, exec| {
                let terminals = terminals.clone();
                Box::pin(async move { execute_signal(terminals, args, exec).await })
            }),
            render: Box::new(|args, value| {
                let signal = args.get("signal").and_then(Value::as_str).unwrap_or("");
                let pgid = value
                    .get("targetPgid")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                text_block(format!(
                    "delivered {signal} to foreground process group {pgid}"
                ))
            }),
            is_concurrency_safe: None,
        });
    }

    {
        let terminals = terminals.clone();
        runtime.register(ToolDefinition {
            name: "terminal_close".into(),
            description: CLOSE_DESCRIPTION.into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "sessionId": {
                        "type": "string",
                        "description": "Terminal session id."
                    }
                },
                "required": ["sessionId"]
            }),
            execute: Box::new(move |args, exec| {
                let terminals = terminals.clone();
                Box::pin(async move { execute_close(terminals, args, exec).await })
            }),
            render: Box::new(|_args, value| {
                let session_id = value.get("sessionId").and_then(Value::as_str).unwrap_or("");
                let outcome = value.get("outcome").and_then(Value::as_str).unwrap_or("");
                let text = if outcome == "closed" {
                    format!("closed terminal session {session_id}")
                } else {
                    format!("terminal session {session_id} was already closing")
                };
                text_block(text)
            }),
            is_concurrency_safe: None,
        });
    }

    runtime.register(ToolDefinition {
        name: "terminal_list".into(),
        description: LIST_DESCRIPTION.into(),
        parameters: json!({
            "type": "object",
            "properties": {}
        }),
        execute: Box::new(move |args, exec| {
            let terminals = terminals.clone();
            Box::pin(async move { execute_list(terminals, args, exec) })
        }),
        render: Box::new(move |_args, value| {
            text_block(render_list(&list_from_json(value), max_result_bytes))
        }),
        is_concurrency_safe: None,
    });
}

async fn execute_open(
    terminals: TerminalSessionService,
    args: Value,
    exec: ToolExecution,
) -> Result<Value, ToolError> {
    let owner = require_owner(&exec)?;
    let backend_type = required_nonempty(&args, "type", "type must be a non-empty string")?;
    let mut request = TerminalSpawnRequest::new(backend_type);
    if let Some(name) = optional_string(&args, "name") {
        request = request.with_name(name);
    }
    if let Some(cwd) = optional_string(&args, "cwd") {
        request = request.with_cwd(cwd);
    }
    let result = terminals.spawn(owner, request).await.map_err(tool_err)?;
    Ok(spawn_to_json(&result))
}

async fn execute_send(
    terminals: TerminalSessionService,
    args: Value,
    exec: ToolExecution,
    enable_run_in_background: bool,
) -> Result<Value, ToolError> {
    let owner = require_owner(&exec)?;
    let id = session_id_from_args(&args)?;
    let text = match args.get("text") {
        Some(Value::String(text)) => text.clone(),
        _ => return Err(tool_err("text must be a string")),
    };
    let submit = args.get("submit").and_then(Value::as_bool).unwrap_or(true);
    if args.get("run_in_background") == Some(&Value::Bool(true)) {
        if !enable_run_in_background {
            return Err(tool_err(BACKGROUND_DISABLED));
        }
        return Err(tool_err(JOBS_REQUIRED));
    }
    let mut operation = terminals
        .start_send(&owner, &id, TerminalSendRequest::new(text, submit))
        .map_err(tool_err)?;
    let result = tokio::select! {
        result = operation.done() => result,
        () = exec.signal.cancelled() => {
            let _ = operation.cancel();
            operation.done().await
        }
    };
    if exec.signal.is_aborted() {
        return Err(tool_err(SEND_ABORTED));
    }
    Ok(json!({
        "kind": "foreground",
        "viewport": result.viewport(),
        "waitReason": wait_reason_str(result.wait_reason()),
        "sessionStatus": status_to_json(&result.session_status()),
        "truncated": result.truncated(),
    }))
}

fn execute_read(
    terminals: TerminalSessionService,
    args: Value,
    exec: ToolExecution,
) -> Result<Value, ToolError> {
    let owner = require_owner(&exec)?;
    let id = session_id_from_args(&args)?;
    let mut request = TerminalReadRequest::new();
    if let Some(offset) = optional_usize(&args, "offset")? {
        request = request.with_offset(offset);
    }
    if let Some(count) = optional_usize(&args, "count")? {
        request = request.with_count(count);
    }
    let result = terminals.read(&owner, &id, request).map_err(tool_err)?;
    Ok(json!({
        "text": result.text(),
        "totalLines": result.total_lines(),
        "lineBegin": result.line_begin(),
        "lineEnd": result.line_end(),
        "truncated": result.truncated(),
    }))
}

async fn execute_signal(
    terminals: TerminalSessionService,
    args: Value,
    exec: ToolExecution,
) -> Result<Value, ToolError> {
    let owner = require_owner(&exec)?;
    let id = session_id_from_args(&args)?;
    let signal = parse_signal(&args)?;
    let result = terminals
        .signal(&owner, &id, signal)
        .await
        .map_err(tool_err)?;
    Ok(json!({
        "delivered": result.delivered(),
        "targetPgid": result.target_pgid(),
    }))
}

async fn execute_close(
    terminals: TerminalSessionService,
    args: Value,
    exec: ToolExecution,
) -> Result<Value, ToolError> {
    let owner = require_owner(&exec)?;
    let id = session_id_from_args(&args)?;
    let closed = terminals
        .kill(&owner, &id, "model request")
        .await
        .map_err(tool_err)?;
    let outcome = if closed { "closed" } else { "already-closing" };
    Ok(json!({
        "sessionId": id.as_str(),
        "outcome": outcome,
    }))
}

fn execute_list(
    terminals: TerminalSessionService,
    _args: Value,
    exec: ToolExecution,
) -> Result<Value, ToolError> {
    let owner = require_owner(&exec)?;
    let sessions = terminals.list(&owner);
    Ok(Value::Array(
        sessions.iter().map(list_item_to_json).collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::{send_description, send_parameters};

    #[test]
    fn send_schema_omits_background_when_disabled() {
        let enabled = send_parameters(true);
        assert!(enabled["properties"].get("run_in_background").is_some());
        assert!(
            send_description(true)
                .contains("Background mode returns a job id for job_output/job_kill.")
        );
        let disabled = send_parameters(false);
        assert!(disabled["properties"].get("run_in_background").is_none());
        assert_eq!(
            send_description(false),
            "Send text to a persistent terminal. By default Enter is submitted and the call waits for a prompt, stdin wait, output silence, timeout, or session exit."
        );
    }
}
