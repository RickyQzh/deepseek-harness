//! POSIX process-group spawn, collect-mode readers, and SIGTERM→grace→SIGKILL.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::{Mutex, Notify};
#[cfg(unix)]
use tokio::task::JoinSet;

use crate::collect::OutputCollector;
use crate::env::child_env;
use crate::error::{MAX_GRACE_MS, SubprocessError};
use crate::types::{
    SubprocessOutcome, SubprocessOutput, SubprocessOutputRead, SubprocessSpawnSpec, SubprocessStdin,
};

/// Live handle for one spawned process group. Cloning shares the same tree via [`Arc`].
#[derive(Clone)]
pub struct SubprocessHandle {
    inner: Arc<HandleInner>,
}

impl std::fmt::Debug for SubprocessHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubprocessHandle")
            .field("pid", &self.inner.pid)
            .finish_non_exhaustive()
    }
}

struct HandleInner {
    pid: i32,
    stdout: Option<Arc<Mutex<OutputCollector>>>,
    stderr: Option<Arc<Mutex<OutputCollector>>>,
    grace_ms: u64,
    escalating: AtomicBool,
    outcome: Mutex<Option<Result<SubprocessOutcome, SubprocessError>>>,
    notify: Notify,
}

impl SubprocessHandle {
    /// Leader pid, or `-1` when spawn did not publish a pid.
    #[must_use]
    pub fn pid(&self) -> i32 {
        self.inner.pid
    }

    /// Whole-stream stdout from offset 0 when a collect-mode collector is idle enough to lock.
    ///
    /// After [`Self::done`] seals the collector, this is the batch read of the retained tail.
    #[must_use]
    pub fn collected_stdout(&self) -> Option<SubprocessOutputRead> {
        let collector = self.stdout_reader()?;
        collector.try_lock().ok().map(|guard| guard.read_from(0))
    }

    /// Collect-mode stdout collector, when stdout was [`SubprocessOutput::Collect`].
    #[must_use]
    pub fn stdout_reader(&self) -> Option<Arc<Mutex<OutputCollector>>> {
        self.inner.stdout.clone()
    }

    /// Collect-mode stderr collector, when stderr was [`SubprocessOutput::Collect`].
    #[must_use]
    pub fn stderr_reader(&self) -> Option<Arc<Mutex<OutputCollector>>> {
        self.inner.stderr.clone()
    }

    /// Start SIGTERM → `grace_ms` → SIGKILL against the process group. Idempotent.
    ///
    /// The SIGKILL timer is a Tokio task, so a live Tokio runtime must still exist.
    pub fn terminate(&self) {
        if self.inner.escalating.swap(true, Ordering::AcqRel) {
            return;
        }
        let pid = self.inner.pid;
        kill_group(pid, sigterm());
        let grace_ms = self.inner.grace_ms;
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(grace_ms)).await;
            kill_group(pid, sigkill());
        });
    }

    /// Wait for the direct child to exit, seal collectors, and return exit facts.
    ///
    /// After the child exits, collect-mode drains are joined for at most `grace_ms`. On timeout the
    /// collect readers are aborted so a descendant that still holds the write end cannot hang this
    /// future. Pipe-mode streams are not closed here.
    ///
    /// Rejects only when waiting on the child fails after a successful spawn.
    pub async fn done(&self) -> Result<SubprocessOutcome, SubprocessError> {
        loop {
            let notified = self.inner.notify.notified();
            if let Some(result) = self.inner.outcome.lock().await.clone() {
                return result;
            }
            notified.await;
        }
    }

    /// Poll until the process group is gone (`kill(-pid, 0)` reports `ESRCH`).
    ///
    /// Returns `true` when the group is absent. `pid <= 0` is already gone.
    pub async fn wait_for_exit(&self) -> bool {
        let pid = self.inner.pid;
        if pid <= 0 {
            return true;
        }
        #[cfg(unix)]
        {
            while process_group_alive(pid) {
                tokio::time::sleep(Duration::from_millis(15)).await;
            }
            true
        }
        #[cfg(not(unix))]
        {
            true
        }
    }

    pub(crate) fn same_tree(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

/// Send `sig` to POSIX process group `-pid`. No-op when `pid <= 0`. Never panics.
pub fn kill_group(pid: i32, sig: i32) {
    if pid <= 0 {
        return;
    }
    #[cfg(unix)]
    {
        // SAFETY: pid is this crate's detached group leader; ESRCH/EPERM are ignored so teardown stays idempotent.
        unsafe {
            libc::kill(-pid, sig);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = sig;
    }
}

/// Spawn a detached POSIX process group. `argv` is never joined into a shell string.
///
/// Collect-mode spill files go under a crate-owned temp directory. Non-unix hosts return [`SubprocessError::UnsupportedPlatform`]. A successful spawn must run on a Tokio runtime.
///
/// # Errors
///
/// [`SubprocessError::InvalidGrace`], [`SubprocessError::AbortedBeforeSpawn`], [`SubprocessError::InvalidArgv`], [`SubprocessError::Spawn`], or [`SubprocessError::UnsupportedPlatform`].
pub fn spawn_subprocess(spec: SubprocessSpawnSpec) -> Result<SubprocessHandle, SubprocessError> {
    spawn_subprocess_with_spill_dir(spec, private_spill_dir())
}

/// Same as [`spawn_subprocess`], using `spill_dir` for collect-mode spill files.
///
/// # Errors
///
/// Same as [`spawn_subprocess`].
pub fn spawn_subprocess_with_spill_dir(
    spec: SubprocessSpawnSpec,
    spill_dir: &Path,
) -> Result<SubprocessHandle, SubprocessError> {
    validate_spec(&spec)?;
    #[cfg(not(unix))]
    {
        let _ = spill_dir;
        Err(SubprocessError::UnsupportedPlatform)
    }
    #[cfg(unix)]
    {
        spawn_unix(spec, spill_dir)
    }
}

#[cfg(unix)]
fn spawn_unix(
    spec: SubprocessSpawnSpec,
    spill_dir: &Path,
) -> Result<SubprocessHandle, SubprocessError> {
    let program = spec.argv[0].clone();
    let args = spec.argv[1..].to_vec();
    let stdin_mode = spec.stdio.stdin;
    let stdout_mode = spec.stdio.stdout;
    let stderr_mode = spec.stdio.stderr;
    let grace_ms = spec.grace_ms;
    let abort = spec.signal.clone();
    let env = child_env(spec.env.as_deref());

    let mut command = Command::new(&program);
    command
        .args(&args)
        .current_dir(&spec.cwd)
        .env_clear()
        .kill_on_drop(false)
        .process_group(0)
        .stdin(map_stdin(&stdin_mode))
        .stdout(map_output(&stdout_mode))
        .stderr(map_output(&stderr_mode));
    for (key, value) in env {
        command.env(key, value);
    }

    let mut child = command
        .spawn()
        .map_err(|error| SubprocessError::Spawn(error.to_string()))?;
    let pid = child
        .id()
        .map(|id| i32::try_from(id).unwrap_or(-1))
        .unwrap_or(-1);

    if let SubprocessStdin::Data(data) = stdin_mode {
        if let Some(mut stdin) = child.stdin.take() {
            tokio::spawn(async move {
                let _ = stdin.write_all(data.as_bytes()).await;
                let _ = stdin.shutdown().await;
            });
        }
    }

    let stdout_pipe = match stdout_mode {
        SubprocessOutput::Collect(_) => child.stdout.take(),
        _ => None,
    };
    let stderr_pipe = match stderr_mode {
        SubprocessOutput::Collect(_) => child.stderr.take(),
        _ => None,
    };
    let mut drains = JoinSet::new();
    let stdout = start_collect(stdout_pipe, &stdout_mode, "stdout", spill_dir, &mut drains);
    let stderr = start_collect(stderr_pipe, &stderr_mode, "stderr", spill_dir, &mut drains);

    let inner = Arc::new(HandleInner {
        pid,
        stdout,
        stderr,
        grace_ms,
        escalating: AtomicBool::new(false),
        outcome: Mutex::new(None),
        notify: Notify::new(),
    });
    let waiter_inner = Arc::clone(&inner);
    tokio::spawn(async move {
        let wait_result = child.wait().await;
        join_collect_drains(&mut drains, waiter_inner.grace_ms).await;
        if let Some(collector) = &waiter_inner.stdout {
            collector.lock().await.seal();
        }
        if let Some(collector) = &waiter_inner.stderr {
            collector.lock().await.seal();
        }
        let result = match wait_result {
            Ok(status) => Ok(outcome_from_status(status)),
            Err(error) => Err(SubprocessError::Spawn(error.to_string())),
        };
        *waiter_inner.outcome.lock().await = Some(result);
        waiter_inner.notify.notify_waiters();
    });

    let handle = SubprocessHandle { inner };
    if let Some(flag) = abort {
        let abort_handle = handle.clone();
        tokio::spawn(async move {
            tokio::select! {
                () = flag.cancelled() => abort_handle.terminate(),
                _ = abort_handle.done() => {}
            }
        });
    }
    Ok(handle)
}

fn validate_spec(spec: &SubprocessSpawnSpec) -> Result<(), SubprocessError> {
    if spec.grace_ms == 0 || spec.grace_ms > MAX_GRACE_MS {
        return Err(SubprocessError::InvalidGrace);
    }
    if spec
        .signal
        .as_ref()
        .is_some_and(dsh_tools::AbortFlag::is_aborted)
    {
        return Err(SubprocessError::AbortedBeforeSpawn);
    }
    match spec.argv.first().map(String::as_str) {
        Some(program) if !program.is_empty() => Ok(()),
        _ => Err(SubprocessError::InvalidArgv),
    }
}

#[cfg(unix)]
fn map_stdin(stdin: &SubprocessStdin) -> Stdio {
    match stdin {
        SubprocessStdin::Ignore => Stdio::null(),
        SubprocessStdin::Pipe | SubprocessStdin::Data(_) => Stdio::piped(),
    }
}

#[cfg(unix)]
fn map_output(output: &SubprocessOutput) -> Stdio {
    match output {
        SubprocessOutput::Inherit => Stdio::inherit(),
        SubprocessOutput::Pipe | SubprocessOutput::Collect(_) => Stdio::piped(),
    }
}

#[cfg(unix)]
fn start_collect<R>(
    reader: Option<R>,
    mode: &SubprocessOutput,
    label: &str,
    spill_dir: &Path,
    drains: &mut JoinSet<()>,
) -> Option<Arc<Mutex<OutputCollector>>>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    match mode {
        SubprocessOutput::Collect(collect) => {
            let reader = reader?;
            let collector = Arc::new(Mutex::new(OutputCollector::new(
                collect.max_bytes,
                collect.spill_max_bytes,
                label,
                spill_dir,
            )));
            let task_collector = Arc::clone(&collector);
            drains.spawn(async move {
                drain_into(reader, task_collector).await;
            });
            Some(collector)
        }
        _ => None,
    }
}

/// Join collect drain tasks for at most `grace_ms`, then abort any still-running readers.
///
/// Drain tasks live in a [`JoinSet`] so each is joined at most once: timeout around `join_next`,
/// then `abort_all` and `join_next` until empty. Aborting drops `ChildStdout`/`ChildStderr` (the
/// Rust equivalent of Node `destroy()`), so a descendant that inherited a collect-mode pipe cannot
/// hold [`SubprocessHandle::done`] open. Pipe-mode streams are not passed here.
#[cfg(unix)]
async fn join_collect_drains(drains: &mut JoinSet<()>, grace_ms: u64) {
    let join_remaining = async { while drains.join_next().await.is_some() {} };
    if tokio::time::timeout(Duration::from_millis(grace_ms), join_remaining)
        .await
        .is_err()
    {
        drains.abort_all();
        while drains.join_next().await.is_some() {}
    }
}

#[cfg(unix)]
async fn drain_into<R>(mut reader: R, collector: Arc<Mutex<OutputCollector>>)
where
    R: AsyncRead + Unpin,
{
    let mut buf = vec![0u8; 8192];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => collector.lock().await.push(&buf[..n]),
        }
    }
}

#[cfg(unix)]
fn outcome_from_status(status: std::process::ExitStatus) -> SubprocessOutcome {
    use std::os::unix::process::ExitStatusExt;
    SubprocessOutcome {
        exit_code: status.code(),
        signal: status.signal(),
    }
}

fn private_spill_dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("dsh-subprocess-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
        dir
    })
    .as_path()
}

const fn sigterm() -> i32 {
    #[cfg(unix)]
    {
        libc::SIGTERM
    }
    #[cfg(not(unix))]
    {
        15
    }
}

const fn sigkill() -> i32 {
    #[cfg(unix)]
    {
        libc::SIGKILL
    }
    #[cfg(not(unix))]
    {
        9
    }
}

#[cfg(unix)]
fn process_group_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    // SAFETY: pid is this crate's detached group leader; a 0 signal only probes liveness.
    let rc = unsafe { libc::kill(-pid, 0) };
    if rc == 0 {
        return true;
    }
    !matches!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(errno) if errno == libc::ESRCH
    )
}

#[cfg(test)]
mod tests {
    use super::spawn_subprocess;
    use crate::{
        MAX_GRACE_MS, SubprocessCollect, SubprocessError, SubprocessOutput, SubprocessSpawnSpec,
        SubprocessStdin, SubprocessStdio,
    };
    use dsh_tools::AbortFlag;

    fn echo_spec(arg: &str) -> SubprocessSpawnSpec {
        SubprocessSpawnSpec {
            argv: vec!["/bin/echo".into(), arg.into()],
            cwd: std::env::current_dir()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            stdio: SubprocessStdio {
                stdin: SubprocessStdin::Ignore,
                stdout: SubprocessOutput::Collect(SubprocessCollect {
                    max_bytes: 64_000,
                    spill_max_bytes: None,
                }),
                stderr: SubprocessOutput::Collect(SubprocessCollect {
                    max_bytes: 64_000,
                    spill_max_bytes: None,
                }),
            },
            grace_ms: 3_000,
            signal: None,
            env: None,
        }
    }

    #[test]
    fn empty_argv0_is_invalid() {
        let mut spec = echo_spec("x");
        spec.argv = vec!["".into()];
        let err = spawn_subprocess(spec).unwrap_err();
        assert!(matches!(err, SubprocessError::InvalidArgv));
    }

    #[test]
    fn zero_grace_is_invalid() {
        let mut spec = echo_spec("x");
        spec.grace_ms = 0;
        assert!(matches!(
            spawn_subprocess(spec).unwrap_err(),
            SubprocessError::InvalidGrace
        ));
    }

    #[test]
    fn oversize_grace_is_invalid() {
        let mut spec = echo_spec("x");
        spec.grace_ms = MAX_GRACE_MS + 1;
        assert!(matches!(
            spawn_subprocess(spec).unwrap_err(),
            SubprocessError::InvalidGrace
        ));
    }

    #[test]
    fn aborted_flag_rejects_before_spawn() {
        let flag = AbortFlag::new();
        flag.abort();
        let mut spec = echo_spec("x");
        spec.signal = Some(flag);
        assert!(matches!(
            spawn_subprocess(spec).unwrap_err(),
            SubprocessError::AbortedBeforeSpawn
        ));
    }

    #[tokio::test]
    async fn argv_is_not_a_shell_string() {
        let handle = spawn_subprocess(echo_spec("a b")).unwrap();
        let outcome = handle.done().await.unwrap();
        assert_eq!(outcome.exit_code, Some(0));
        let stdout = handle.stdout_reader().unwrap();
        let text = stdout.lock().await.read_from(0).text;
        assert_eq!(text, "a b\n");
    }

    #[tokio::test]
    async fn terminate_kills_a_sleeping_process_group() {
        let spec = SubprocessSpawnSpec {
            argv: vec!["/bin/sleep".into(), "30".into()],
            cwd: "/".into(),
            stdio: SubprocessStdio {
                stdin: SubprocessStdin::Ignore,
                stdout: SubprocessOutput::Collect(SubprocessCollect {
                    max_bytes: 64,
                    spill_max_bytes: None,
                }),
                stderr: SubprocessOutput::Collect(SubprocessCollect {
                    max_bytes: 64,
                    spill_max_bytes: None,
                }),
            },
            grace_ms: 200,
            signal: None,
            env: None,
        };
        let handle = spawn_subprocess(spec).unwrap();
        assert!(handle.pid() > 0);
        handle.terminate();
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), handle.done())
            .await
            .expect("done")
            .unwrap();
        assert!(outcome.exit_code != Some(0) || outcome.signal.is_some());
        assert!(handle.wait_for_exit().await);
    }

    fn background_sleep_spec(shell_command: &str) -> SubprocessSpawnSpec {
        SubprocessSpawnSpec {
            argv: vec!["/bin/sh".into(), "-c".into(), shell_command.into()],
            cwd: "/".into(),
            stdio: SubprocessStdio {
                stdin: SubprocessStdin::Ignore,
                stdout: SubprocessOutput::Collect(SubprocessCollect {
                    max_bytes: 64,
                    spill_max_bytes: None,
                }),
                stderr: SubprocessOutput::Collect(SubprocessCollect {
                    max_bytes: 64,
                    spill_max_bytes: None,
                }),
            },
            grace_ms: 200,
            signal: None,
            env: None,
        }
    }

    async fn done_then_reap(handle: super::SubprocessHandle) {
        tokio::time::timeout(std::time::Duration::from_secs(5), handle.done())
            .await
            .expect("done")
            .unwrap();
        handle.terminate();
        assert!(handle.wait_for_exit().await);
    }

    #[tokio::test]
    async fn bounds_inherited_pipe_draining_after_the_shell_exits() {
        let handle = spawn_subprocess(background_sleep_spec("sleep 30 &")).unwrap();
        done_then_reap(handle).await;
    }

    #[tokio::test]
    async fn bounds_inherited_stderr_when_stdout_is_redirected() {
        let handle = spawn_subprocess(background_sleep_spec("sleep 30 >/dev/null &")).unwrap();
        done_then_reap(handle).await;
    }

    #[tokio::test]
    async fn bounds_inherited_stdout_when_stderr_is_redirected() {
        let handle = spawn_subprocess(background_sleep_spec("sleep 30 2>/dev/null &")).unwrap();
        done_then_reap(handle).await;
    }
}
