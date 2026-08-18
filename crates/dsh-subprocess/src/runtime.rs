//! Local subprocess runtime: PATH lookup, live-tree tracking, and dispose.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::env::{EnvEntry, child_env};
use crate::error::SubprocessError;
use crate::spawn::{SubprocessHandle, spawn_subprocess};
use crate::terminal::{SubprocessTerminalHandle, SubprocessTerminalSpawnSpec};
use crate::types::SubprocessSpawnSpec;

/// Host-local subprocess runtime: PATH lookup plus a live set of spawned trees.
pub struct LocalSubprocessRuntime {
    live: Arc<Mutex<Vec<SubprocessHandle>>>,
    terminals: Arc<Mutex<Vec<SubprocessTerminalHandle>>>,
}

impl LocalSubprocessRuntime {
    /// Create a runtime with an empty live set.
    #[must_use]
    pub fn new() -> Self {
        Self {
            live: Arc::new(Mutex::new(Vec::new())),
            terminals: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Resolve `command` to an executable path in this process's execution world.
    ///
    /// Empty command is [`SubprocessError::EmptyExecutable`]. A non-absolute command containing `/` is [`SubprocessError::RelativePath`]. An absolute path must be a file with unix execute bits (`mode() & 0o111 != 0`) or [`SubprocessError::NotExecutable`]. A bare name searches `PATH` from [`child_env`] after applying `extra`; the first executable file wins, otherwise [`SubprocessError::NotOnPath`].
    ///
    /// # Errors
    ///
    /// Lookup failures listed above.
    pub async fn resolve_executable(
        &self,
        command: &str,
        extra: Option<&[EnvEntry]>,
    ) -> Result<String, SubprocessError> {
        resolve_executable(command, extra)
    }

    /// Spawn one process group and retain the handle until the tree exits or [`Self::dispose`] runs.
    ///
    /// # Errors
    ///
    /// [`crate::spawn_subprocess`] errors.
    pub fn spawn(&self, spec: SubprocessSpawnSpec) -> Result<SubprocessHandle, SubprocessError> {
        let handle = spawn_subprocess(spec)?;
        self.live
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(handle.clone());
        let live = Arc::clone(&self.live);
        let tracked = handle.clone();
        tokio::spawn(async move {
            let _ = tracked.done().await;
            let _ = tracked.wait_for_exit().await;
            live.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .retain(|item| !item.same_tree(&tracked));
        });
        Ok(handle)
    }

    /// Spawn one POSIX PTY child and retain a clone until [`Self::dispose`].
    ///
    /// `dispose` calls [`SubprocessTerminalHandle::terminate`] on each tracked handle.
    ///
    /// # Parameters
    ///
    /// * `spec` - Fully specified PTY spawn request from [`SubprocessTerminalSpawnSpec::new`].
    ///
    /// # Returns
    ///
    /// A handle sharing the tracked session.
    ///
    /// # Errors
    ///
    /// [`crate::spawn_terminal`] errors.
    pub fn spawn_terminal(
        &self,
        spec: SubprocessTerminalSpawnSpec,
    ) -> Result<SubprocessTerminalHandle, SubprocessError> {
        let handle = crate::terminal::spawn_terminal(spec)?;
        self.terminals
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(handle.clone());
        Ok(handle)
    }

    /// Terminate every live pipe tree and every tracked PTY session.
    ///
    /// Pipe trees receive SIGTERM then SIGKILL after `grace_ms`, then `wait_for_exit`. Each PTY
    /// handle is [`SubprocessTerminalHandle::terminate`]d and awaited.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// Nothing.
    pub async fn dispose(&self) {
        let handles: Vec<SubprocessHandle> = {
            let mut live = self
                .live
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::take(&mut *live)
        };
        for handle in &handles {
            handle.terminate();
        }
        for handle in &handles {
            let _ = handle.wait_for_exit().await;
        }
        let terminals: Vec<SubprocessTerminalHandle> = {
            let mut live = self
                .terminals
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::take(&mut *live)
        };
        for terminal in &terminals {
            let _ = terminal.terminate().await;
        }
    }
}

impl Default for LocalSubprocessRuntime {
    fn default() -> Self {
        Self::new()
    }
}

fn resolve_executable(
    command: &str,
    extra: Option<&[EnvEntry]>,
) -> Result<String, SubprocessError> {
    if command.is_empty() {
        return Err(SubprocessError::EmptyExecutable);
    }
    let path = Path::new(command);
    if !path.is_absolute() && command.contains('/') {
        return Err(SubprocessError::RelativePath(command.to_string()));
    }
    if path.is_absolute() {
        if is_executable_file(path) {
            return Ok(command.to_string());
        }
        return Err(SubprocessError::NotExecutable(command.to_string()));
    }
    let env = child_env(extra);
    let path_var = env
        .iter()
        .find(|(key, _)| key == "PATH")
        .map(|(_, value)| value.as_str())
        .unwrap_or("");
    for dir in path_var.split(':') {
        let candidate = join_path_entry(dir, command);
        if is_executable_file(&candidate) {
            return Ok(candidate.to_string_lossy().into_owned());
        }
    }
    Err(SubprocessError::NotOnPath(command.to_string()))
}

fn join_path_entry(dir: &str, command: &str) -> PathBuf {
    let joined = Path::new(dir).join(command);
    if joined.is_absolute() {
        joined
    } else {
        match std::env::current_dir() {
            Ok(cwd) => cwd.join(joined),
            Err(_) => joined,
        }
    }
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::LocalSubprocessRuntime;
    use crate::SubprocessError;

    #[tokio::test]
    async fn resolve_rejects_relative_path_with_slash() {
        let rt = LocalSubprocessRuntime::new();
        let err = rt.resolve_executable("./bin/echo", None).await.unwrap_err();
        assert!(matches!(err, SubprocessError::RelativePath(_)));
    }

    #[tokio::test]
    async fn resolve_finds_true_on_path() {
        let rt = LocalSubprocessRuntime::new();
        let path = rt.resolve_executable("true", None).await.unwrap();
        assert!(path.ends_with("/true") || path.ends_with("/true.exe"));
        assert!(std::path::Path::new(&path).is_absolute());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dispose_terminates_tracked_pty() {
        use crate::inspector::create_process_inspector;
        use crate::terminal::SubprocessTerminalSpawnSpec;
        use std::path::PathBuf;
        use std::time::Duration;

        let _guard = crate::terminal::lock_pty_tests().await;
        let rt = LocalSubprocessRuntime::new();
        let spec = SubprocessTerminalSpawnSpec::new(
            vec!["/bin/sh".into(), "-c".into(), "sleep 60; true".into()],
            PathBuf::from("/"),
            Vec::new(),
            40,
            160,
            3_000,
        )
        .unwrap();
        let handle = rt.spawn_terminal(spec).unwrap();
        let inspector = create_process_inspector().unwrap();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut sleep_identity = None;
        while tokio::time::Instant::now() < deadline {
            let tree = inspector.process_tree(handle.pid());
            if let Some(child) = tree.into_iter().find(|member| member.pid() != handle.pid()) {
                sleep_identity = Some(child);
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let sleep_identity = sleep_identity.expect("sleep descendant");
        rt.dispose().await;
        tokio::time::timeout(Duration::from_secs(5), handle.done())
            .await
            .expect("done")
            .unwrap();
        assert!(!inspector.is_alive(&sleep_identity));
    }
}
