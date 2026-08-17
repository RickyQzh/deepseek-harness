//! Local shell backend registered under the configured type.

use std::path::PathBuf;
use std::sync::Arc;

use dsh_kernel::Context;
use dsh_sandbox::{
    LocalSandboxProvider, SandboxExecutionPolicy, SandboxMode, SandboxPolicy, SandboxPolicyResolver,
};
use dsh_subprocess::{EnvEntry, LocalSubprocessRuntime, SubprocessTerminalSpawnSpec};
use dsh_terminal::{
    TerminalBackend, TerminalBackendSession, TerminalBackendSpawnFuture, TerminalBackendSpawnSpec,
    TerminalError, TerminalErrorCode,
};

use crate::config::ResolvedConfig;
use crate::sanitize::CONTROLLED_PROMPT;
use crate::session::LocalPtySession;

fn pty_error(message: impl Into<String>) -> TerminalError {
    TerminalError::new(message, TerminalErrorCode::NoBackend)
}

fn env(key: &str, value: &str) -> EnvEntry {
    EnvEntry {
        key: key.to_string(),
        value: Some(value.to_string()),
    }
}

fn child_environment(spec: &TerminalBackendSpawnSpec) -> Vec<EnvEntry> {
    vec![
        env("TERM", "dumb"),
        env("PAGER", "cat"),
        env("GIT_PAGER", "cat"),
        env("PS1", CONTROLLED_PROMPT),
        env("PROMPT_COMMAND", "printf \"\x1b]133;D;%s\x07\" \"$?\""),
        env("BASH_SILENCE_DEPRECATION_WARNING", "1"),
        env("DSH_SHELL", "1"),
        env("DSH_SESSION_ID", spec.owner().as_str()),
        env("DSH_PTY_SESSION_ID", spec.session_id().as_str()),
    ]
}

fn spawn_argv(
    ctx: &Context,
    config: &ResolvedConfig,
    policy: &SandboxExecutionPolicy,
) -> Result<Vec<String>, TerminalError> {
    let mut argv = Vec::with_capacity(config.shell_args.len().saturating_add(1));
    argv.push(config.shell_path.clone());
    argv.extend(config.shell_args.iter().cloned());
    if policy.mode == SandboxMode::DangerFullAccess {
        return Ok(argv);
    }
    let sandbox = match ctx.get::<LocalSandboxProvider>("sandbox") {
        Some(provider) => provider,
        None => {
            return Err(pty_error(format!(
                "terminal-bash: sandbox mode \"{}\" requires a ctx.sandbox provider in the execution world",
                policy.mode.as_str()
            )));
        }
    };
    let confined =
        SandboxPolicy::confined(policy.clone()).map_err(|error| pty_error(error.to_string()))?;
    let wrapped = sandbox
        .confine(&argv, &confined)
        .map_err(|error| pty_error(error.to_string()))?;
    Ok(wrapped.argv)
}

/// Local bash PTY backend.
pub(crate) struct BashTerminalBackend {
    config: ResolvedConfig,
    subprocess: Arc<LocalSubprocessRuntime>,
    sandbox_policy: Arc<SandboxPolicyResolver>,
    ctx: Context,
}

impl BashTerminalBackend {
    pub(crate) fn new(
        config: ResolvedConfig,
        subprocess: Arc<LocalSubprocessRuntime>,
        sandbox_policy: Arc<SandboxPolicyResolver>,
        ctx: Context,
    ) -> Self {
        Self {
            config,
            subprocess,
            sandbox_policy,
            ctx,
        }
    }

    pub(crate) fn backend_type(&self) -> &str {
        &self.config.backend_type
    }
}

impl TerminalBackend for BashTerminalBackend {
    fn spawn(&self, spec: TerminalBackendSpawnSpec) -> TerminalBackendSpawnFuture {
        let config = self.config.clone();
        let subprocess = Arc::clone(&self.subprocess);
        let sandbox_policy = Arc::clone(&self.sandbox_policy);
        let ctx = self.ctx.clone();
        Box::pin(async move { spawn_session(config, subprocess, sandbox_policy, ctx, spec).await })
    }
}

async fn spawn_session(
    config: ResolvedConfig,
    subprocess: Arc<LocalSubprocessRuntime>,
    sandbox_policy: Arc<SandboxPolicyResolver>,
    ctx: Context,
    spec: TerminalBackendSpawnSpec,
) -> Result<Box<dyn TerminalBackendSession>, TerminalError> {
    let policy = sandbox_policy.resolve(Some(spec.owner().clone()), None);
    let cwd = match spec.cwd() {
        Some(cwd) => PathBuf::from(cwd),
        None => PathBuf::from(&policy.workspace_root),
    };
    let argv = spawn_argv(&ctx, &config, &policy)?;
    let spawn_spec = SubprocessTerminalSpawnSpec::new(
        argv,
        cwd,
        child_environment(&spec),
        config.rows,
        config.cols,
        config.dispose_grace_ms,
    )
    .map_err(|error| pty_error(error.to_string()))?;
    let terminal = subprocess
        .spawn_terminal(spawn_spec)
        .map_err(|error| pty_error(error.to_string()))?;
    let session = LocalPtySession::new(terminal, config);
    if let Err(error) = session.initialize().await {
        match session.close("PTY startup failed").await {
            Ok(()) => return Err(error),
            Err(close_error) => {
                return Err(pty_error(format!("{error}; {close_error}")));
            }
        }
    }
    Ok(Box::new(session))
}
