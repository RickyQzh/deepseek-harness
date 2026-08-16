//! Model-facing `bash` tool for the DeepSeek Harness Rust host.

mod render;

use std::sync::Arc;

use dsh_sandbox::{ESCALATION_TARGETS, SANDBOX_UNAVAILABLE, SandboxMode};
use dsh_session::ContentBlock;
use dsh_shell::{
    LocalBashExecutor, SandboxBashExecutor, ShellError, ShellExecRequest, ShellRunResult,
};
use dsh_tools::{TOOL_ABORTED, ToolDefinition, ToolError, ToolExecution, ToolRuntime};
use serde_json::{Value, json};

pub use render::render_result;

const BACKGROUND_UNAVAILABLE: &str =
    "Background execution is not available; long-running commands must finish within the timeout.";
const SANDBOX_PERMISSIONS_UNAVAILABLE: &str =
    "sandbox_permissions is not available in this composition (no sandboxing executor to escalate)";
const BASH_DESCRIPTION: &str = "Execute a bash command (`bash -c`) and return its stdout/stderr. Each call runs in a fresh shell: no state (cwd, variables, functions) persists between calls — pass `workdir` instead of using `cd`. Non-zero exits are reported as `[exit code: N]`. Commands may run under a file sandbox; a blocked file operation is reported as `[sandbox: file access denied under <mode> mode]`. Background execution is not available; long-running commands must finish within the timeout.";

/// Executor used by one registered `bash` tool.
#[derive(Clone)]
enum BashBackend {
    Local(Arc<LocalBashExecutor>),
    Sandbox(Arc<SandboxBashExecutor>),
}

/// Register the model-facing `bash` tool on `runtime`.
///
/// When `sandbox_shell` is [`Some`], execute uses that executor; otherwise it
/// uses `shell`. Calls are foreground only: `run_in_background: true` is
/// refused and `start` is never called. Nonzero command exits are successful
/// results whose rendered text includes `[exit code: N]`. `sandbox_permissions`
/// and `justification` are refused even when a sandbox executor is mounted;
/// this phase has no approval channel.
///
/// # Parameters
///
/// * `runtime` — registry that receives the `bash` definition.
/// * `shell` — unfenced executor used when `sandbox_shell` is [`None`].
/// * `sandbox_shell` — confined executor used when present.
pub fn register_bash_tool(
    runtime: &mut ToolRuntime,
    shell: Arc<LocalBashExecutor>,
    sandbox_shell: Option<Arc<SandboxBashExecutor>>,
) {
    let escalation_modes: Vec<SandboxMode> = if sandbox_shell.is_some() {
        ESCALATION_TARGETS.to_vec()
    } else {
        Vec::new()
    };
    let backend = match sandbox_shell {
        Some(sandbox) => BashBackend::Sandbox(sandbox),
        None => BashBackend::Local(shell),
    };
    runtime.register(ToolDefinition {
        name: "bash".into(),
        description: BASH_DESCRIPTION.into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The bash command to execute."
                },
                "description": {
                    "type": "string",
                    "description": "Clear, concise description of what this command does in active voice, 5-10 words (shown in the UI)."
                },
                "timeout_ms": {
                    "type": "number",
                    "description": "Timeout in milliseconds. The executor applies its configured default and cap, and kills the command on expiry."
                },
                "workdir": {
                    "type": "string",
                    "description": "Working directory for this command. Defaults to the executor cwd when omitted."
                }
            },
            "required": ["command", "description"]
        }),
        execute: Box::new(move |args, exec| {
            let backend = backend.clone();
            let escalation_modes = escalation_modes.clone();
            Box::pin(async move { execute(&backend, &escalation_modes, args, exec).await })
        }),
        render: Box::new(|_args, value| {
            let text = value
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            vec![ContentBlock::Text { text }]
        }),
        is_concurrency_safe: None,
    });
}

async fn execute(
    backend: &BashBackend,
    escalation_modes: &[SandboxMode],
    args: Value,
    exec: ToolExecution,
) -> Result<Value, ToolError> {
    let command = parse_required_string(&args, "command")?;
    let _description = parse_required_string(&args, "description")?;
    if args.get("run_in_background") == Some(&Value::Bool(true)) {
        return Err(ToolError::Other(BACKGROUND_UNAVAILABLE.into()));
    }
    if args.get("sandbox_permissions").is_some() || args.get("justification").is_some() {
        return Err(ToolError::Other(SANDBOX_PERMISSIONS_UNAVAILABLE.into()));
    }
    let timeout_ms = parse_timeout_ms(&args)?;
    let workdir = parse_workdir(&args)?;
    let request = ShellExecRequest {
        command,
        workdir,
        timeout_ms,
        stdout_max_bytes: None,
        signal: Some(exec.signal),
        stdin: None,
        env: None,
        dsh_env: None,
        sandbox_policy: None,
    };
    let result = run_foreground(backend, request).await?;
    if result.aborted {
        return Err(ToolError::Coded {
            message: "tool call aborted".into(),
            name: "AbortError".into(),
            code: TOOL_ABORTED.into(),
        });
    }
    Ok(json!({ "text": render_result(&result, escalation_modes) }))
}

async fn run_foreground(
    backend: &BashBackend,
    request: ShellExecRequest,
) -> Result<ShellRunResult, ToolError> {
    let outcome = match backend {
        BashBackend::Local(shell) => {
            let spec = shell.resolve(request).map_err(map_shell_error)?;
            shell.run(spec).await
        }
        BashBackend::Sandbox(shell) => {
            let spec = shell.resolve(request).map_err(map_shell_error)?;
            shell.run(spec).await
        }
    };
    outcome.map_err(map_shell_error)
}

fn map_shell_error(err: ShellError) -> ToolError {
    match err {
        ShellError::Sandbox(sandbox) if sandbox.code() == SANDBOX_UNAVAILABLE => ToolError::Coded {
            message: sandbox.to_string(),
            name: "SandboxUnavailableError".into(),
            code: SANDBOX_UNAVAILABLE.into(),
        },
        other => ToolError::Other(other.to_string()),
    }
}

fn parse_required_string(args: &Value, key: &str) -> Result<String, ToolError> {
    match args.get(key) {
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(value.clone()),
        _ => Err(ToolError::Other(format!(
            "invalid {key}: expected a non-empty string"
        ))),
    }
}

fn parse_timeout_ms(args: &Value) -> Result<Option<u64>, ToolError> {
    match args.get("timeout_ms") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            let Some(number) = value.as_f64() else {
                return Err(invalid_timeout_ms(value));
            };
            if !number.is_finite() || number <= 0.0 {
                return Err(invalid_timeout_ms(value));
            }
            let timeout_ms = number as u64;
            if timeout_ms == 0 {
                return Err(invalid_timeout_ms(value));
            }
            Ok(Some(timeout_ms))
        }
    }
}

fn invalid_timeout_ms(value: &Value) -> ToolError {
    ToolError::Other(format!(
        "invalid timeout_ms: expected a positive number, got {value}"
    ))
}

fn parse_workdir(args: &Value) -> Result<Option<String>, ToolError> {
    match args.get("workdir") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(dir)) => Ok(Some(dir.clone())),
        Some(_) => Err(ToolError::Other(
            "invalid workdir: expected a string".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::register_bash_tool;
    use dsh_agent_loop::{LoopAgent, LoopOptions};
    use dsh_fs::{LocalFileSystem, ObservationGate, ObservationOwner};
    use dsh_llm::{LlmRuntime, MockAdapter, MockScript, text_response, tool_call_response};
    use dsh_session::{SESSION_FORMAT_VERSION, Session, SessionHeader, SessionId};
    use dsh_shell::{BashConfig, LocalBashExecutor};
    use dsh_subprocess::LocalSubprocessRuntime;
    use dsh_system_prompt::{SystemPrompt, SystemPromptConfig};
    use dsh_tool_fs::{FsToolContext, register_fs_tools};
    use dsh_tools::{ToolPresentationMode, ToolRuntime};
    use serde_json::json;
    use std::sync::Arc;

    fn header(cwd: &str) -> SessionHeader {
        SessionHeader {
            version: SESSION_FORMAT_VERSION,
            id: SessionId::new("phase4-smoke"),
            created_at: 1,
            cwd: Some(cwd.into()),
            parent_session: None,
            seed_length: None,
            origin: None,
            delegation_depth: None,
            agent_preset: None,
        }
    }

    #[tokio::test]
    async fn loop_smoke_runs_real_bash_echo() {
        let subprocess = Arc::new(LocalSubprocessRuntime::new());
        let shell =
            Arc::new(LocalBashExecutor::new(subprocess.clone(), BashConfig::default()).unwrap());
        let mut tools = ToolRuntime::new(ToolPresentationMode::Native);
        register_bash_tool(&mut tools, shell, None);
        let adapter = Arc::new(MockAdapter::new(vec![
            MockScript::Chunks(tool_call_response(
                "c1",
                "bash",
                &json!({"command":"echo phase4-ok","description":"echo"}),
                Some("running"),
            )),
            MockScript::Chunks(text_response("done")),
        ]));
        let mut llm = LlmRuntime::new();
        llm.register_adapter("mock", adapter);
        let mut agent = LoopAgent::new(
            Session::new(header("/")),
            LoopOptions {
                provider: "mock".into(),
                model: "mock".into(),
                max_tokens: None,
                max_parallel_tool_calls: 10,
            },
            tools,
            SystemPrompt::new(SystemPromptConfig::default()).unwrap(),
            llm,
        )
        .unwrap();
        agent
            .followup(dsh_session::Message {
                id: dsh_session::MessageId::new("m1"),
                role: dsh_session::MessageRole::User,
                content: vec![dsh_session::ContentBlock::Text { text: "go".into() }],
                source: dsh_session::MessageSource::User,
            })
            .unwrap();
        agent.run_until_idle().await.unwrap();
        let serialized = format!("{:?}", agent.session.events());
        assert!(serialized.contains("phase4-ok"), "{serialized}");
    }

    #[tokio::test]
    async fn loop_smoke_fs_edit_without_read_is_not_observed() {
        let dir = std::env::temp_dir().join(format!("dsh-loop-fs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "hello").unwrap();
        let subprocess = Arc::new(LocalSubprocessRuntime::new());
        let mut tools = ToolRuntime::new(ToolPresentationMode::Native);
        register_fs_tools(
            &mut tools,
            FsToolContext {
                fs: Arc::new(LocalFileSystem::new(&dir)),
                gate: Arc::new(ObservationGate::new()),
                owner: ObservationOwner(1),
                sandbox: None,
                subprocess,
                rg_binary: "rg".into(),
            },
        );
        let adapter = Arc::new(MockAdapter::new(vec![
            MockScript::Chunks(tool_call_response(
                "c1",
                "edit",
                &json!({"file_path":"a.txt","old_string":"hello","new_string":"world"}),
                None,
            )),
            MockScript::Chunks(text_response("ok")),
        ]));
        let mut llm = LlmRuntime::new();
        llm.register_adapter("mock", adapter);
        let mut agent = LoopAgent::new(
            Session::new(header(&dir.to_string_lossy())),
            LoopOptions {
                provider: "mock".into(),
                model: "mock".into(),
                max_tokens: None,
                max_parallel_tool_calls: 10,
            },
            tools,
            SystemPrompt::new(SystemPromptConfig::default()).unwrap(),
            llm,
        )
        .unwrap();
        agent
            .followup(dsh_session::Message {
                id: dsh_session::MessageId::new("m1"),
                role: dsh_session::MessageRole::User,
                content: vec![dsh_session::ContentBlock::Text {
                    text: "edit".into(),
                }],
                source: dsh_session::MessageSource::User,
            })
            .unwrap();
        agent.run_until_idle().await.unwrap();
        let serialized = format!("{:?}", agent.session.events());
        assert!(serialized.contains("FS_NOT_OBSERVED"), "{serialized}");
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "hello");
    }
}
