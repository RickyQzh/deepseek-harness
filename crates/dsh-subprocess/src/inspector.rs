//! Platform process-table inspection for PTY foreground groups, stdin-wait, and teardown identity.
//!
//! Linux reads `/proc/<pid>/stat`, `/proc/<pid>/task/<tid>/syscall`, `/proc/<pid>/mem`, and
//! epoll fdinfo. macOS reads `ps` (`tpgid=` and `pid=,ppid=,lstart=`). Unreadable process
//! memory is not a stdin wait. macOS [`ProcessInspector::is_stdin_waiting`] is always false.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, ErrorKind};
use std::sync::{Arc, Mutex};

use crate::error::SubprocessError;

/// PID plus start identity, preventing teardown escalation after PID reuse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessIdentity {
    pid: i32,
    started: String,
}

impl ProcessIdentity {
    /// Construct a pid plus start-identity fence.
    ///
    /// # Parameters
    ///
    /// * `pid` - OS process id.
    /// * `started` - Linux `/proc/<pid>/stat` starttime ticks, or macOS `ps` `lstart` text.
    ///
    /// # Returns
    ///
    /// The identity used by [`ProcessInspector::is_alive`] and [`ProcessInspector::signal_process`].
    #[must_use]
    pub fn new(pid: i32, started: impl Into<String>) -> Self {
        Self {
            pid,
            started: started.into(),
        }
    }

    /// OS process id.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The pid passed to [`Self::new`].
    #[must_use]
    pub fn pid(&self) -> i32 {
        self.pid
    }

    /// Start identity fence.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// Linux starttime ticks or macOS `lstart` text.
    #[must_use]
    pub fn started(&self) -> &str {
        &self.started
    }
}

/// Signals a terminal foreground group may receive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubprocessTerminalSignal {
    /// `SIGINT`
    Sigint,
    /// `SIGTERM`
    Sigterm,
    /// `SIGKILL`
    Sigkill,
    /// `SIGTSTP`
    Sigtstp,
    /// `SIGHUP`
    Sighup,
}

/// Escalation signals sent to one identity, never to a whole group through this type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TermKill {
    /// `SIGTERM`
    Sigterm,
    /// `SIGKILL`
    Sigkill,
}

fn terminal_signal_name(signal: SubprocessTerminalSignal) -> &'static str {
    match signal {
        SubprocessTerminalSignal::Sigint => "SIGINT",
        SubprocessTerminalSignal::Sigterm => "SIGTERM",
        SubprocessTerminalSignal::Sigkill => "SIGKILL",
        SubprocessTerminalSignal::Sigtstp => "SIGTSTP",
        SubprocessTerminalSignal::Sighup => "SIGHUP",
    }
}

fn term_kill_name(signal: TermKill) -> &'static str {
    match signal {
        TermKill::Sigterm => "SIGTERM",
        TermKill::Sigkill => "SIGKILL",
    }
}

/// Fields used from Linux `/proc/<pid>/stat`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcStat {
    pid: i32,
    parent_pid: i32,
    pgrp: i32,
    session: i32,
    state: char,
    tpgid: i32,
    started: String,
}

impl ProcStat {
    /// Process id from the stat prefix.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The parsed pid.
    #[must_use]
    pub fn pid(&self) -> i32 {
        self.pid
    }

    /// Parent pid (`ppid`).
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The parsed parent pid.
    #[must_use]
    pub fn parent_pid(&self) -> i32 {
        self.parent_pid
    }

    /// Process-group id (`pgrp`).
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The parsed process-group id.
    #[must_use]
    pub fn pgrp(&self) -> i32 {
        self.pgrp
    }

    /// Session id.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The parsed session id.
    #[must_use]
    pub fn session(&self) -> i32 {
        self.session
    }

    /// Single-character run state (`R`, `S`, `Z`, …).
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The parsed state character.
    #[must_use]
    pub fn state(&self) -> char {
        self.state
    }

    /// Foreground process-group id of the controlling terminal (`tpgid`).
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The parsed `tpgid`, which may be `-1` when the process has no controlling terminal.
    #[must_use]
    pub fn tpgid(&self) -> i32 {
        self.tpgid
    }

    /// Starttime ticks from `/proc/<pid>/stat`.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The raw starttime field string.
    #[must_use]
    pub fn started(&self) -> &str {
        &self.started
    }
}

/// Testable filesystem, process-table, and signal operations used by one inspector.
pub trait ProcessInspectorInternals: Send + Sync {
    /// Read one file as UTF-8 text.
    ///
    /// # Parameters
    ///
    /// * `path` - Absolute path, typically under `/proc`.
    ///
    /// # Returns
    ///
    /// The file contents.
    ///
    /// # Errors
    ///
    /// When the path cannot be read as UTF-8 text.
    fn read_file(&self, path: &str) -> Result<String, io::Error>;

    /// List directory entry names (not full paths).
    ///
    /// # Parameters
    ///
    /// * `path` - Absolute directory path.
    ///
    /// # Returns
    ///
    /// Entry names, including non-numeric `/proc` names.
    ///
    /// # Errors
    ///
    /// When the directory cannot be read.
    fn read_dir(&self, path: &str) -> Result<Vec<String>, io::Error>;

    /// Open a file for positioned reads of `/proc/<pid>/mem`.
    ///
    /// # Parameters
    ///
    /// * `path` - Absolute path to open.
    ///
    /// # Returns
    ///
    /// An inspector-local file descriptor.
    ///
    /// # Errors
    ///
    /// When the path cannot be opened.
    fn open(&self, path: &str) -> Result<i32, io::Error>;

    /// Read `buffer.len()` bytes at `position`, like `pread`.
    ///
    /// # Parameters
    ///
    /// * `fd` - Descriptor from [`Self::open`].
    /// * `buffer` - Destination; length is the requested count.
    /// * `position` - Byte offset, including a process virtual address for `/proc/<pid>/mem`.
    ///
    /// # Returns
    ///
    /// Bytes copied into `buffer`.
    ///
    /// # Errors
    ///
    /// When `fd` is unknown or the read fails.
    fn read(&self, fd: i32, buffer: &mut [u8], position: u64) -> Result<usize, io::Error>;

    /// Close a descriptor from [`Self::open`]. Missing fds are ignored.
    ///
    /// # Parameters
    ///
    /// * `fd` - Descriptor to close.
    ///
    /// # Returns
    ///
    /// None.
    fn close(&self, fd: i32);

    /// Run `file` with `args` and return UTF-8 stdout.
    ///
    /// # Parameters
    ///
    /// * `file` - Executable path.
    /// * `args` - Arguments, not a shell string.
    ///
    /// # Returns
    ///
    /// Stdout text.
    ///
    /// # Errors
    ///
    /// When spawn fails or the process exits unsuccessfully.
    fn exec(&self, file: &str, args: &[&str]) -> Result<String, io::Error>;

    /// Deliver `signal` to `pid`. Negative `pid` means a process group, as with `kill(2)`.
    ///
    /// # Parameters
    ///
    /// * `pid` - Process id or negated process-group id.
    /// * `signal` - POSIX name such as `SIGINT`.
    ///
    /// # Returns
    ///
    /// None.
    fn kill(&self, pid: i32, signal: &str);
}

/// Platform process-table inspection for terminal readiness, signals, and teardown.
pub trait ProcessInspector: Send + Sync {
    /// Foreground process-group id for a terminal shell pid.
    ///
    /// # Parameters
    ///
    /// * `shell_pid` - Top-level terminal process id.
    ///
    /// # Returns
    ///
    /// `tpgid` when it is greater than zero.
    fn foreground_pgid(&self, shell_pid: i32) -> Option<i32>;

    /// Whether any member of `pgid` is waiting on stdin.
    ///
    /// # Parameters
    ///
    /// * `pgid` - POSIX process-group id.
    ///
    /// # Returns
    ///
    /// `true` only when a Linux syscall table proves a stdin wait. macOS is always `false`.
    fn is_stdin_waiting(&self, pgid: i32) -> bool;

    /// Return `root_pid` and its current transitive descendants, children first.
    ///
    /// # Parameters
    ///
    /// * `root_pid` - Tree root pid.
    ///
    /// # Returns
    ///
    /// Identities in post-order, or empty when `root_pid` is absent. Cycles are skipped after
    /// the first visit.
    fn process_tree(&self, root_pid: i32) -> Vec<ProcessIdentity>;

    /// Current members of one POSIX process session when the platform exposes them.
    ///
    /// # Parameters
    ///
    /// * `session_id` - Session id from Linux `/proc/<pid>/stat`.
    ///
    /// # Returns
    ///
    /// Linux members of that session. macOS always returns an empty vec.
    fn process_session(&self, session_id: i32) -> Vec<ProcessIdentity>;

    /// Whether the exact identity remains a non-quiescent process.
    ///
    /// # Parameters
    ///
    /// * `identity` - Pid plus start fence.
    ///
    /// # Returns
    ///
    /// Linux: starttime matches and state is not `Z`/`X`/`x`. macOS: pid and `lstart` still appear
    /// in `ps`.
    fn is_alive(&self, identity: &ProcessIdentity) -> bool;

    /// Send `signal` to POSIX process group `-pgid`.
    ///
    /// # Parameters
    ///
    /// * `pgid` - Process-group id.
    /// * `signal` - Terminal signal name.
    ///
    /// # Returns
    ///
    /// None.
    fn signal_group(&self, pgid: i32, signal: SubprocessTerminalSignal);

    /// Send `SIGTERM` or `SIGKILL` to `identity` when [`Self::is_alive`] is true.
    ///
    /// # Parameters
    ///
    /// * `identity` - Pid plus start fence.
    /// * `signal` - Escalation signal.
    ///
    /// # Returns
    ///
    /// None.
    fn signal_process(&self, identity: &ProcessIdentity, signal: TermKill);
}

/// Parse fields used from Linux `/proc/<pid>/stat`, including parenthesized comm text.
///
/// # Parameters
///
/// * `text` - Complete stat line.
///
/// # Returns
///
/// Parsed identity and group fields, or `None` for malformed input.
#[must_use]
pub fn parse_proc_stat(text: &str) -> Option<ProcStat> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    if open == 0 || close <= open {
        return None;
    }
    let pid: i32 = text.get(..open)?.trim().parse().ok()?;
    let rest_text = text.get(close.checked_add(2)?..).unwrap_or("").trim();
    let rest: Vec<&str> = rest_text.split_whitespace().collect();
    let state = *rest.first()?;
    if state.len() != 1 {
        return None;
    }
    let state = state.chars().next()?;
    let parent_pid: i32 = rest.get(1)?.parse().ok()?;
    let pgrp: i32 = rest.get(2)?.parse().ok()?;
    let session: i32 = rest.get(3)?.parse().ok()?;
    let tpgid: i32 = rest.get(5)?.parse().ok()?;
    let started = (*rest.get(19)?).to_string();
    Some(ProcStat {
        pid,
        parent_pid,
        pgrp,
        session,
        state,
        tpgid,
        started,
    })
}

fn is_numeric_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_digit())
}

fn read_linux_stat(internals: &dyn ProcessInspectorInternals, pid: i32) -> Option<ProcStat> {
    let text = internals.read_file(&format!("/proc/{pid}/stat")).ok()?;
    parse_proc_stat(&text)
}

fn is_quiescent_state(state: char) -> bool {
    matches!(state, 'Z' | 'X' | 'x')
}

/// Report whether a Linux process group has an executing member.
///
/// `Some(false)` means the group contains only zombie/dead entries. `None` means the process
/// table could not prove either outcome.
///
/// # Parameters
///
/// * `process_group_id` - POSIX process-group id to inspect.
/// * `internals` - Injectable process-table operations.
///
/// # Returns
///
/// Live-member presence, or `None` when `/proc` is unreadable or no matching pids exist.
#[must_use]
pub fn linux_process_group_has_live_members(
    process_group_id: i32,
    internals: &dyn ProcessInspectorInternals,
) -> Option<bool> {
    let entries = match internals.read_dir("/proc") {
        Ok(entries) => entries,
        Err(_) => return None,
    };
    let mut matched = false;
    for entry in entries {
        if !is_numeric_name(&entry) {
            continue;
        }
        let Ok(pid) = entry.parse::<i32>() else {
            continue;
        };
        let Some(stat) = read_linux_stat(internals, pid) else {
            continue;
        };
        if stat.pgrp != process_group_id {
            continue;
        }
        matched = true;
        if !is_quiescent_state(stat.state) {
            return Some(true);
        }
    }
    if matched { Some(false) } else { None }
}

fn numeric_entries(internals: &dyn ProcessInspectorInternals, path: &str) -> Vec<i32> {
    let Ok(entries) = internals.read_dir(path) else {
        return Vec::new();
    };
    entries
        .into_iter()
        .filter(|entry| is_numeric_name(entry))
        .filter_map(|entry| entry.parse().ok())
        .collect()
}

struct SyscallInfo {
    number: i64,
    args: Vec<i64>,
}

fn parse_hex_i64(text: &str) -> Option<i64> {
    let text = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .unwrap_or(text);
    if let Ok(value) = i64::from_str_radix(text, 16) {
        return Some(value);
    }
    u64::from_str_radix(text, 16).ok().map(|value| value as i64)
}

fn read_syscall(
    internals: &dyn ProcessInspectorInternals,
    pid: i32,
    tid: i32,
) -> Option<SyscallInfo> {
    let text = internals
        .read_file(&format!("/proc/{pid}/task/{tid}/syscall"))
        .ok()?;
    let text = text.trim();
    if text == "running" || text.starts_with("-1 ") {
        return None;
    }
    let mut fields = text.split_whitespace();
    let number: i64 = fields.next()?.parse().ok()?;
    let mut args = Vec::new();
    for field in fields.take(6) {
        let value = parse_hex_i64(field)?;
        args.push(value);
    }
    Some(SyscallInfo { number, args })
}

fn read_memory(
    internals: &dyn ProcessInspectorInternals,
    pid: i32,
    address: i64,
    length: usize,
) -> Option<Vec<u8>> {
    let fd = internals.open(&format!("/proc/{pid}/mem")).ok()?;
    let mut buffer = vec![0u8; length];
    let result = internals.read(fd, &mut buffer, address as u64);
    internals.close(fd);
    match result {
        Ok(count) => {
            buffer.truncate(count);
            Some(buffer)
        }
        Err(_) => None,
    }
}

fn fd_set_has_stdin(internals: &dyn ProcessInspectorInternals, pid: i32, address: i64) -> bool {
    if address == 0 {
        return false;
    }
    let Some(memory) = read_memory(internals, pid, address, 8) else {
        return false;
    };
    let byte = memory.first().copied().unwrap_or(0);
    byte % 2 == 1
}

fn poll_has_stdin(
    internals: &dyn ProcessInspectorInternals,
    pid: i32,
    address: i64,
    count: i64,
) -> bool {
    if address == 0 || count <= 0 {
        return false;
    }
    let capped = count.min(1024);
    let Ok(capped_usize) = usize::try_from(capped) else {
        return false;
    };
    let length = capped_usize.saturating_mul(8);
    let Some(memory) = read_memory(internals, pid, address, length) else {
        return false;
    };
    let mut offset: usize = 0;
    while offset.saturating_add(8) <= memory.len() {
        let fd_bytes: [u8; 4] = match memory[offset..offset + 4].try_into() {
            Ok(bytes) => bytes,
            Err(_) => break,
        };
        let event_bytes: [u8; 2] = match memory[offset + 4..offset + 6].try_into() {
            Ok(bytes) => bytes,
            Err(_) => break,
        };
        let fd = i32::from_le_bytes(fd_bytes);
        let events = i16::from_le_bytes(event_bytes);
        if fd == 0 && (events & 0x001) != 0 {
            return true;
        }
        offset += 8;
    }
    false
}

fn tfd_is_stdin(line: &str) -> bool {
    let trimmed = line.trim();
    let Some(after_colon) = trimmed.strip_prefix("tfd:") else {
        return false;
    };
    let Some(first) = after_colon.chars().next() else {
        return false;
    };
    if !first.is_whitespace() {
        return false;
    }
    let rest = after_colon.trim_start();
    let Some(after_zero) = rest.strip_prefix('0') else {
        return false;
    };
    after_zero.is_empty()
        || after_zero.starts_with(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
}

fn epoll_has_stdin(internals: &dyn ProcessInspectorInternals, pid: i32, epfd: i64) -> bool {
    let Ok(text) = internals.read_file(&format!("/proc/{pid}/fdinfo/{epfd}")) else {
        return false;
    };
    text.split('\n').any(tfd_is_stdin)
}

#[derive(Clone, Copy)]
struct SyscallTable {
    read: i64,
    select: Option<i64>,
    pselect: i64,
    poll: Option<i64>,
    ppoll: i64,
    epoll_wait: Option<i64>,
    epoll_pwait: i64,
}

const X86_64_SYSCALLS: SyscallTable = SyscallTable {
    read: 0,
    select: Some(23),
    pselect: 270,
    poll: Some(7),
    ppoll: 271,
    epoll_wait: Some(232),
    epoll_pwait: 281,
};

const AARCH64_SYSCALLS: SyscallTable = SyscallTable {
    read: 63,
    select: None,
    pselect: 72,
    poll: None,
    ppoll: 73,
    epoll_wait: None,
    epoll_pwait: 22,
};

fn syscall_table(arch: &str) -> Option<SyscallTable> {
    match arch {
        "x86_64" | "x64" => Some(X86_64_SYSCALLS),
        "aarch64" | "arm64" => Some(AARCH64_SYSCALLS),
        _ => None,
    }
}

fn syscall_arg(syscall: &SyscallInfo, index: usize) -> i64 {
    syscall.args.get(index).copied().unwrap_or(0)
}

fn syscall_waits_on_stdin(
    internals: &dyn ProcessInspectorInternals,
    pid: i32,
    syscall: &SyscallInfo,
    table: SyscallTable,
) -> bool {
    let a0 = syscall_arg(syscall, 0);
    let a1 = syscall_arg(syscall, 1);
    let a2 = syscall_arg(syscall, 2);
    if syscall.number == table.read {
        return a0 == 0;
    }
    if table.select == Some(syscall.number) || syscall.number == table.pselect {
        return a0 >= 1 && fd_set_has_stdin(internals, pid, a1);
    }
    if table.poll == Some(syscall.number) || syscall.number == table.ppoll {
        return a1 >= 1 && poll_has_stdin(internals, pid, a0, a1);
    }
    if table.epoll_wait == Some(syscall.number) || syscall.number == table.epoll_pwait {
        return a2 >= 1 && epoll_has_stdin(internals, pid, a0);
    }
    false
}

struct ProcessTreeEntry {
    pid: i32,
    parent_pid: i32,
    started: String,
}

fn visit_tree(
    entry: &ProcessTreeEntry,
    by_parent: &HashMap<i32, Vec<&ProcessTreeEntry>>,
    visited: &mut HashSet<i32>,
    result: &mut Vec<ProcessIdentity>,
) {
    if !visited.insert(entry.pid) {
        return;
    }
    if let Some(children) = by_parent.get(&entry.pid) {
        for child in children {
            visit_tree(child, by_parent, visited, result);
        }
    }
    result.push(ProcessIdentity::new(entry.pid, entry.started.clone()));
}

fn process_tree(entries: &[ProcessTreeEntry], root_pid: i32) -> Vec<ProcessIdentity> {
    let mut by_pid: HashMap<i32, &ProcessTreeEntry> = HashMap::new();
    for entry in entries {
        by_pid.insert(entry.pid, entry);
    }
    let Some(root) = by_pid.get(&root_pid).copied() else {
        return Vec::new();
    };
    let mut by_parent: HashMap<i32, Vec<&ProcessTreeEntry>> = HashMap::new();
    for entry in entries {
        by_parent.entry(entry.parent_pid).or_default().push(entry);
    }
    let mut visited = HashSet::new();
    let mut result = Vec::new();
    visit_tree(root, &by_parent, &mut visited, &mut result);
    result
}

struct DefaultInternals {
    files: Mutex<HashMap<i32, File>>,
    next_fd: Mutex<i32>,
}

impl DefaultInternals {
    fn new() -> Self {
        Self {
            files: Mutex::new(HashMap::new()),
            next_fd: Mutex::new(100),
        }
    }
}

fn lock_poison<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(unix)]
fn unix_signal_number(name: &str) -> Option<i32> {
    match name {
        "SIGINT" => Some(libc::SIGINT),
        "SIGTERM" => Some(libc::SIGTERM),
        "SIGKILL" => Some(libc::SIGKILL),
        "SIGTSTP" => Some(libc::SIGTSTP),
        "SIGHUP" => Some(libc::SIGHUP),
        _ => None,
    }
}

impl ProcessInspectorInternals for DefaultInternals {
    fn read_file(&self, path: &str) -> Result<String, io::Error> {
        std::fs::read_to_string(path)
    }

    fn read_dir(&self, path: &str) -> Result<Vec<String>, io::Error> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
        Ok(names)
    }

    fn open(&self, path: &str) -> Result<i32, io::Error> {
        let file = File::open(path)?;
        let mut files = lock_poison(&self.files);
        let mut next_fd = lock_poison(&self.next_fd);
        let fd = *next_fd;
        *next_fd = next_fd.saturating_add(1);
        files.insert(fd, file);
        Ok(fd)
    }

    fn read(&self, fd: i32, buffer: &mut [u8], position: u64) -> Result<usize, io::Error> {
        let files = lock_poison(&self.files);
        let file = files
            .get(&fd)
            .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "bad fd"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileExt;
            file.read_at(buffer, position)
        }
        #[cfg(not(unix))]
        {
            let _ = (file, position);
            Err(io::Error::new(
                ErrorKind::Unsupported,
                "positioned process-memory reads require unix",
            ))
        }
    }

    fn close(&self, fd: i32) {
        lock_poison(&self.files).remove(&fd);
    }

    fn exec(&self, file: &str, args: &[&str]) -> Result<String, io::Error> {
        let output = std::process::Command::new(file).args(args).output()?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "exec {file} exited {}",
                output.status
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn kill(&self, pid: i32, signal: &str) {
        #[cfg(unix)]
        {
            let Some(sig) = unix_signal_number(signal) else {
                return;
            };
            // SAFETY: pid is a live process or negated process-group id from this inspector;
            // ESRCH/EPERM are ignored so signal delivery stays idempotent.
            unsafe {
                libc::kill(pid, sig);
            }
        }
        #[cfg(not(unix))]
        {
            let _ = (pid, signal);
        }
    }
}

struct LinuxProcessInspector {
    table: Option<SyscallTable>,
    internals: Arc<dyn ProcessInspectorInternals>,
}

impl ProcessInspector for LinuxProcessInspector {
    fn foreground_pgid(&self, shell_pid: i32) -> Option<i32> {
        let tpgid = read_linux_stat(self.internals.as_ref(), shell_pid)?.tpgid;
        if tpgid > 0 { Some(tpgid) } else { None }
    }

    fn is_stdin_waiting(&self, pgid: i32) -> bool {
        let Some(table) = self.table else {
            return false;
        };
        for pid in numeric_entries(self.internals.as_ref(), "/proc") {
            let Some(stat) = read_linux_stat(self.internals.as_ref(), pid) else {
                continue;
            };
            if stat.pgrp != pgid {
                continue;
            }
            for tid in numeric_entries(self.internals.as_ref(), &format!("/proc/{pid}/task")) {
                let Some(syscall) = read_syscall(self.internals.as_ref(), pid, tid) else {
                    continue;
                };
                if syscall_waits_on_stdin(self.internals.as_ref(), pid, &syscall, table) {
                    return true;
                }
            }
        }
        false
    }

    fn process_tree(&self, root_pid: i32) -> Vec<ProcessIdentity> {
        let entries: Vec<ProcessTreeEntry> = numeric_entries(self.internals.as_ref(), "/proc")
            .into_iter()
            .filter_map(|pid| {
                let stat = read_linux_stat(self.internals.as_ref(), pid)?;
                Some(ProcessTreeEntry {
                    pid,
                    parent_pid: stat.parent_pid,
                    started: stat.started,
                })
            })
            .collect();
        process_tree(&entries, root_pid)
    }

    fn process_session(&self, session_id: i32) -> Vec<ProcessIdentity> {
        numeric_entries(self.internals.as_ref(), "/proc")
            .into_iter()
            .filter_map(|pid| {
                let stat = read_linux_stat(self.internals.as_ref(), pid)?;
                if stat.session == session_id {
                    Some(ProcessIdentity::new(pid, stat.started))
                } else {
                    None
                }
            })
            .collect()
    }

    fn is_alive(&self, identity: &ProcessIdentity) -> bool {
        let Some(stat) = read_linux_stat(self.internals.as_ref(), identity.pid) else {
            return false;
        };
        stat.started == identity.started && !is_quiescent_state(stat.state)
    }

    fn signal_group(&self, pgid: i32, signal: SubprocessTerminalSignal) {
        self.internals.kill(-pgid, terminal_signal_name(signal));
    }

    fn signal_process(&self, identity: &ProcessIdentity, signal: TermKill) {
        if self.is_alive(identity) {
            self.internals.kill(identity.pid, term_kill_name(signal));
        }
    }
}

fn parse_ps_line(line: &str) -> Option<ProcessTreeEntry> {
    let rest = line.trim();
    let mut parts = rest.split_whitespace();
    let pid: i32 = parts.next()?.parse().ok()?;
    let parent_pid: i32 = parts.next()?.parse().ok()?;
    let started = parts.collect::<Vec<_>>().join(" ");
    if started.is_empty() {
        return None;
    }
    Some(ProcessTreeEntry {
        pid,
        parent_pid,
        started,
    })
}

fn mac_process_table(internals: &dyn ProcessInspectorInternals) -> Vec<ProcessTreeEntry> {
    let Ok(text) = internals.exec("/bin/ps", &["-axo", "pid=,ppid=,lstart="]) else {
        return Vec::new();
    };
    text.split('\n').filter_map(parse_ps_line).collect()
}

struct MacProcessInspector {
    internals: Arc<dyn ProcessInspectorInternals>,
}

impl ProcessInspector for MacProcessInspector {
    fn foreground_pgid(&self, shell_pid: i32) -> Option<i32> {
        let text = self
            .internals
            .exec("/bin/ps", &["-o", "tpgid=", "-p", &shell_pid.to_string()])
            .ok()?;
        let value: i32 = text.trim().parse().ok()?;
        if value > 0 { Some(value) } else { None }
    }

    fn is_stdin_waiting(&self, _pgid: i32) -> bool {
        false
    }

    fn process_tree(&self, root_pid: i32) -> Vec<ProcessIdentity> {
        process_tree(&mac_process_table(self.internals.as_ref()), root_pid)
    }

    fn process_session(&self, _session_id: i32) -> Vec<ProcessIdentity> {
        Vec::new()
    }

    fn is_alive(&self, identity: &ProcessIdentity) -> bool {
        mac_process_table(self.internals.as_ref())
            .iter()
            .any(|entry| entry.pid == identity.pid && entry.started == identity.started)
    }

    fn signal_group(&self, pgid: i32, signal: SubprocessTerminalSignal) {
        self.internals.kill(-pgid, terminal_signal_name(signal));
    }

    fn signal_process(&self, identity: &ProcessIdentity, signal: TermKill) {
        if self.is_alive(identity) {
            self.internals.kill(identity.pid, term_kill_name(signal));
        }
    }
}

/// Create the host-platform inspector using real filesystem and `kill(2)` operations.
///
/// # Parameters
///
/// None.
///
/// # Returns
///
/// A Linux or macOS inspector for [`std::env::consts::OS`] and [`std::env::consts::ARCH`].
///
/// # Errors
///
/// [`SubprocessError::UnsupportedInspection`] when the OS is neither Linux nor macOS.
pub fn create_process_inspector() -> Result<Box<dyn ProcessInspector>, SubprocessError> {
    create_process_inspector_with(
        std::env::consts::OS,
        std::env::consts::ARCH,
        Arc::new(DefaultInternals::new()),
    )
}

/// Create an inspector for `platform` / `arch` with injectable process-table operations.
///
/// # Parameters
///
/// * `platform` - `linux`, `macos`, or `darwin`. Other names fail.
/// * `arch` - Linux syscall table key: `x86_64`/`x64` or `aarch64`/`arm64`.
/// * `internals` - Filesystem, `ps`, and `kill` operations.
///
/// # Returns
///
/// A platform inspector. Linux uses `arch` for stdin-wait syscall numbers.
///
/// # Errors
///
/// [`SubprocessError::UnsupportedInspection`] when `platform` is not Linux or macOS.
pub fn create_process_inspector_with(
    platform: &str,
    arch: &str,
    internals: Arc<dyn ProcessInspectorInternals>,
) -> Result<Box<dyn ProcessInspector>, SubprocessError> {
    match platform {
        "linux" => Ok(Box::new(LinuxProcessInspector {
            table: syscall_table(arch),
            internals,
        })),
        "macos" | "darwin" => Ok(Box::new(MacProcessInspector { internals })),
        other => Err(SubprocessError::UnsupportedInspection {
            platform: other.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ProcessIdentity, ProcessInspectorInternals, SubprocessTerminalSignal, TermKill,
        create_process_inspector, create_process_inspector_with,
        linux_process_group_has_live_members, parse_proc_stat,
    };
    use std::collections::HashMap;
    use std::io::{self, ErrorKind};
    use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
    use std::sync::{Arc, Mutex};

    struct FakeInternals {
        files: Mutex<HashMap<String, String>>,
        dirs: Mutex<HashMap<String, Vec<String>>>,
        memories: Mutex<HashMap<String, Vec<u8>>>,
        fds: Mutex<HashMap<i32, String>>,
        next_fd: AtomicI32,
        ps: Mutex<String>,
        tpgid: Mutex<String>,
        kills: Mutex<Vec<(i32, String)>>,
        exec_fails: AtomicBool,
    }

    impl FakeInternals {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                files: Mutex::new(HashMap::new()),
                dirs: Mutex::new(HashMap::new()),
                memories: Mutex::new(HashMap::new()),
                fds: Mutex::new(HashMap::new()),
                next_fd: AtomicI32::new(10),
                ps: Mutex::new(String::new()),
                tpgid: Mutex::new("0".to_string()),
                kills: Mutex::new(Vec::new()),
                exec_fails: AtomicBool::new(false),
            })
        }

        fn set_file(self: &Arc<Self>, path: &str, value: &str) {
            self.files
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(path.to_string(), value.to_string());
        }

        fn set_dir(self: &Arc<Self>, path: &str, entries: &[&str]) {
            self.dirs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(
                    path.to_string(),
                    entries.iter().map(|entry| (*entry).to_string()).collect(),
                );
        }

        fn delete_dir(self: &Arc<Self>, path: &str) {
            self.dirs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(path);
        }

        fn set_memory(self: &Arc<Self>, path: &str, bytes: Vec<u8>) {
            self.memories
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(path.to_string(), bytes);
        }

        fn set_ps(self: &Arc<Self>, value: &str) {
            *self
                .ps
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = value.to_string();
        }

        fn set_tpgid(self: &Arc<Self>, value: &str) {
            *self
                .tpgid
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = value.to_string();
        }

        fn fail_exec(self: &Arc<Self>) {
            self.exec_fails.store(true, Ordering::SeqCst);
        }

        fn kills(self: &Arc<Self>) -> Vec<(i32, String)> {
            self.kills
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }
    }

    impl ProcessInspectorInternals for FakeInternals {
        fn read_file(&self, path: &str) -> Result<String, io::Error> {
            self.files
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(path)
                .cloned()
                .ok_or_else(|| io::Error::new(ErrorKind::NotFound, format!("missing {path}")))
        }

        fn read_dir(&self, path: &str) -> Result<Vec<String>, io::Error> {
            self.dirs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(path)
                .cloned()
                .ok_or_else(|| io::Error::new(ErrorKind::NotFound, format!("missing {path}")))
        }

        fn open(&self, path: &str) -> Result<i32, io::Error> {
            if !self
                .memories
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .contains_key(path)
            {
                return Err(io::Error::new(
                    ErrorKind::NotFound,
                    format!("missing {path}"),
                ));
            }
            let fd = self.next_fd.fetch_add(1, Ordering::SeqCst);
            self.fds
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(fd, path.to_string());
            Ok(fd)
        }

        fn read(&self, fd: i32, buffer: &mut [u8], position: u64) -> Result<usize, io::Error> {
            let path = self
                .fds
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&fd)
                .cloned()
                .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "bad fd"))?;
            let memories = self
                .memories
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let source = memories
                .get(&path)
                .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "missing memory"))?;
            let start = usize::try_from(position).unwrap_or(usize::MAX);
            if start >= source.len() {
                return Ok(0);
            }
            let count = (source.len() - start).min(buffer.len());
            buffer[..count].copy_from_slice(&source[start..start + count]);
            Ok(count)
        }

        fn close(&self, fd: i32) {
            self.fds
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&fd);
        }

        fn exec(&self, _file: &str, args: &[&str]) -> Result<String, io::Error> {
            if self.exec_fails.load(Ordering::SeqCst) {
                return Err(io::Error::other("gone"));
            }
            if args.iter().any(|arg| *arg == "tpgid=") {
                return Ok(self
                    .tpgid
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone());
            }
            Ok(self
                .ps
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone())
        }

        fn kill(&self, pid: i32, signal: &str) {
            self.kills
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push((pid, signal.to_string()));
        }
    }

    fn stat_line(
        pid: i32,
        pgrp: i32,
        session: i32,
        tpgid: i32,
        started: &str,
        parent_pid: i32,
        state: &str,
    ) -> String {
        let mut rest = vec![
            state.to_string(),
            parent_pid.to_string(),
            pgrp.to_string(),
            session.to_string(),
            "99".to_string(),
            tpgid.to_string(),
        ];
        while rest.len() < 19 {
            rest.push("0".to_string());
        }
        rest.push(started.to_string());
        format!("{pid} (command with space) {}", rest.join(" "))
    }

    fn syscall_line(number: i64, args: &[i64]) -> String {
        let mut six: Vec<i64> = args.to_vec();
        while six.len() < 6 {
            six.push(0);
        }
        let hex: Vec<String> = six
            .iter()
            .take(6)
            .map(|value| format!("0x{value:x}"))
            .collect();
        format!("{number} {}", hex.join(" "))
    }

    fn linux(fake: Arc<FakeInternals>, arch: &str) -> Box<dyn super::ProcessInspector> {
        create_process_inspector_with("linux", arch, fake).expect("linux inspector")
    }

    #[test]
    fn parse_proc_stat_reads_tpgid() {
        let parsed = parse_proc_stat(&stat_line(10, 20, 30, 40, "500", 1, "S")).expect("stat");
        assert_eq!(parsed.pid(), 10);
        assert_eq!(parsed.parent_pid(), 1);
        assert_eq!(parsed.pgrp(), 20);
        assert_eq!(parsed.session(), 30);
        assert_eq!(parsed.state(), 'S');
        assert_eq!(parsed.tpgid(), 40);
        assert_eq!(parsed.started(), "500");
        assert!(parse_proc_stat("bad").is_none());
        assert!(parse_proc_stat("1 () ").is_none());
        assert!(parse_proc_stat("1 () S").is_none());
        assert!(parse_proc_stat(&stat_line(10, 20, 30, 40, "500", 1, "SS")).is_none());
    }

    #[test]
    fn linux_read_fd0_is_stdin_wait() {
        let fake = FakeInternals::new();
        fake.set_dir("/proc", &["100", "101"]);
        fake.set_file("/proc/100/stat", &stat_line(100, 77, 100, 77, "1", 1, "S"));
        fake.set_file("/proc/101/stat", &stat_line(101, 77, 100, 77, "2", 1, "S"));
        fake.set_dir("/proc/100/task", &["100"]);
        fake.set_dir("/proc/101/task", &["101", "102"]);
        fake.set_file("/proc/100/task/100/syscall", "running");
        fake.set_file("/proc/101/task/101/syscall", "-1 0x0");
        fake.set_file("/proc/101/task/102/syscall", &syscall_line(0, &[0]));
        let inspector = linux(Arc::clone(&fake), "x86_64");
        assert!(inspector.is_stdin_waiting(77));
    }

    #[test]
    fn linux_unreadable_mem_is_not_stdin_wait() {
        let fake = FakeInternals::new();
        fake.set_dir("/proc", &["100"]);
        fake.set_file("/proc/100/stat", &stat_line(100, 77, 100, 77, "1", 1, "S"));
        fake.set_dir("/proc/100/task", &["100"]);
        let inspector = linux(Arc::clone(&fake), "x86_64");
        fake.set_file("/proc/100/task/100/syscall", &syscall_line(270, &[1, 0x10]));
        assert!(!inspector.is_stdin_waiting(77));
        fake.set_file("/proc/100/task/100/syscall", &syscall_line(232, &[5, 0, 1]));
        assert!(!inspector.is_stdin_waiting(77));
    }

    #[test]
    fn linux_aarch64_read_63_fd0_is_stdin_wait() {
        let fake = FakeInternals::new();
        fake.set_dir("/proc", &["100"]);
        fake.set_file("/proc/100/stat", &stat_line(100, 77, 100, 77, "1", 1, "S"));
        fake.set_dir("/proc/100/task", &["100"]);
        fake.set_file("/proc/100/task/100/syscall", &syscall_line(63, &[0]));
        let inspector = linux(Arc::clone(&fake), "aarch64");
        assert!(inspector.is_stdin_waiting(77));
        let x86 = linux(Arc::clone(&fake), "x86_64");
        assert!(!x86.is_stdin_waiting(77));
        if std::env::consts::ARCH == "aarch64" {
            let host = linux(Arc::clone(&fake), std::env::consts::ARCH);
            assert!(host.is_stdin_waiting(77));
        }
    }

    #[test]
    fn macos_is_stdin_waiting_is_always_false() {
        let fake = FakeInternals::new();
        fake.set_dir("/proc", &["100"]);
        fake.set_file("/proc/100/stat", &stat_line(100, 77, 100, 77, "1", 1, "S"));
        fake.set_dir("/proc/100/task", &["100"]);
        fake.set_file("/proc/100/task/100/syscall", &syscall_line(0, &[0]));
        let inspector =
            create_process_inspector_with("darwin", "aarch64", fake).expect("macos inspector");
        assert!(!inspector.is_stdin_waiting(77));
    }

    #[test]
    fn linux_process_group_and_tree_and_signals() {
        let fake = FakeInternals::new();
        assert_eq!(
            linux_process_group_has_live_members(77, fake.as_ref()),
            None
        );

        fake.set_dir("/proc", &["self", "10", "11", "12"]);
        fake.set_file("/proc/10/stat", &stat_line(10, 77, 10, -1, "500", 1, "Z"));
        fake.set_file("/proc/11/stat", &stat_line(11, 77, 10, -1, "501", 1, "X"));
        fake.set_file("/proc/12/stat", &stat_line(12, 88, 12, -1, "502", 1, "S"));
        assert_eq!(
            linux_process_group_has_live_members(77, fake.as_ref()),
            Some(false)
        );
        assert_eq!(
            linux_process_group_has_live_members(99, fake.as_ref()),
            None
        );

        fake.set_file("/proc/11/stat", &stat_line(11, 77, 10, -1, "501", 1, "S"));
        assert_eq!(
            linux_process_group_has_live_members(77, fake.as_ref()),
            Some(true)
        );

        fake.set_dir("/proc", &["x", "10", "11", "12", "13", "14"]);
        fake.set_file("/proc/10/stat", &stat_line(10, 20, 30, 40, "500", 1, "S"));
        fake.set_file("/proc/11/stat", &stat_line(11, 21, 30, -1, "501", 1, "S"));
        fake.set_file("/proc/12/stat", &stat_line(12, 22, 30, -1, "502", 10, "S"));
        fake.set_file("/proc/13/stat", &stat_line(13, 23, 30, -1, "503", 12, "S"));
        let inspector = linux(Arc::clone(&fake), "x86_64");
        assert_eq!(inspector.foreground_pgid(10), Some(40));
        assert_eq!(inspector.foreground_pgid(11), None);
        assert_eq!(inspector.foreground_pgid(99), None);
        assert_eq!(
            inspector.process_tree(10),
            vec![
                ProcessIdentity::new(13, "503"),
                ProcessIdentity::new(12, "502"),
                ProcessIdentity::new(10, "500"),
            ]
        );
        assert_eq!(inspector.process_tree(99), Vec::<ProcessIdentity>::new());
        assert_eq!(
            inspector.process_session(30),
            vec![
                ProcessIdentity::new(10, "500"),
                ProcessIdentity::new(11, "501"),
                ProcessIdentity::new(12, "502"),
                ProcessIdentity::new(13, "503"),
            ]
        );
        assert!(inspector.is_alive(&ProcessIdentity::new(10, "500")));
        assert!(!inspector.is_alive(&ProcessIdentity::new(10, "old")));
        inspector.signal_group(40, SubprocessTerminalSignal::Sigint);
        inspector.signal_process(&ProcessIdentity::new(10, "500"), TermKill::Sigterm);
        inspector.signal_process(&ProcessIdentity::new(10, "old"), TermKill::Sigkill);
        assert_eq!(
            fake.kills(),
            vec![(-40, "SIGINT".to_string()), (10, "SIGTERM".to_string())]
        );
        fake.set_file("/proc/10/stat", &stat_line(10, 20, 30, 40, "500", 1, "Z"));
        assert!(!inspector.is_alive(&ProcessIdentity::new(10, "500")));
        inspector.signal_process(&ProcessIdentity::new(10, "500"), TermKill::Sigkill);
        assert_eq!(
            fake.kills(),
            vec![(-40, "SIGINT".to_string()), (10, "SIGTERM".to_string())]
        );
    }

    #[test]
    fn linux_detects_select_poll_and_epoll_stdin_waits() {
        let fake = FakeInternals::new();
        fake.set_dir("/proc", &["100", "101"]);
        fake.set_file("/proc/100/stat", &stat_line(100, 77, 100, 77, "1", 1, "S"));
        fake.set_file("/proc/101/stat", &stat_line(101, 77, 100, 77, "2", 1, "S"));
        fake.set_dir("/proc/100/task", &["100"]);
        fake.set_dir("/proc/101/task", &["101", "102"]);
        let inspector = linux(Arc::clone(&fake), "x86_64");

        fake.set_file("/proc/101/task/102/syscall", &syscall_line(270, &[1, 0x10]));
        let mut fd_set = vec![0u8; 0x11];
        fd_set[0x10] = 1;
        fake.set_memory("/proc/101/mem", fd_set);
        assert!(inspector.is_stdin_waiting(77));

        let mut poll = vec![0u8; 8];
        poll[0..4].copy_from_slice(&0i32.to_le_bytes());
        poll[4..6].copy_from_slice(&1i16.to_le_bytes());
        fake.set_file("/proc/101/task/102/syscall", &syscall_line(7, &[0x20, 1]));
        let mut poll_mem = vec![0u8; 0x20];
        poll_mem.extend_from_slice(&poll);
        fake.set_memory("/proc/101/mem", poll_mem);
        assert!(inspector.is_stdin_waiting(77));

        fake.set_file("/proc/101/task/102/syscall", &syscall_line(232, &[5, 0, 1]));
        fake.set_file("/proc/101/fdinfo/5", "pos: 0\ntfd: 0 events: 19\n");
        assert!(inspector.is_stdin_waiting(77));
    }

    #[test]
    fn linux_fails_closed_on_non_stdin_and_malformed_waits() {
        let fake = FakeInternals::new();
        fake.set_dir("/proc", &["100"]);
        fake.set_file("/proc/100/stat", &stat_line(100, 77, 100, 77, "1", 1, "S"));
        fake.set_dir("/proc/100/task", &["100"]);
        fake.set_file("/proc/100/task/100/syscall", &syscall_line(0, &[2]));
        assert!(!linux(Arc::clone(&fake), "mips").is_stdin_waiting(77));
        assert!(!linux(Arc::clone(&fake), "x86_64").is_stdin_waiting(77));

        fake.set_file("/proc/100/task/100/syscall", &syscall_line(270, &[1, 0]));
        assert!(!linux(Arc::clone(&fake), "x86_64").is_stdin_waiting(77));
        fake.set_file("/proc/100/task/100/syscall", &syscall_line(7, &[0, 0]));
        assert!(!linux(Arc::clone(&fake), "x86_64").is_stdin_waiting(77));
        fake.set_file("/proc/100/task/100/syscall", &syscall_line(7, &[0, 1]));
        assert!(!linux(Arc::clone(&fake), "x86_64").is_stdin_waiting(77));
        fake.set_file("/proc/100/task/100/syscall", &syscall_line(7, &[0x20, 1]));
        assert!(!linux(Arc::clone(&fake), "x86_64").is_stdin_waiting(77));
        fake.set_file("/proc/100/task/100/syscall", &syscall_line(232, &[9, 0, 1]));
        assert!(!linux(Arc::clone(&fake), "x86_64").is_stdin_waiting(77));
        fake.set_file("/proc/100/task/100/syscall", &syscall_line(999, &[]));
        assert!(!linux(Arc::clone(&fake), "x86_64").is_stdin_waiting(77));
        fake.set_file("/proc/100/task/100/syscall", "not-a-number 0x0");
        assert!(!linux(Arc::clone(&fake), "x86_64").is_stdin_waiting(77));
        fake.delete_dir("/proc/100/task");
        assert!(!linux(Arc::clone(&fake), "x86_64").is_stdin_waiting(77));
        fake.set_dir("/proc", &["100", "200"]);
        fake.set_file("/proc/200/stat", &stat_line(200, 88, 200, 88, "2", 1, "S"));
        assert!(!linux(Arc::clone(&fake), "x86_64").is_stdin_waiting(77));
    }

    #[test]
    fn linux_poll_without_stdin_fd_is_not_wait() {
        let fake = FakeInternals::new();
        fake.set_dir("/proc", &["100"]);
        fake.set_file("/proc/100/stat", &stat_line(100, 77, 100, 77, "1", 1, "S"));
        fake.set_dir("/proc/100/task", &["100"]);
        let inspector = linux(Arc::clone(&fake), "x86_64");
        let mut no_stdin = vec![0u8; 0x28];
        no_stdin[0x20..0x24].copy_from_slice(&2i32.to_le_bytes());
        no_stdin[0x24..0x26].copy_from_slice(&1i16.to_le_bytes());
        fake.set_memory("/proc/100/mem", no_stdin);
        fake.set_file("/proc/100/task/100/syscall", &syscall_line(7, &[0x20, 1]));
        assert!(!inspector.is_stdin_waiting(77));
    }

    #[test]
    fn macos_reads_tpgid_trees_and_identity_fences() {
        let fake = FakeInternals::new();
        fake.set_tpgid("55\n");
        fake.set_ps(
            " 10 1 Mon Jul 21 10:00:00 2026\n 11 10 Mon Jul 21 10:00:01 2026\n 12 11 Mon Jul 21 10:00:02 2026\n 13 99 Mon Jul 21 10:00:03 2026\nmalformed\n",
        );
        let inspector =
            create_process_inspector_with("darwin", "aarch64", Arc::clone(&fake) as Arc<_>)
                .expect("macos inspector");
        assert_eq!(inspector.foreground_pgid(10), Some(55));
        assert!(!inspector.is_stdin_waiting(55));
        assert_eq!(
            inspector.process_tree(10),
            vec![
                ProcessIdentity::new(12, "Mon Jul 21 10:00:02 2026"),
                ProcessIdentity::new(11, "Mon Jul 21 10:00:01 2026"),
                ProcessIdentity::new(10, "Mon Jul 21 10:00:00 2026"),
            ]
        );
        assert_eq!(inspector.process_tree(99), Vec::<ProcessIdentity>::new());
        assert_eq!(inspector.process_session(10), Vec::<ProcessIdentity>::new());
        assert!(inspector.is_alive(&ProcessIdentity::new(11, "Mon Jul 21 10:00:01 2026")));
        inspector.signal_group(55, SubprocessTerminalSignal::Sigtstp);
        inspector.signal_process(
            &ProcessIdentity::new(11, "Mon Jul 21 10:00:01 2026"),
            TermKill::Sigkill,
        );
        inspector.signal_process(&ProcessIdentity::new(12, "missing"), TermKill::Sigterm);
        assert_eq!(
            fake.kills(),
            vec![(-55, "SIGTSTP".to_string()), (11, "SIGKILL".to_string())]
        );

        fake.set_ps(" 10 11 Mon Jul 21 10:00:00 2026\n 11 10 Mon Jul 21 10:00:01 2026\n");
        assert_eq!(
            inspector.process_tree(10),
            vec![
                ProcessIdentity::new(11, "Mon Jul 21 10:00:01 2026"),
                ProcessIdentity::new(10, "Mon Jul 21 10:00:00 2026"),
            ]
        );
    }

    #[test]
    fn macos_missing_tpgid_and_unsupported_platform() {
        let fake = FakeInternals::new();
        fake.set_tpgid("-1");
        assert_eq!(
            create_process_inspector_with("darwin", "aarch64", Arc::clone(&fake) as Arc<_>)
                .expect("macos")
                .foreground_pgid(1),
            None
        );
        fake.fail_exec();
        assert_eq!(
            create_process_inspector_with("darwin", "aarch64", Arc::clone(&fake) as Arc<_>)
                .expect("macos")
                .foreground_pgid(1),
            None
        );
        let err = match create_process_inspector_with("win32", "x86_64", fake) {
            Ok(_) => panic!("expected unsupported platform"),
            Err(error) => error,
        };
        assert_eq!(
            err.to_string(),
            "subprocess-local: terminal inspection is unsupported on platform win32"
        );
    }

    #[test]
    fn create_process_inspector_matches_host_os() {
        match std::env::consts::OS {
            "linux" | "macos" => {
                create_process_inspector().expect("host inspector");
            }
            other => {
                let err = match create_process_inspector() {
                    Ok(_) => panic!("expected unsupported platform"),
                    Err(error) => error,
                };
                assert_eq!(
                    err.to_string(),
                    format!(
                        "subprocess-local: terminal inspection is unsupported on platform {other}"
                    )
                );
            }
        }
    }

    #[test]
    fn terminal_handle_does_not_expose_inspect_signal_terminate() {
        let terminal = include_str!("terminal.rs");
        assert!(
            !terminal.contains("fn inspect_foreground"),
            "inspect_foreground is not a SubprocessTerminalHandle method"
        );
        assert!(
            !terminal.contains("fn signal_foreground"),
            "signal_foreground is not a SubprocessTerminalHandle method"
        );
        assert!(
            !terminal.contains("fn terminate"),
            "terminate is not a SubprocessTerminalHandle method"
        );
    }
}
