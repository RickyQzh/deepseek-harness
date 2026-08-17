//! POSIX PTY allocation via `portable-pty`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::Notify;

use crate::env::{EnvEntry, child_env};
use crate::error::SubprocessError;
use crate::types::SubprocessOutcome;

/// How many UTF-8 lossy chunks a [`tokio::sync::broadcast`] subscriber may buffer before lagging.
const OUTPUT_CAPACITY: usize = 256;

/// Fully specified POSIX PTY spawn request. Every field is explicit; this type applies no defaults.
#[derive(Clone, Debug)]
pub struct SubprocessTerminalSpawnSpec {
    argv: Vec<String>,
    cwd: PathBuf,
    env: Vec<EnvEntry>,
    rows: u16,
    cols: u16,
    grace_ms: u64,
}

impl SubprocessTerminalSpawnSpec {
    /// Construct a PTY spawn request.
    ///
    /// # Parameters
    ///
    /// * `argv` - Program and arguments; `argv[0]` is the executable, never a shell string.
    /// * `cwd` - Child working directory.
    /// * `env` - Overlay applied after the scrubbed parent environment.
    /// * `rows` - Initial PTY rows.
    /// * `cols` - Initial PTY columns.
    /// * `grace_ms` - TERM-to-KILL grace retained for session teardown.
    ///
    /// # Returns
    ///
    /// The spec when `argv` names a program.
    ///
    /// # Errors
    ///
    /// [`SubprocessError::InvalidArgv`] when `argv` is empty or `argv[0]` is empty.
    pub fn new(
        argv: Vec<String>,
        cwd: PathBuf,
        env: Vec<EnvEntry>,
        rows: u16,
        cols: u16,
        grace_ms: u64,
    ) -> Result<Self, SubprocessError> {
        match argv.first().map(String::as_str) {
            Some(program) if !program.is_empty() => Ok(Self {
                argv,
                cwd,
                env,
                rows,
                cols,
                grace_ms,
            }),
            _ => Err(SubprocessError::InvalidArgv),
        }
    }

    /// Program and arguments. `argv[0]` is the executable.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The argv slice passed to [`Self::new`].
    #[must_use]
    pub fn argv(&self) -> &[String] {
        &self.argv
    }

    /// Child working directory.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The cwd path passed to [`Self::new`].
    #[must_use]
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// Explicit environment overlay merged after the scrubbed parent.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The overlay passed to [`Self::new`].
    #[must_use]
    pub fn env(&self) -> &[EnvEntry] {
        &self.env
    }

    /// Initial PTY rows.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The row count passed to [`Self::new`].
    #[must_use]
    pub fn rows(&self) -> u16 {
        self.rows
    }

    /// Initial PTY columns.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The column count passed to [`Self::new`].
    #[must_use]
    pub fn cols(&self) -> u16 {
        self.cols
    }

    /// TERM-to-KILL grace in milliseconds.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The grace passed to [`Self::new`].
    #[must_use]
    pub fn grace_ms(&self) -> u64 {
        self.grace_ms
    }
}

/// Live POSIX PTY child. Cloning shares the same session via [`Arc`].
///
/// Last-handle drop closes the PTY master and signals the child. Inspect, signal, and terminate
/// are omitted.
#[derive(Clone)]
pub struct SubprocessTerminalHandle {
    inner: Arc<HandleInner>,
}

impl std::fmt::Debug for SubprocessTerminalHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubprocessTerminalHandle")
            .field("pid", &self.inner.pid)
            .finish_non_exhaustive()
    }
}

struct HandleInner {
    pid: i32,
    sender: tokio::sync::broadcast::Sender<String>,
    starter_rx: std::sync::Mutex<Option<tokio::sync::broadcast::Receiver<String>>>,
    outcome: tokio::sync::Mutex<Option<Result<SubprocessOutcome, SubprocessError>>>,
    notify: Notify,
    #[cfg(unix)]
    writer: std::sync::Mutex<Option<Box<dyn std::io::Write + Send>>>,
    #[cfg(unix)]
    master: std::sync::Mutex<Option<Box<dyn portable_pty::MasterPty + Send>>>,
    #[cfg(unix)]
    reader: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
    #[cfg(unix)]
    waiter: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl SubprocessTerminalHandle {
    /// Top-level PTY child pid, or `-1` when spawn did not publish a pid.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The child pid as `i32`.
    #[must_use]
    pub fn pid(&self) -> i32 {
        self.inner.pid
    }

    /// Write text to the PTY master without adding a newline.
    ///
    /// # Parameters
    ///
    /// * `data` - Bytes to deliver; encoding is the caller's UTF-8 text.
    ///
    /// # Returns
    ///
    /// `Ok(())` when the write and flush succeed.
    ///
    /// # Errors
    ///
    /// [`SubprocessError::UnsupportedPlatform`] on non-unix hosts.
    /// [`SubprocessError::Spawn`] when the writer is closed or the OS write fails.
    pub async fn write(&self, data: &str) -> Result<(), SubprocessError> {
        #[cfg(not(unix))]
        {
            let _ = data;
            Err(SubprocessError::UnsupportedPlatform)
        }
        #[cfg(unix)]
        {
            use std::io::Write;
            let bytes = data.as_bytes().to_vec();
            let inner = Arc::clone(&self.inner);
            tokio::task::spawn_blocking(move || {
                let mut guard = inner
                    .writer
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                match guard.as_mut() {
                    None => Err(SubprocessError::Spawn("pty writer closed".to_string())),
                    Some(sink) => sink
                        .write_all(&bytes)
                        .and_then(|()| sink.flush())
                        .map_err(|error| SubprocessError::Spawn(error.to_string())),
                }
            })
            .await
            .map_err(|error| SubprocessError::Spawn(error.to_string()))?
        }
    }

    /// Subscribe to UTF-8 lossy PTY output chunks.
    ///
    /// The first call returns the receiver created at spawn, so it observes chunks already
    /// published. Later calls use [`tokio::sync::broadcast::Sender::subscribe`] and may miss
    /// earlier chunks. A receiver that lags by more than 256 chunks reports
    /// [`tokio::sync::broadcast::error::RecvError::Lagged`].
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// A [`tokio::sync::broadcast::Receiver<String>`] of lossy UTF-8 chunks.
    #[must_use]
    pub fn output(&self) -> tokio::sync::broadcast::Receiver<String> {
        let mut slot = self
            .inner
            .starter_rx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match slot.take() {
            Some(rx) => rx,
            None => self.inner.sender.subscribe(),
        }
    }

    /// Wait until the top-level PTY child exits.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// Exit facts with no output payload.
    ///
    /// # Errors
    ///
    /// [`SubprocessError::Spawn`] when waiting on the child fails after a successful spawn.
    pub async fn done(&self) -> Result<SubprocessOutcome, SubprocessError> {
        loop {
            let notified = self.inner.notify.notified();
            if let Some(result) = self.inner.outcome.lock().await.clone() {
                return result;
            }
            notified.await;
        }
    }
}

/// Allocate a POSIX PTY and spawn `spec.argv` as the session child.
///
/// Child environment is [`child_env`] after `env_clear` on the PTY command. Output is UTF-8
/// lossy `String` chunks on a [`tokio::sync::broadcast`] channel; see
/// [`SubprocessTerminalHandle::output`].
///
/// # Parameters
///
/// * `spec` - Fully specified PTY spawn request from [`SubprocessTerminalSpawnSpec::new`].
///
/// # Returns
///
/// A handle to the live PTY. Last-handle drop closes the master and signals the child.
///
/// # Errors
///
/// [`SubprocessError::UnsupportedPlatform`] on non-unix hosts.
/// [`SubprocessError::Spawn`] when the OS refuses PTY allocation or spawn.
pub fn spawn_terminal(
    spec: SubprocessTerminalSpawnSpec,
) -> Result<SubprocessTerminalHandle, SubprocessError> {
    #[cfg(not(unix))]
    {
        let _ = spec;
        Err(SubprocessError::UnsupportedPlatform)
    }
    #[cfg(unix)]
    {
        spawn_unix(spec)
    }
}

#[cfg(unix)]
fn spawn_unix(
    spec: SubprocessTerminalSpawnSpec,
) -> Result<SubprocessTerminalHandle, SubprocessError> {
    use std::io::Read;

    use portable_pty::{CommandBuilder, PtySize, native_pty_system};

    let pty_system = native_pty_system();
    let portable_pty::PtyPair { master, slave } = pty_system
        .openpty(PtySize {
            rows: spec.rows,
            cols: spec.cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| SubprocessError::Spawn(error.to_string()))?;

    let mut reader = master
        .try_clone_reader()
        .map_err(|error| SubprocessError::Spawn(error.to_string()))?;
    let writer = master
        .take_writer()
        .map_err(|error| SubprocessError::Spawn(error.to_string()))?;

    let mut command = CommandBuilder::new(&spec.argv[0]);
    command.args(&spec.argv[1..]);
    command.cwd(&spec.cwd);
    command.env_clear();
    for (key, value) in child_env(Some(&spec.env)) {
        command.env(key, value);
    }

    let mut child = slave
        .spawn_command(command)
        .map_err(|error| SubprocessError::Spawn(error.to_string()))?;
    drop(slave);

    let pid = child
        .process_id()
        .map(|id| i32::try_from(id).unwrap_or(-1))
        .unwrap_or(-1);

    let (sender, starter_rx) = tokio::sync::broadcast::channel(OUTPUT_CAPACITY);
    let reader_sender = sender.clone();
    let reader_thread = std::thread::Builder::new()
        .name(format!("dsh-pty-reader-{pid}"))
        .spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let chunk = String::from_utf8_lossy(&buf[..n]).into_owned();
                        let _ = reader_sender.send(chunk);
                    }
                }
            }
        })
        .map_err(|error| {
            let _ = portable_pty::ChildKiller::kill(&mut *child);
            SubprocessError::Spawn(error.to_string())
        })?;

    let inner = Arc::new(HandleInner {
        pid,
        sender,
        starter_rx: std::sync::Mutex::new(Some(starter_rx)),
        outcome: tokio::sync::Mutex::new(None),
        notify: Notify::new(),
        writer: std::sync::Mutex::new(Some(writer)),
        master: std::sync::Mutex::new(Some(master)),
        reader: std::sync::Mutex::new(Some(reader_thread)),
        waiter: std::sync::Mutex::new(None),
    });

    let waiter_inner = Arc::downgrade(&inner);
    let waiter_thread = std::thread::Builder::new()
        .name(format!("dsh-pty-wait-{pid}"))
        .spawn(move || {
            let result = match child.wait() {
                Ok(status) => Ok(outcome_from_pty_status(status)),
                Err(error) => Err(SubprocessError::Spawn(error.to_string())),
            };
            if let Some(inner) = waiter_inner.upgrade() {
                *inner.outcome.blocking_lock() = Some(result);
                inner.notify.notify_waiters();
            }
        })
        .map_err(|error| SubprocessError::Spawn(error.to_string()))?;

    *inner
        .waiter
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(waiter_thread);

    Ok(SubprocessTerminalHandle { inner })
}

#[cfg(unix)]
fn outcome_from_pty_status(status: portable_pty::ExitStatus) -> SubprocessOutcome {
    match status.signal() {
        Some(_) => SubprocessOutcome {
            exit_code: None,
            signal: None,
        },
        None => SubprocessOutcome {
            exit_code: i32::try_from(status.exit_code()).ok(),
            signal: None,
        },
    }
}

#[cfg(unix)]
fn take_mutex<T>(mutex: &std::sync::Mutex<Option<T>>) -> Option<T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
}

#[cfg(unix)]
impl Drop for HandleInner {
    fn drop(&mut self) {
        if self.pid > 0 {
            // SAFETY: pid is this PTY child's pid; ESRCH/EPERM are ignored so Drop stays idempotent.
            unsafe {
                libc::kill(self.pid, libc::SIGHUP);
                libc::kill(self.pid, libc::SIGKILL);
            }
        }
        drop(take_mutex(&self.writer));
        drop(take_mutex(&self.master));
        let reader = take_mutex(&self.reader);
        let waiter = take_mutex(&self.waiter);
        if let Some(handle) = reader {
            let _ = handle.join();
        }
        if let Some(handle) = waiter {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SubprocessTerminalHandle, SubprocessTerminalSpawnSpec, spawn_terminal};
    use crate::SubprocessError;
    use std::path::PathBuf;
    use std::time::Duration;

    fn spec(argv: Vec<String>) -> Result<SubprocessTerminalSpawnSpec, SubprocessError> {
        SubprocessTerminalSpawnSpec::new(argv, PathBuf::from("/"), Vec::new(), 40, 160, 3_000)
    }

    #[test]
    fn spawn_terminal_rejects_empty_argv() {
        let err = spec(Vec::new()).unwrap_err();
        assert!(matches!(err, SubprocessError::InvalidArgv));
        let err = spec(vec![String::new()]).unwrap_err();
        assert!(matches!(err, SubprocessError::InvalidArgv));
        assert_eq!(
            SubprocessError::InvalidArgv.to_string(),
            "invalid argv: expected a non-empty program name at argv[0]"
        );
    }

    #[cfg(unix)]
    async fn lock_pty_tests() -> tokio::sync::MutexGuard<'static, ()> {
        static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        LOCK.lock().await
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_terminal_echo_hello_on_unix() {
        let _guard = lock_pty_tests().await;
        let spec = spec(vec!["/bin/sh".into(), "-c".into(), "printf hello".into()]).unwrap();
        let handle: SubprocessTerminalHandle = spawn_terminal(spec).unwrap();
        assert!(handle.pid() > 0);
        let mut output = handle.output();
        let mut collected = String::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !collected.contains("hello") {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                panic!("timed out waiting for hello, got {collected:?}");
            }
            match tokio::time::timeout(remaining, output.recv()).await {
                Err(_) => panic!("timed out waiting for hello, got {collected:?}"),
                Ok(Ok(chunk)) => collected.push_str(&chunk),
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {}
                Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => break,
            }
        }
        assert!(collected.contains("hello"), "{collected:?}");
        let outcome = tokio::time::timeout(Duration::from_secs(5), handle.done())
            .await
            .expect("done")
            .unwrap();
        assert_eq!(outcome.exit_code, Some(0));
        drop(handle);
    }

    #[cfg(not(unix))]
    #[test]
    fn spawn_terminal_unsupported() {
        let spec = spec(vec!["/bin/sh".into()]).unwrap();
        let err = spawn_terminal(spec).unwrap_err();
        assert!(matches!(err, SubprocessError::UnsupportedPlatform));
        assert_eq!(err.to_string(), "unsupported on this platform");
    }
}
