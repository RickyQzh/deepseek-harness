//! Request, resolved spec, run result, and process-status types for the shell executor.

use dsh_sandbox::{SandboxEnforcement, SandboxExecutionPolicy, SandboxMode};
use dsh_subprocess::{CollectedOutput, EnvEntry};
use dsh_tools::AbortFlag;

/// Caller-facing execution request. Omitted `workdir`, `timeout_ms`, and `stdout_max_bytes` are filled by executor `resolve`.
///
/// Pass the result of `resolve` to `run` / `start`; those methods never re-default omitted request fields.
#[derive(Clone, Debug)]
pub struct ShellExecRequest {
    /// Shell command string.
    pub command: String,
    /// Working-directory override. `None` uses the implementation default.
    pub workdir: Option<String>,
    /// Timeout override in milliseconds. Implementations cap it.
    pub timeout_ms: Option<u64>,
    /// Foreground stdout capture budget in bytes. `None` uses the executor's default cap.
    pub stdout_max_bytes: Option<usize>,
    /// Abort flag; implementations kill the command when it fires. `None` means no abort signal.
    pub signal: Option<AbortFlag>,
    /// Bytes to write to stdin, then close it. `None` leaves stdin empty.
    pub stdin: Option<String>,
    /// Ordinary environment overlay merged after the credential scrub. `None` means no extra env.
    pub env: Option<Vec<EnvEntry>>,
    /// Harness-owned `DSH_*` pairs merged after [`env`](Self::env). Keys must start with `DSH_`. [`crate::LocalBashExecutor::resolve`] rejects any other prefix.
    pub dsh_env: Option<Vec<(String, String)>>,
    /// Fully resolved per-call sandbox policy. Sandboxing executors default it when `None`.
    pub sandbox_policy: Option<SandboxExecutionPolicy>,
}

/// Resolved execution spec. `run` and `start` take this type and never re-default omitted request fields.
#[derive(Clone, Debug)]
pub struct ShellExecSpec {
    /// Shell command string.
    pub command: String,
    /// Working directory after `resolve`.
    pub workdir: String,
    /// Timeout in milliseconds after defaulting and capping. Background `start` ignores this field.
    pub timeout_ms: u64,
    /// Foreground stdout capture budget in bytes after `resolve`.
    pub stdout_max_bytes: usize,
    /// Abort flag; implementations kill the command when it fires. `None` means no abort signal.
    pub signal: Option<AbortFlag>,
    /// Bytes to write to stdin, then close it. `None` means no stdin.
    pub stdin: Option<String>,
    /// Ordinary environment overlay carried through from [`ShellExecRequest::env`].
    pub env: Option<Vec<EnvEntry>>,
    /// Harness-owned `DSH_*` pairs. Keys must start with `DSH_`. [`crate::LocalBashExecutor::resolve`] rejects any other prefix.
    pub dsh_env: Option<Vec<(String, String)>>,
    /// Resolved sandbox policy; ignored by executors that do not confine. `None` means no policy.
    pub sandbox_policy: Option<SandboxExecutionPolicy>,
}

/// Sandbox facts for one run, present iff a sandboxing executor handled it.
///
/// Facts are independent of process exit status so callers can distinguish command failures from policy denials and runner failures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellSandboxInfo {
    /// Mode the command actually ran under.
    pub mode: SandboxMode,
    /// Whether the sandbox denied a file operation.
    pub denied: bool,
    /// How completely the selected runner enforced the requested mode.
    pub enforcement: Option<SandboxEnforcement>,
    /// Whether the sandbox runner failed before the command could run.
    pub runner_failed: Option<bool>,
}

/// Outcome of one completed or killed foreground run.
#[derive(Clone, Debug)]
pub struct ShellRunResult {
    /// Exit code. `None` when the process died from a signal.
    pub exit_code: Option<i32>,
    /// Terminating signal number. `None` on normal exit.
    pub signal: Option<i32>,
    /// True when the executor timeout was the first cause that cut the command short.
    pub timed_out: bool,
    /// True when the caller's abort flag was the first cause that killed the command.
    pub aborted: bool,
    /// Effective timeout applied to this run after defaulting and capping.
    pub timeout_ms: u64,
    /// Captured stdout.
    pub stdout: CollectedOutput,
    /// Captured stderr.
    pub stderr: CollectedOutput,
    /// Sandbox execution facts. `None` for an unsandboxed executor.
    pub sandbox: Option<ShellSandboxInfo>,
}

/// Lifecycle of a background process.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShellProcessStatus {
    /// The process has not yet closed.
    Running,
    /// The process exited on its own.
    Completed,
    /// The process was killed.
    Killed,
}

#[cfg(test)]
mod tests {
    use super::{ShellExecRequest, ShellExecSpec, ShellProcessStatus};

    #[test]
    fn request_omits_resolved_fields_and_spec_requires_them() {
        let request = ShellExecRequest {
            command: "true".into(),
            workdir: None,
            timeout_ms: None,
            stdout_max_bytes: None,
            signal: None,
            stdin: None,
            env: None,
            dsh_env: Some(vec![("NOT_DSH".into(), "x".into())]),
            sandbox_policy: None,
        };
        assert_eq!(request.command, "true");
        assert!(request.workdir.is_none());
        let spec = ShellExecSpec {
            command: request.command.clone(),
            workdir: "/tmp".into(),
            timeout_ms: 30_000,
            stdout_max_bytes: 1024,
            signal: None,
            stdin: None,
            env: None,
            dsh_env: request.dsh_env.clone(),
            sandbox_policy: None,
        };
        assert_eq!(spec.workdir, "/tmp");
        assert_eq!(spec.timeout_ms, 30_000);
        assert_eq!(spec.stdout_max_bytes, 1024);
        assert_eq!(spec.dsh_env.as_ref().unwrap()[0].0, "NOT_DSH");
        assert_ne!(ShellProcessStatus::Running, ShellProcessStatus::Completed);
    }
}
