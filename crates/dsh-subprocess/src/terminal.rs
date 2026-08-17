//! POSIX PTY allocation via `portable-pty`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[cfg(unix)]
use std::collections::HashSet;
#[cfg(unix)]
use std::time::Duration;

use tokio::sync::Notify;

use crate::env::{EnvEntry, child_env};
use crate::error::SubprocessError;
#[cfg(unix)]
use crate::inspector::{ProcessIdentity, TermKill, create_process_inspector};
use crate::inspector::{ProcessInspector, SubprocessTerminalSignal};
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

/// Foreground process group currently owning a live PTY.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubprocessTerminalForeground {
    process_group_id: i32,
    input_waiting: bool,
}

impl SubprocessTerminalForeground {
    /// POSIX process-group id that currently owns the terminal.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The foreground `tpgid`.
    #[must_use]
    pub fn process_group_id(&self) -> i32 {
        self.process_group_id
    }

    /// Whether a member of that group is waiting on stdin.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// Linux `/proc` stdin-wait; macOS is always false.
    #[must_use]
    pub fn input_waiting(&self) -> bool {
        self.input_waiting
    }
}

/// Live POSIX PTY child. Cloning shares the same session via [`Arc`].
///
/// Last-handle drop closes the PTY master and signals the child unless `done` already recorded
/// the exit or [`Self::terminate`] already ran.
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

struct CleanupShared {
    started: AtomicBool,
    result: Mutex<Option<Result<(), SubprocessError>>>,
    notify: Notify,
}

struct HandleInner {
    pid: i32,
    sender: tokio::sync::broadcast::Sender<String>,
    starter_rx: Mutex<Option<tokio::sync::broadcast::Receiver<String>>>,
    outcome: Mutex<Option<Result<SubprocessOutcome, SubprocessError>>>,
    notify: Notify,
    exited: AtomicBool,
    terminate_started: AtomicBool,
    cleanup: tokio::sync::Mutex<Option<Arc<CleanupShared>>>,
    #[cfg(unix)]
    inspector: Arc<dyn ProcessInspector>,
    #[cfg(unix)]
    root_identity: Option<ProcessIdentity>,
    #[cfg(unix)]
    tracked_descendants: Mutex<Vec<ProcessIdentity>>,
    #[cfg(unix)]
    grace_ms: u64,
    #[cfg(unix)]
    writer: Mutex<Option<Box<dyn std::io::Write + Send>>>,
    #[cfg(unix)]
    master: Mutex<Option<Box<dyn portable_pty::MasterPty + Send>>>,
    #[cfg(unix)]
    reader: Mutex<Option<std::thread::JoinHandle<()>>>,
    #[cfg(unix)]
    waiter: Mutex<Option<std::thread::JoinHandle<()>>>,
    #[cfg(all(unix, test))]
    test_shell: Option<TestShell>,
}

#[cfg(all(unix, test))]
struct TestShell {
    kills: Mutex<Vec<String>>,
    auto_exit_on_kill: AtomicBool,
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
    /// [`SubprocessError::TerminalExited`] when the top-level child has already exited.
    /// [`SubprocessError::Spawn`] when the writer is closed or the OS write fails.
    pub async fn write(&self, data: &str) -> Result<(), SubprocessError> {
        #[cfg(not(unix))]
        {
            let _ = data;
            Err(SubprocessError::UnsupportedPlatform)
        }
        #[cfg(unix)]
        {
            if self.inner.exited.load(Ordering::SeqCst) {
                return Err(SubprocessError::TerminalExited);
            }
            #[cfg(test)]
            if self.inner.test_shell.is_some() {
                let _ = data;
                return Ok(());
            }
            use std::io::Write;
            let bytes = data.as_bytes().to_vec();
            let inner = Arc::clone(&self.inner);
            tokio::task::spawn_blocking(move || {
                if inner.exited.load(Ordering::SeqCst) {
                    return Err(SubprocessError::TerminalExited);
                }
                let mut guard = lock_std(&inner.writer);
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
        let mut slot = lock_std(&self.inner.starter_rx);
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
            if let Some(result) = lock_std(&self.inner.outcome).clone() {
                return result;
            }
            notified.await;
        }
    }

    /// Inspect the current foreground process group and stdin-wait state.
    ///
    /// Missing PGID is `Ok(None)`, matching TypeScript `undefined`. Cancel must use
    /// [`Self::signal_foreground`] with [`SubprocessTerminalSignal::Sigint`], never a PTY write of
    /// `\x03`.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The foreground group when `tpgid` is known.
    ///
    /// # Errors
    ///
    /// [`SubprocessError::UnsupportedPlatform`] on non-unix hosts.
    pub async fn inspect_foreground(
        &self,
    ) -> Result<Option<SubprocessTerminalForeground>, SubprocessError> {
        #[cfg(not(unix))]
        {
            let _ = self;
            Err(SubprocessError::UnsupportedPlatform)
        }
        #[cfg(unix)]
        {
            let inner = Arc::clone(&self.inner);
            tokio::task::spawn_blocking(move || inspect_foreground_sync(&inner))
                .await
                .map_err(|error| SubprocessError::Spawn(error.to_string()))?
        }
    }

    /// Deliver `signal` to the current foreground process group.
    ///
    /// # Parameters
    ///
    /// * `signal` - Group signal. `SIGKILL` of the shell pid's own group is refused.
    ///
    /// # Returns
    ///
    /// The process-group id that received the signal.
    ///
    /// # Errors
    ///
    /// [`SubprocessError::UnsupportedPlatform`] on non-unix hosts.
    /// [`SubprocessError::CannotResolveForeground`] when PGID is missing.
    /// [`SubprocessError::RefusingSigkillShell`] when `SIGKILL` targets the shell pid.
    pub async fn signal_foreground(
        &self,
        signal: SubprocessTerminalSignal,
    ) -> Result<i32, SubprocessError> {
        #[cfg(not(unix))]
        {
            let _ = (self, signal);
            Err(SubprocessError::UnsupportedPlatform)
        }
        #[cfg(unix)]
        {
            let inner = Arc::clone(&self.inner);
            tokio::task::spawn_blocking(move || signal_foreground_sync(&inner, signal))
                .await
                .map_err(|error| SubprocessError::Spawn(error.to_string()))?
        }
    }

    /// Tear down descendants then the top-level PTY child (TypeScript `closeOnce`).
    ///
    /// A second call awaits the same in-flight cleanup. A failed cleanup clears the slot so a
    /// later call retries.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// `Ok(())` when no live non-zombie identities remain.
    ///
    /// # Errors
    ///
    /// [`SubprocessError::UnsupportedPlatform`] on non-unix hosts.
    /// [`SubprocessError::TerminalCleanupSurvivingPids`] when a descendant survives SIGKILL.
    /// [`SubprocessError::TerminalCleanupSurvivingPid`] when the shell survives SIGKILL.
    pub async fn terminate(&self) -> Result<(), SubprocessError> {
        #[cfg(not(unix))]
        {
            let _ = self;
            Err(SubprocessError::UnsupportedPlatform)
        }
        #[cfg(unix)]
        {
            self.inner.terminate_started.store(true, Ordering::SeqCst);
            let shared = {
                let mut slot = self.inner.cleanup.lock().await;
                match slot.clone() {
                    Some(existing) => existing,
                    None => {
                        let created = Arc::new(CleanupShared {
                            started: AtomicBool::new(false),
                            result: Mutex::new(None),
                            notify: Notify::new(),
                        });
                        *slot = Some(Arc::clone(&created));
                        created
                    }
                }
            };
            if shared.started.swap(true, Ordering::SeqCst) {
                loop {
                    let notified = shared.notify.notified();
                    if let Some(result) = lock_std(&shared.result).clone() {
                        return result;
                    }
                    notified.await;
                }
            }
            let result = close_once(self).await;
            *lock_std(&shared.result) = Some(result.clone());
            shared.notify.notify_waiters();
            if result.is_err() {
                *self.inner.cleanup.lock().await = None;
            }
            result
        }
    }

    #[cfg(all(test, unix))]
    fn injected(pid: i32, inspector: Arc<dyn ProcessInspector>, grace_ms: u64) -> Self {
        let root_identity = inspector
            .process_tree(pid)
            .into_iter()
            .find(|member| member.pid() == pid);
        let (sender, starter_rx) = tokio::sync::broadcast::channel(OUTPUT_CAPACITY);
        Self {
            inner: Arc::new(HandleInner {
                pid,
                sender,
                starter_rx: Mutex::new(Some(starter_rx)),
                outcome: Mutex::new(None),
                notify: Notify::new(),
                exited: AtomicBool::new(false),
                terminate_started: AtomicBool::new(false),
                cleanup: tokio::sync::Mutex::new(None),
                inspector,
                root_identity,
                tracked_descendants: Mutex::new(Vec::new()),
                grace_ms,
                writer: Mutex::new(None),
                master: Mutex::new(None),
                reader: Mutex::new(None),
                waiter: Mutex::new(None),
                test_shell: Some(TestShell {
                    kills: Mutex::new(Vec::new()),
                    auto_exit_on_kill: AtomicBool::new(true),
                }),
            }),
        }
    }

    #[cfg(all(test, not(unix)))]
    fn unsupported_placeholder() -> Self {
        let (sender, starter_rx) = tokio::sync::broadcast::channel(OUTPUT_CAPACITY);
        Self {
            inner: Arc::new(HandleInner {
                pid: -1,
                sender,
                starter_rx: Mutex::new(Some(starter_rx)),
                outcome: Mutex::new(None),
                notify: Notify::new(),
                exited: AtomicBool::new(false),
                terminate_started: AtomicBool::new(false),
                cleanup: tokio::sync::Mutex::new(None),
            }),
        }
    }

    #[cfg(all(test, unix))]
    fn test_emit_exit(&self, outcome: SubprocessOutcome) {
        record_exit(&self.inner, Ok(outcome));
    }

    #[cfg(all(test, unix))]
    fn test_shell_kills(&self) -> Vec<String> {
        match &self.inner.test_shell {
            Some(shell) => lock_std(&shell.kills).clone(),
            None => Vec::new(),
        }
    }

    #[cfg(all(test, unix))]
    fn test_set_auto_exit_on_kill(&self, value: bool) {
        if let Some(shell) = &self.inner.test_shell {
            shell.auto_exit_on_kill.store(value, Ordering::SeqCst);
        }
    }
}

/// Allocate a POSIX PTY and spawn `spec.argv` as the session child.
///
/// Child environment is [`child_env`] after `env_clear` on the PTY command. Output is UTF-8
/// lossy `String` chunks on a [`tokio::sync::broadcast`] channel; see
/// [`SubprocessTerminalHandle::output`]. Unix spawn attaches [`create_process_inspector`].
///
/// # Parameters
///
/// * `spec` - Fully specified PTY spawn request from [`SubprocessTerminalSpawnSpec::new`].
///
/// # Returns
///
/// A handle to the live PTY. Last-handle drop closes the master and signals the child unless
/// `done` already recorded the exit or [`SubprocessTerminalHandle::terminate`] already ran.
///
/// # Errors
///
/// [`SubprocessError::UnsupportedPlatform`] on non-unix hosts.
/// [`SubprocessError::UnsupportedInspection`] when the host has no process inspector.
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
        let inspector: Arc<dyn ProcessInspector> = Arc::from(create_process_inspector()?);
        spawn_terminal_with_inspector(spec, inspector)
    }
}

/// Allocate a POSIX PTY using an injected [`ProcessInspector`].
///
/// # Parameters
///
/// * `spec` - Fully specified PTY spawn request from [`SubprocessTerminalSpawnSpec::new`].
/// * `inspector` - Process-table operations used for inspect, signal, and terminate.
///
/// # Returns
///
/// A handle sharing `inspector` for the session lifetime.
///
/// # Errors
///
/// [`SubprocessError::UnsupportedPlatform`] on non-unix hosts.
/// [`SubprocessError::Spawn`] when the OS refuses PTY allocation or spawn.
pub(crate) fn spawn_terminal_with_inspector(
    spec: SubprocessTerminalSpawnSpec,
    inspector: Arc<dyn ProcessInspector>,
) -> Result<SubprocessTerminalHandle, SubprocessError> {
    #[cfg(not(unix))]
    {
        let _ = (spec, inspector);
        Err(SubprocessError::UnsupportedPlatform)
    }
    #[cfg(unix)]
    {
        spawn_unix(spec, inspector)
    }
}

#[cfg(unix)]
fn spawn_unix(
    spec: SubprocessTerminalSpawnSpec,
    inspector: Arc<dyn ProcessInspector>,
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

    let root_identity = inspector
        .process_tree(pid)
        .into_iter()
        .find(|member| member.pid() == pid);

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
        starter_rx: Mutex::new(Some(starter_rx)),
        outcome: Mutex::new(None),
        notify: Notify::new(),
        exited: AtomicBool::new(false),
        terminate_started: AtomicBool::new(false),
        cleanup: tokio::sync::Mutex::new(None),
        inspector,
        root_identity,
        tracked_descendants: Mutex::new(Vec::new()),
        grace_ms: spec.grace_ms,
        writer: Mutex::new(Some(writer)),
        master: Mutex::new(Some(master)),
        reader: Mutex::new(Some(reader_thread)),
        waiter: Mutex::new(None),
        #[cfg(test)]
        test_shell: None,
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
                record_exit(&inner, result);
            }
        })
        .map_err(|error| SubprocessError::Spawn(error.to_string()))?;

    *lock_std(&inner.waiter) = Some(waiter_thread);

    Ok(SubprocessTerminalHandle { inner })
}

fn lock_std<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn record_exit(inner: &HandleInner, result: Result<SubprocessOutcome, SubprocessError>) {
    let mut outcome = lock_std(&inner.outcome);
    if outcome.is_some() {
        return;
    }
    *outcome = Some(result);
    inner.exited.store(true, Ordering::SeqCst);
    drop(outcome);
    inner.notify.notify_waiters();
}

#[cfg(unix)]
fn inspect_foreground_sync(
    inner: &HandleInner,
) -> Result<Option<SubprocessTerminalForeground>, SubprocessError> {
    descendants(inner);
    let Some(process_group_id) = inner.inspector.foreground_pgid(inner.pid) else {
        return Ok(None);
    };
    Ok(Some(SubprocessTerminalForeground {
        process_group_id,
        input_waiting: inner.inspector.is_stdin_waiting(process_group_id),
    }))
}

#[cfg(unix)]
fn signal_foreground_sync(
    inner: &HandleInner,
    signal: SubprocessTerminalSignal,
) -> Result<i32, SubprocessError> {
    descendants(inner);
    let Some(process_group_id) = inner.inspector.foreground_pgid(inner.pid) else {
        return Err(SubprocessError::CannotResolveForeground { pid: inner.pid });
    };
    if signal == SubprocessTerminalSignal::Sigkill && process_group_id == inner.pid {
        return Err(SubprocessError::RefusingSigkillShell);
    }
    inner.inspector.signal_group(process_group_id, signal);
    Ok(process_group_id)
}

#[cfg(unix)]
fn union_members(groups: &[Vec<ProcessIdentity>]) -> Vec<ProcessIdentity> {
    let mut members = Vec::new();
    let mut seen = HashSet::new();
    for group in groups {
        for member in group {
            let key = format!("{}:{}", member.pid(), member.started());
            if seen.contains(&key) {
                continue;
            }
            seen.insert(key);
            members.push(member.clone());
        }
    }
    members
}

#[cfg(unix)]
fn survivors(inner: &HandleInner, members: &[ProcessIdentity]) -> Vec<ProcessIdentity> {
    members
        .iter()
        .filter(|member| inner.inspector.is_alive(member))
        .cloned()
        .collect()
}

#[cfg(unix)]
fn descendants(inner: &HandleInner) -> Vec<ProcessIdentity> {
    let tree = inner.inspector.process_tree(inner.pid);
    let root = tree.iter().find(|member| member.pid() == inner.pid);
    let root_verified = match (&inner.root_identity, root) {
        (Some(identity), Some(current)) => current.started() == identity.started(),
        _ => false,
    };
    let tracked = lock_std(&inner.tracked_descendants).clone();
    let mut groups = vec![tracked];
    if root_verified {
        groups.push(tree);
        groups.push(inner.inspector.process_session(inner.pid));
    }
    let unioned = union_members(&groups);
    let filtered: Vec<ProcessIdentity> = unioned
        .into_iter()
        .filter(|member| member.pid() != inner.pid)
        .collect();
    let next = survivors(inner, &filtered);
    *lock_std(&inner.tracked_descendants) = next.clone();
    next
}

#[cfg(unix)]
fn signal_members(inner: &HandleInner, members: &[ProcessIdentity], signal: TermKill) {
    for member in members {
        inner.inspector.signal_process(member, signal);
    }
}

#[cfg(unix)]
async fn wait_for_members(
    inner: &HandleInner,
    members: &[ProcessIdentity],
) -> Vec<ProcessIdentity> {
    let until = tokio::time::Instant::now() + Duration::from_millis(inner.grace_ms);
    let mut live = survivors(inner, members);
    while !live.is_empty() {
        let now = tokio::time::Instant::now();
        if now >= until {
            break;
        }
        let remaining = until.saturating_duration_since(now);
        let sleep = remaining
            .min(Duration::from_millis(25))
            .max(Duration::from_millis(1));
        tokio::time::sleep(sleep).await;
        live = survivors(inner, members);
    }
    live
}

#[cfg(unix)]
async fn stop_descendants(inner: &HandleInner) -> Vec<ProcessIdentity> {
    let captured = descendants(inner);
    signal_members(inner, &captured, TermKill::Sigterm);
    let captured_survivors = wait_for_members(inner, &captured).await;
    let members = union_members(&[captured_survivors, descendants(inner)]);
    signal_members(inner, &members, TermKill::Sigkill);
    let live = wait_for_members(inner, &members).await;
    survivors(inner, &union_members(&[live, descendants(inner)]))
}

#[cfg(unix)]
fn kill_shell(inner: &HandleInner, signal: TermKill) {
    #[cfg(test)]
    if let Some(shell) = &inner.test_shell {
        let name = match signal {
            TermKill::Sigterm => "SIGTERM",
            TermKill::Sigkill => "SIGKILL",
        };
        lock_std(&shell.kills).push(name.to_string());
        if shell.auto_exit_on_kill.load(Ordering::SeqCst) {
            let code = match signal {
                TermKill::Sigkill => 9,
                TermKill::Sigterm => 15,
            };
            record_exit(
                inner,
                Ok(SubprocessOutcome {
                    exit_code: None,
                    signal: Some(code),
                }),
            );
        }
        return;
    }
    if inner.pid <= 0 {
        return;
    }
    let sig = match signal {
        TermKill::Sigterm => libc::SIGTERM,
        TermKill::Sigkill => libc::SIGKILL,
    };
    // SAFETY: pid is this PTY child's pid; ESRCH/EPERM are ignored so teardown stays idempotent.
    unsafe {
        libc::kill(inner.pid, sig);
    }
}

#[cfg(unix)]
async fn wait_until_exited(inner: &HandleInner) {
    loop {
        let notified = inner.notify.notified();
        if inner.exited.load(Ordering::SeqCst) {
            return;
        }
        notified.await;
    }
}

#[cfg(unix)]
async fn wait_exit_or_grace(inner: &HandleInner) {
    tokio::select! {
        () = wait_until_exited(inner) => {}
        () = tokio::time::sleep(Duration::from_millis(inner.grace_ms)) => {}
    }
}

#[cfg(unix)]
async fn stop_shell(inner: &HandleInner) -> Result<(), SubprocessError> {
    if !inner.exited.load(Ordering::SeqCst) {
        kill_shell(inner, TermKill::Sigterm);
        wait_exit_or_grace(inner).await;
    }
    if !inner.exited.load(Ordering::SeqCst) {
        kill_shell(inner, TermKill::Sigkill);
        wait_exit_or_grace(inner).await;
    }
    if !inner.exited.load(Ordering::SeqCst) {
        return Err(SubprocessError::TerminalCleanupSurvivingPid { pid: inner.pid });
    }
    Ok(())
}

#[cfg(unix)]
fn surviving_pids_error(survivors: &[ProcessIdentity]) -> SubprocessError {
    let pids = survivors
        .iter()
        .map(|member| member.pid().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    SubprocessError::TerminalCleanupSurvivingPids { pids }
}

#[cfg(unix)]
async fn close_once(handle: &SubprocessTerminalHandle) -> Result<(), SubprocessError> {
    let inner = handle.inner.as_ref();
    let live = stop_descendants(inner).await;
    if !live.is_empty() {
        return Err(surviving_pids_error(&live));
    }
    stop_shell(inner).await?;
    let live = stop_descendants(inner).await;
    if !live.is_empty() {
        return Err(surviving_pids_error(&live));
    }
    Ok(())
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
fn take_mutex<T>(mutex: &Mutex<Option<T>>) -> Option<T> {
    lock_std(mutex).take()
}

#[cfg(unix)]
impl Drop for HandleInner {
    fn drop(&mut self) {
        let skip_kill =
            self.exited.load(Ordering::SeqCst) || self.terminate_started.load(Ordering::SeqCst);
        #[cfg(test)]
        let skip_kill = skip_kill || self.test_shell.is_some();
        if !skip_kill && self.pid > 0 {
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

#[cfg(all(test, unix))]
pub(crate) async fn lock_pty_tests() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    LOCK.lock().await
}

#[cfg(test)]
mod tests {
    use super::{SubprocessTerminalHandle, SubprocessTerminalSpawnSpec, spawn_terminal};
    use crate::SubprocessError;
    use crate::inspector::{
        ProcessIdentity, ProcessInspector, SubprocessTerminalSignal, TermKill,
        create_process_inspector,
    };
    use crate::types::SubprocessOutcome;
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    fn spec(argv: Vec<String>) -> Result<SubprocessTerminalSpawnSpec, SubprocessError> {
        SubprocessTerminalSpawnSpec::new(argv, PathBuf::from("/"), Vec::new(), 40, 160, 3_000)
    }

    fn lock_poison<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        mutex
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    struct FakeInspector {
        pgid: Mutex<Option<i32>>,
        waiting: AtomicBool,
        root: Mutex<Option<ProcessIdentity>>,
        members: Mutex<Vec<ProcessIdentity>>,
        session_members: Mutex<Vec<ProcessIdentity>>,
        alive: Mutex<HashSet<i32>>,
        groups: Mutex<Vec<(i32, SubprocessTerminalSignal)>>,
        processes: Mutex<Vec<(i32, TermKill)>>,
        remove_on_signal: AtomicBool,
        tree_calls: AtomicUsize,
        tree_script: Mutex<Option<Vec<Vec<ProcessIdentity>>>>,
    }

    impl FakeInspector {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                pgid: Mutex::new(Some(456)),
                waiting: AtomicBool::new(false),
                root: Mutex::new(Some(ProcessIdentity::new(123, "shell"))),
                members: Mutex::new(Vec::new()),
                session_members: Mutex::new(Vec::new()),
                alive: Mutex::new(HashSet::new()),
                groups: Mutex::new(Vec::new()),
                processes: Mutex::new(Vec::new()),
                remove_on_signal: AtomicBool::new(true),
                tree_calls: AtomicUsize::new(0),
                tree_script: Mutex::new(None),
            })
        }

        fn set_pgid(&self, pgid: Option<i32>) {
            *lock_poison(&self.pgid) = pgid;
        }

        fn set_waiting(&self, waiting: bool) {
            self.waiting.store(waiting, Ordering::SeqCst);
        }

        fn set_root(&self, root: Option<ProcessIdentity>) {
            *lock_poison(&self.root) = root;
        }

        fn set_members(&self, members: Vec<ProcessIdentity>) {
            *lock_poison(&self.members) = members;
        }

        fn set_session_members(&self, members: Vec<ProcessIdentity>) {
            *lock_poison(&self.session_members) = members;
        }

        fn alive_insert(&self, pid: i32) {
            lock_poison(&self.alive).insert(pid);
        }

        fn alive_remove(&self, pid: i32) {
            lock_poison(&self.alive).remove(&pid);
        }

        fn set_remove_on_signal(&self, value: bool) {
            self.remove_on_signal.store(value, Ordering::SeqCst);
        }

        fn set_tree_script(&self, script: Vec<Vec<ProcessIdentity>>) {
            *lock_poison(&self.tree_script) = Some(script);
        }

        fn groups(&self) -> Vec<(i32, SubprocessTerminalSignal)> {
            lock_poison(&self.groups).clone()
        }

        fn processes(&self) -> Vec<(i32, TermKill)> {
            lock_poison(&self.processes).clone()
        }
    }

    impl ProcessInspector for FakeInspector {
        fn foreground_pgid(&self, _shell_pid: i32) -> Option<i32> {
            *lock_poison(&self.pgid)
        }

        fn is_stdin_waiting(&self, _pgid: i32) -> bool {
            self.waiting.load(Ordering::SeqCst)
        }

        fn process_tree(&self, _root_pid: i32) -> Vec<ProcessIdentity> {
            let call = self.tree_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(script) = lock_poison(&self.tree_script).as_ref() {
                return script.get(call).cloned().unwrap_or_default();
            }
            match lock_poison(&self.root).clone() {
                None => lock_poison(&self.members).clone(),
                Some(root) => {
                    let mut tree = vec![root];
                    tree.extend(lock_poison(&self.members).clone());
                    tree
                }
            }
        }

        fn process_session(&self, _session_id: i32) -> Vec<ProcessIdentity> {
            lock_poison(&self.session_members).clone()
        }

        fn is_alive(&self, identity: &ProcessIdentity) -> bool {
            lock_poison(&self.alive).contains(&identity.pid())
        }

        fn signal_group(&self, pgid: i32, signal: SubprocessTerminalSignal) {
            lock_poison(&self.groups).push((pgid, signal));
        }

        fn signal_process(&self, identity: &ProcessIdentity, signal: TermKill) {
            if !self.is_alive(identity) {
                return;
            }
            lock_poison(&self.processes).push((identity.pid(), signal));
            if self.remove_on_signal.load(Ordering::SeqCst) {
                lock_poison(&self.alive).remove(&identity.pid());
            }
        }
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
    #[tokio::test]
    async fn spawn_terminal_echo_hello_on_unix() {
        let _guard = super::lock_pty_tests().await;
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

    #[cfg(unix)]
    #[tokio::test]
    async fn refuses_sigkill_of_shell_pgid() {
        let fake = FakeInspector::new();
        fake.set_pgid(Some(123));
        let handle = SubprocessTerminalHandle::injected(123, Arc::clone(&fake) as Arc<_>, 10);
        let err = handle
            .signal_foreground(SubprocessTerminalSignal::Sigkill)
            .await
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "refusing to SIGKILL the terminal shell; terminate the terminal session instead"
        );
        assert!(fake.groups().is_empty());

        fake.set_pgid(Some(456));
        let pgid = handle
            .signal_foreground(SubprocessTerminalSignal::Sigint)
            .await
            .unwrap();
        assert_eq!(pgid, 456);
        assert_eq!(fake.groups(), vec![(456, SubprocessTerminalSignal::Sigint)]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn terminate_reaps_sleep_child() {
        let _guard = super::lock_pty_tests().await;
        let spec = spec(vec!["/bin/sh".into(), "-c".into(), "sleep 60; true".into()]).unwrap();
        let handle = spawn_terminal(spec).unwrap();
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
        let sleep_identity = sleep_identity.expect("sleep descendant of /bin/sh -c");
        handle.terminate().await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), handle.done())
            .await
            .expect("done")
            .unwrap();
        assert!(
            !inspector.is_alive(&sleep_identity),
            "sleep pid {} still live",
            sleep_identity.pid()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn inspect_foreground_returns_none_when_pgid_missing() {
        let fake = FakeInspector::new();
        fake.set_pgid(None);
        let handle = SubprocessTerminalHandle::injected(123, Arc::clone(&fake) as Arc<_>, 10);
        assert!(handle.inspect_foreground().await.unwrap().is_none());
        fake.set_pgid(Some(456));
        fake.set_waiting(true);
        let foreground = handle.inspect_foreground().await.unwrap().expect("pgid");
        assert_eq!(foreground.process_group_id(), 456);
        assert!(foreground.input_waiting());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn inspect_foreground_missing_pgid_does_not_signal() {
        let fake = FakeInspector::new();
        fake.set_pgid(None);
        let handle = SubprocessTerminalHandle::injected(123, Arc::clone(&fake) as Arc<_>, 10);
        let err = handle
            .signal_foreground(SubprocessTerminalSignal::Sigterm)
            .await
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "cannot resolve foreground process group for terminal 123"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn write_after_exit_reports_terminal_process_has_exited() {
        let fake = FakeInspector::new();
        let handle = SubprocessTerminalHandle::injected(123, Arc::clone(&fake) as Arc<_>, 10);
        handle.test_emit_exit(SubprocessOutcome {
            exit_code: Some(3),
            signal: None,
        });
        let err = handle.write("late").await.unwrap_err();
        assert_eq!(err.to_string(), "terminal process has exited");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn terminate_sigterm_then_sigkill_descendants() {
        let fake = FakeInspector::new();
        fake.set_members(vec![ProcessIdentity::new(124, "child")]);
        fake.alive_insert(124);
        fake.set_remove_on_signal(false);
        let handle = SubprocessTerminalHandle::injected(123, Arc::clone(&fake) as Arc<_>, 20);
        let pending = {
            let handle = handle.clone();
            tokio::spawn(async move { handle.terminate().await })
        };
        tokio::time::sleep(Duration::from_millis(25)).await;
        let processes = fake.processes();
        assert!(
            processes.contains(&(124, TermKill::Sigkill)),
            "expected SIGKILL after SIGTERM grace, got {processes:?}"
        );
        assert!(handle.test_shell_kills().is_empty());
        fake.alive_remove(124);
        pending.await.expect("join").unwrap();
        assert_eq!(handle.test_shell_kills(), vec!["SIGTERM".to_string()]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn terminate_does_not_adopt_recycled_shell_children() {
        let fake = FakeInspector::new();
        let handle = SubprocessTerminalHandle::injected(123, Arc::clone(&fake) as Arc<_>, 10);
        handle.test_emit_exit(SubprocessOutcome {
            exit_code: Some(0),
            signal: None,
        });
        fake.set_root(Some(ProcessIdentity::new(123, "imposter")));
        fake.set_members(vec![ProcessIdentity::new(999, "imposter-child")]);
        fake.alive_insert(999);
        handle.terminate().await.unwrap();
        assert!(fake.processes().is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn terminate_rescans_descendants_forked_during_term() {
        let fake = FakeInspector::new();
        let root = ProcessIdentity::new(123, "shell");
        fake.set_tree_script(vec![
            vec![root.clone()],
            vec![root.clone(), ProcessIdentity::new(124, "first")],
            vec![root.clone(), ProcessIdentity::new(125, "late")],
        ]);
        let handle = SubprocessTerminalHandle::injected(123, Arc::clone(&fake) as Arc<_>, 10);
        fake.alive_insert(124);
        fake.alive_insert(125);
        handle.terminate().await.unwrap();
        assert_eq!(
            fake.processes(),
            vec![(124, TermKill::Sigterm), (125, TermKill::Sigkill)]
        );
        assert_eq!(handle.test_shell_kills(), vec!["SIGTERM".to_string()]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn terminate_reports_surviving_pids() {
        let fake = FakeInspector::new();
        fake.set_members(vec![ProcessIdentity::new(124, "child")]);
        fake.alive_insert(124);
        fake.set_remove_on_signal(false);
        let handle = SubprocessTerminalHandle::injected(123, Arc::clone(&fake) as Arc<_>, 5);
        let err = handle.terminate().await.unwrap_err();
        assert_eq!(
            err.to_string(),
            "terminal cleanup failed; surviving pids: 124"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn terminate_reports_surviving_shell_pid() {
        let fake = FakeInspector::new();
        let handle = SubprocessTerminalHandle::injected(123, Arc::clone(&fake) as Arc<_>, 5);
        handle.test_set_auto_exit_on_kill(false);
        let err = handle.terminate().await.unwrap_err();
        assert_eq!(
            err.to_string(),
            "terminal cleanup failed; surviving pid: 123"
        );
        assert_eq!(
            handle.test_shell_kills(),
            vec!["SIGTERM".to_string(), "SIGKILL".to_string()]
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn terminate_cleans_same_session_descendant_after_shell_exit() {
        let fake = FakeInspector::new();
        let disowned = ProcessIdentity::new(124, "disowned");
        fake.set_session_members(vec![disowned]);
        fake.alive_insert(124);
        let handle = SubprocessTerminalHandle::injected(123, Arc::clone(&fake) as Arc<_>, 20);
        handle.test_emit_exit(SubprocessOutcome {
            exit_code: Some(0),
            signal: None,
        });
        handle.terminate().await.unwrap();
        assert_eq!(fake.processes(), vec![(124, TermKill::Sigterm)]);
    }

    #[cfg(not(unix))]
    #[tokio::test]
    async fn inspect_signal_terminate_unsupported() {
        let handle = SubprocessTerminalHandle::unsupported_placeholder();
        assert!(matches!(
            handle.inspect_foreground().await,
            Err(SubprocessError::UnsupportedPlatform)
        ));
        assert!(matches!(
            handle
                .signal_foreground(SubprocessTerminalSignal::Sigint)
                .await,
            Err(SubprocessError::UnsupportedPlatform)
        ));
        assert!(matches!(
            handle.terminate().await,
            Err(SubprocessError::UnsupportedPlatform)
        ));
    }
}
