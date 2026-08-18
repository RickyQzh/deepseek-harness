//! MCP stdio spawn: credential-scrubbed child env, no shell, piped stdin/stdout.

use std::process::Stdio;

use dsh_subprocess::{EnvEntry, child_env};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::process::{Child, Command};

use crate::client::McpSession;

/// Failure spawning or wiring an MCP stdio child.
#[derive(Debug, thiserror::Error)]
pub enum StdioSpawnError {
    /// The OS refused to spawn the process.
    #[error("MCP stdio spawn failed: {0}")]
    Spawn(String),
    /// Spawned child is missing piped stdin or stdout.
    #[error("MCP stdio child is missing stdin or stdout")]
    MissingStdio,
}

/// Build the credential-scrubbed MCP stdio child environment, then apply `overlay`.
///
/// # Parameters
///
/// * `overlay` - Explicit `config.env` entries. `Some` sets or replaces that key;
///   `None` removes it.
///
/// # Returns
///
/// Pairs for [`Command::env_clear`] then [`Command::envs`]. Parent credential-shaped
/// names such as `DEEPSEEK_API_KEY` are absent unless `overlay` restores them.
#[must_use]
pub fn stdio_child_env(overlay: &[EnvEntry]) -> Vec<(String, String)> {
    child_env(Some(overlay))
}

/// Build a tokio process command for MCP stdio without spawning.
///
/// Uses `Command::new(program).args(args)` only (no shell). `cwd` empty or
/// `None` inherits the parent working directory. Callers pass the map from
/// [`stdio_child_env`].
///
/// # Parameters
///
/// * `program` - Child executable, not a shell string.
/// * `args` - Argument vector after `program`.
/// * `env` - Map from [`stdio_child_env`], applied after `env_clear`.
/// * `cwd` - Child working directory; empty or `None` inherits.
///
/// # Returns
///
/// A command with piped stdin/stdout, inherited stderr, and `kill_on_drop`.
#[must_use]
pub fn stdio_command(
    program: &str,
    args: &[String],
    env: &[(String, String)],
    cwd: Option<&str>,
) -> Command {
    let mut command = Command::new(program);
    command.args(args);
    command.env_clear().envs(env.iter().cloned());
    if let Some(path) = cwd {
        if !path.is_empty() {
            command.current_dir(path);
        }
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    command
}

/// Spawn an MCP stdio child and bind [`McpSession::from_stdio`] to its pipes.
///
/// # Parameters
///
/// * `program` - Child executable, not a shell string.
/// * `args` - Argument vector after `program`.
/// * `overlay` - Explicit `config.env` entries applied after the scrubbed parent env.
/// * `cwd` - Child working directory; empty or `None` inherits.
///
/// # Returns
///
/// The session (child stdout is the MCP read stream; child stdin is the MCP write
/// stream) and the live [`Child`].
///
/// # Errors
///
/// [`StdioSpawnError::Spawn`] when the OS refuses spawn.
/// [`StdioSpawnError::MissingStdio`] when stdin or stdout is not piped.
pub fn spawn_stdio(
    program: &str,
    args: &[String],
    overlay: &[EnvEntry],
    cwd: Option<&str>,
) -> Result<(McpSession, Child), StdioSpawnError> {
    let env = stdio_child_env(overlay);
    let mut command = stdio_command(program, args, &env, cwd);
    let mut child = command
        .spawn()
        .map_err(|err| StdioSpawnError::Spawn(err.to_string()))?;
    let stdout = child.stdout.take().ok_or(StdioSpawnError::MissingStdio)?;
    let stdin = child.stdin.take().ok_or(StdioSpawnError::MissingStdio)?;
    Ok((McpSession::from_stdio(stdout, stdin), child))
}

impl McpSession {
    /// Bind an MCP session to already-open server stdout and stdin streams.
    ///
    /// Does not spawn a child. `server_stdout` is the MCP read stream;
    /// `server_stdin` is the MCP write stream.
    ///
    /// # Parameters
    ///
    /// * `server_stdout` - Bytes from the server (child stdout or a duplex end).
    /// * `server_stdin` - Bytes to the server (child stdin or a duplex end).
    ///
    /// # Returns
    ///
    /// A session that speaks Content-Length JSON-RPC on those streams.
    #[must_use]
    pub fn from_stdio<R, W>(server_stdout: R, server_stdin: W) -> Self
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        Self::new(server_stdout, server_stdin)
    }
}

#[cfg(test)]
mod tests {
    use super::stdio_child_env;
    use dsh_subprocess::EnvEntry;

    #[test]
    fn stdio_child_env_scrubs_parent_secret() {
        let previous_key = std::env::var("DEEPSEEK_API_KEY").ok();
        unsafe {
            std::env::set_var("DEEPSEEK_API_KEY", "secret-value");
        }
        let env = stdio_child_env(&[EnvEntry {
            key: "MCP_CONFIG_ENV".into(),
            value: Some("overlay-value".into()),
        }]);
        assert!(env.iter().all(|(k, _)| k != "DEEPSEEK_API_KEY"));
        assert_eq!(
            env.iter()
                .find(|(k, _)| k == "MCP_CONFIG_ENV")
                .map(|(_, v)| v.as_str()),
            Some("overlay-value")
        );
        match previous_key {
            Some(value) => unsafe { std::env::set_var("DEEPSEEK_API_KEY", value) },
            None => unsafe { std::env::remove_var("DEEPSEEK_API_KEY") },
        }
    }
}
