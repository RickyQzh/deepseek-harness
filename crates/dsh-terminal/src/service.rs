//! Owner-scoped PTY session registry.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use dsh_session::SessionId;

use crate::TerminalSessionId;

/// Machine-routable PTY service failure code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalErrorCode {
    /// A backend type is already registered.
    DuplicateBackend,
    /// An owner-local session name is already in use.
    DuplicateName,
    /// The caller is not the session owner.
    ForeignSession,
    /// No backend is registered for the requested type.
    NoBackend,
    /// The session id is not published.
    NoSession,
    /// The owner is no longer live.
    OwnerNotLive,
    /// The session already has an exclusive send.
    SendActive,
    /// The service is disposing.
    ServiceDisposing,
}

impl TerminalErrorCode {
    /// Stable routing token.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The TypeScript `TerminalErrorCode` string.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DuplicateBackend => "DUPLICATE_BACKEND",
            Self::DuplicateName => "DUPLICATE_NAME",
            Self::ForeignSession => "FOREIGN_SESSION",
            Self::NoBackend => "NO_BACKEND",
            Self::NoSession => "NO_SESSION",
            Self::OwnerNotLive => "OWNER_NOT_LIVE",
            Self::SendActive => "SEND_ACTIVE",
            Self::ServiceDisposing => "SERVICE_DISPOSING",
        }
    }
}

/// Error carrying a stable [`TerminalErrorCode`]. [`Display`](std::fmt::Display) is the message.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct TerminalError {
    message: String,
    code: TerminalErrorCode,
}

impl TerminalError {
    /// Construct a failure with `message` and `code`.
    ///
    /// # Parameters
    ///
    /// * `message` - Human-readable failure summary.
    /// * `code` - Stable routing token.
    ///
    /// # Returns
    ///
    /// The typed error.
    #[must_use]
    pub fn new(message: impl Into<String>, code: TerminalErrorCode) -> Self {
        Self {
            message: message.into(),
            code,
        }
    }

    /// Stable routing token.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The [`TerminalErrorCode`] stored on this error.
    #[must_use]
    pub fn code(&self) -> TerminalErrorCode {
        self.code
    }
}

/// Why one interactive send returned control to its caller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalWaitReason {
    /// Linux stdin-wait, or the snapshot backend's locked reason.
    StdinRead,
    /// Prompt-marker plus silence without stdin-wait evidence.
    InferredIdle,
    /// Send wait exceeded the configured timeout.
    Timeout,
    /// The top-level PTY process exited.
    SessionExit,
}

/// Signals the model-facing PTY surface permits for foreground process groups.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalSignal {
    /// Interrupt the foreground process group.
    Sigint,
    /// Terminate the foreground process group.
    Sigterm,
    /// Kill the foreground process group. The bash backend refuses this for the shell PGID.
    Sigkill,
    /// Stop the foreground process group.
    Sigtstp,
    /// Hangup the foreground process group.
    Sighup,
}

/// Top-level PTY process status, independent of a send's wait reason.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalSessionStatus {
    /// The top-level process is still running.
    Running,
    /// The top-level process has exited.
    Exited {
        /// Process exit code, or none when the process was signaled without a code.
        exit_code: Option<i32>,
        /// Signal name when the process was terminated by a signal.
        signal: Option<String>,
    },
}

/// Request to create one owner-scoped PTY session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalSpawnRequest {
    backend_type: String,
    name: Option<String>,
    cwd: Option<String>,
}

impl TerminalSpawnRequest {
    /// Request a session from the backend registered as `backend_type`.
    ///
    /// # Parameters
    ///
    /// * `backend_type` - Registered backend type such as `shell`.
    ///
    /// # Returns
    ///
    /// A request with no owner-local name and no cwd overlay.
    #[must_use]
    pub fn new(backend_type: impl Into<String>) -> Self {
        Self {
            backend_type: backend_type.into(),
            name: None,
            cwd: None,
        }
    }

    /// Set the optional owner-local display name.
    ///
    /// # Parameters
    ///
    /// * `name` - Owner-local display name.
    ///
    /// # Returns
    ///
    /// The same request with `name` set.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the optional initial working directory.
    ///
    /// # Parameters
    ///
    /// * `cwd` - Backend-interpreted working directory.
    ///
    /// # Returns
    ///
    /// The same request with `cwd` set.
    #[must_use]
    pub fn with_cwd(mut self, cwd: impl Into<String>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    /// Registered backend type.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The backend type string.
    #[must_use]
    pub fn backend_type(&self) -> &str {
        &self.backend_type
    }

    /// Optional owner-local display name.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The name when the caller supplied one.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Optional initial working directory.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The cwd when the caller supplied one.
    #[must_use]
    pub fn cwd(&self) -> Option<&str> {
        self.cwd.as_deref()
    }
}

/// Fully identified request handed from the registry to a backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalBackendSpawnSpec {
    session_id: TerminalSessionId,
    owner: SessionId,
    backend_type: String,
    name: Option<String>,
    cwd: Option<String>,
}

impl TerminalBackendSpawnSpec {
    /// Build the spec the registry passes into [`TerminalBackend::spawn`].
    ///
    /// # Parameters
    ///
    /// * `session_id` - Registry-minted session identity.
    /// * `owner` - Owning session.
    /// * `backend_type` - Registered backend type.
    /// * `name` - Optional owner-local display name.
    /// * `cwd` - Optional initial working directory.
    ///
    /// # Returns
    ///
    /// The spec.
    #[must_use]
    pub fn new(
        session_id: TerminalSessionId,
        owner: SessionId,
        backend_type: impl Into<String>,
        name: Option<String>,
        cwd: Option<String>,
    ) -> Self {
        Self {
            session_id,
            owner,
            backend_type: backend_type.into(),
            name,
            cwd,
        }
    }

    /// Registry-minted session identity.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The branded id.
    #[must_use]
    pub fn session_id(&self) -> &TerminalSessionId {
        &self.session_id
    }

    /// Owning session.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The owner [`SessionId`].
    #[must_use]
    pub fn owner(&self) -> &SessionId {
        &self.owner
    }

    /// Registered backend type.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The backend type string.
    #[must_use]
    pub fn backend_type(&self) -> &str {
        &self.backend_type
    }

    /// Optional owner-local display name.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The name when present.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Optional initial working directory.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The cwd when present.
    #[must_use]
    pub fn cwd(&self) -> Option<&str> {
        self.cwd.as_deref()
    }
}

/// Input for one line-oriented terminal interaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalSendRequest {
    text: String,
    submit: bool,
}

impl TerminalSendRequest {
    /// Build a send request.
    ///
    /// # Parameters
    ///
    /// * `text` - UTF-8 text to write.
    /// * `submit` - Whether to write the backend's Enter sequence after `text`.
    ///
    /// # Returns
    ///
    /// The request.
    #[must_use]
    pub fn new(text: impl Into<String>, submit: bool) -> Self {
        Self {
            text: text.into(),
            submit,
        }
    }

    /// UTF-8 text to write.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Whether to write the backend's Enter sequence after [`Self::text`].
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// `true` when Enter should follow the text.
    #[must_use]
    pub fn submit(&self) -> bool {
        self.submit
    }
}

/// Incremental output consumed from one live send operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalSendRead {
    delta: String,
    truncated: bool,
}

impl TerminalSendRead {
    /// Build one incremental read.
    ///
    /// # Parameters
    ///
    /// * `delta` - Output produced since the previous operation read.
    /// * `truncated` - Whether unread operation output was dropped.
    ///
    /// # Returns
    ///
    /// The read.
    #[must_use]
    pub fn new(delta: impl Into<String>, truncated: bool) -> Self {
        Self {
            delta: delta.into(),
            truncated,
        }
    }

    /// Output produced since the previous operation read.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The delta text.
    #[must_use]
    pub fn delta(&self) -> &str {
        &self.delta
    }

    /// Whether unread operation output was dropped by the backend's bound.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// `true` when output was dropped.
    #[must_use]
    pub fn truncated(&self) -> bool {
        self.truncated
    }
}

/// Settled result for one foreground or background send.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalSendResult {
    viewport: String,
    wait_reason: TerminalWaitReason,
    session_status: TerminalSessionStatus,
    truncated: bool,
}

impl TerminalSendResult {
    /// Build a settled send result.
    ///
    /// # Parameters
    ///
    /// * `viewport` - Bounded rendered terminal delta remaining at settlement.
    /// * `wait_reason` - Why the wait returned.
    /// * `session_status` - Top-level session status at settlement.
    /// * `truncated` - Whether output was dropped.
    ///
    /// # Returns
    ///
    /// The result.
    #[must_use]
    pub fn new(
        viewport: impl Into<String>,
        wait_reason: TerminalWaitReason,
        session_status: TerminalSessionStatus,
        truncated: bool,
    ) -> Self {
        Self {
            viewport: viewport.into(),
            wait_reason,
            session_status,
            truncated,
        }
    }

    /// Bounded rendered terminal delta remaining at settlement.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The viewport text.
    #[must_use]
    pub fn viewport(&self) -> &str {
        &self.viewport
    }

    /// Why the wait returned; this does not imply arbitrary child-process exit.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The wait reason.
    #[must_use]
    pub fn wait_reason(&self) -> TerminalWaitReason {
        self.wait_reason
    }

    /// Top-level session status observed at settlement.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// A clone of the status.
    #[must_use]
    pub fn session_status(&self) -> TerminalSessionStatus {
        self.session_status.clone()
    }

    /// Whether output was dropped from the operation or retained scrollback.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// `true` when output was dropped.
    #[must_use]
    pub fn truncated(&self) -> bool {
        self.truncated
    }
}

/// Live backend-owned send; exactly one may be active per PTY session.
pub struct TerminalSendOperation {
    done: Pin<Box<dyn Future<Output = TerminalSendResult> + Send>>,
    read_output: Box<dyn Fn() -> TerminalSendRead + Send + Sync>,
    cancel: Box<dyn Fn() -> bool + Send + Sync>,
    on_settle: Option<Box<dyn FnOnce() + Send>>,
}

impl TerminalSendOperation {
    /// Build an operation from backend closures.
    ///
    /// # Parameters
    ///
    /// * `done` - Future that settles after readiness, timeout, cancellation, or process exit.
    /// * `read_output` - Consume output produced since the prior call.
    /// * `cancel` - Request `SIGINT`; returns false after the operation settled.
    ///
    /// # Returns
    ///
    /// The live operation handle.
    #[must_use]
    pub fn new(
        done: Pin<Box<dyn Future<Output = TerminalSendResult> + Send>>,
        read_output: Box<dyn Fn() -> TerminalSendRead + Send + Sync>,
        cancel: Box<dyn Fn() -> bool + Send + Sync>,
    ) -> Self {
        Self {
            done,
            read_output,
            cancel,
            on_settle: None,
        }
    }

    pub(crate) fn with_on_settle(mut self, on_settle: Box<dyn FnOnce() + Send>) -> Self {
        self.on_settle = Some(on_settle);
        self
    }

    /// Await settlement.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The settled send result. Clears the session's exclusive-send flag.
    pub async fn done(&mut self) -> TerminalSendResult {
        let result = self.done.as_mut().await;
        self.settle();
        result
    }

    /// Consume output produced since the prior call.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The incremental read.
    #[must_use]
    pub fn read_output(&self) -> TerminalSendRead {
        (self.read_output)()
    }

    /// Request `SIGINT`. Snapshot `done` is already resolved, so this returns false.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// `true` when cancellation was accepted; `false` after settlement.
    #[must_use]
    pub fn cancel(&self) -> bool {
        (self.cancel)()
    }

    fn settle(&mut self) {
        if let Some(on_settle) = self.on_settle.take() {
            on_settle();
        }
    }
}

impl Drop for TerminalSendOperation {
    fn drop(&mut self) {
        self.settle();
    }
}

impl fmt::Debug for TerminalSendOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminalSendOperation")
            .finish_non_exhaustive()
    }
}

/// Request for one backward scrollback page.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TerminalReadRequest {
    offset: Option<usize>,
    count: Option<usize>,
}

impl TerminalReadRequest {
    /// Empty request; backends apply their own defaults.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// A request with no offset or count.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Offset from the newest retained line.
    ///
    /// # Parameters
    ///
    /// * `offset` - Newest-relative offset.
    ///
    /// # Returns
    ///
    /// The same request with `offset` set.
    #[must_use]
    pub fn with_offset(mut self, offset: usize) -> Self {
        self.offset = Some(offset);
        self
    }

    /// Requested line count; backend limits still apply.
    ///
    /// # Parameters
    ///
    /// * `count` - Requested line count.
    ///
    /// # Returns
    ///
    /// The same request with `count` set.
    #[must_use]
    pub fn with_count(mut self, count: usize) -> Self {
        self.count = Some(count);
        self
    }

    /// Offset from the newest retained line.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The offset when the caller supplied one.
    #[must_use]
    pub fn offset(&self) -> Option<usize> {
        self.offset
    }

    /// Requested line count.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The count when the caller supplied one.
    #[must_use]
    pub fn count(&self) -> Option<usize> {
        self.count
    }
}

/// Bounded scrollback page.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalReadResult {
    text: String,
    total_lines: usize,
    line_begin: usize,
    line_end: usize,
    truncated: bool,
}

impl TerminalReadResult {
    /// Build a scrollback page.
    ///
    /// # Parameters
    ///
    /// * `text` - Retained text in chronological order.
    /// * `total_lines` - Number of lines currently retained.
    /// * `line_begin` - Inclusive newest-relative offset of the first returned line.
    /// * `line_end` - Exclusive newest-relative offset after the returned page.
    /// * `truncated` - Whether a bound dropped output.
    ///
    /// # Returns
    ///
    /// The page.
    #[must_use]
    pub fn new(
        text: impl Into<String>,
        total_lines: usize,
        line_begin: usize,
        line_end: usize,
        truncated: bool,
    ) -> Self {
        Self {
            text: text.into(),
            total_lines,
            line_begin,
            line_end,
            truncated,
        }
    }

    /// Retained text in chronological order.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The page text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Number of lines currently retained.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The retained line count.
    #[must_use]
    pub fn total_lines(&self) -> usize {
        self.total_lines
    }

    /// Inclusive newest-relative offset of the first returned line.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The begin offset.
    #[must_use]
    pub fn line_begin(&self) -> usize {
        self.line_begin
    }

    /// Exclusive newest-relative offset after the returned page.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The end offset.
    #[must_use]
    pub fn line_end(&self) -> usize {
        self.line_end
    }

    /// Whether older retained output or the requested result exceeded a bound.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// `true` when a bound dropped output.
    #[must_use]
    pub fn truncated(&self) -> bool {
        self.truncated
    }
}

/// Result of delivering a signal to a verified foreground process group.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalSignalResult {
    delivered: bool,
    target_pgid: i32,
}

impl TerminalSignalResult {
    /// Build a signal result. `delivered` is true after the backend delivered the signal.
    ///
    /// # Parameters
    ///
    /// * `delivered` - Whether the backend delivered the signal.
    /// * `target_pgid` - Process group that received the signal.
    ///
    /// # Returns
    ///
    /// The result.
    #[must_use]
    pub fn new(delivered: bool, target_pgid: i32) -> Self {
        Self {
            delivered,
            target_pgid,
        }
    }

    /// True only after the backend delivered the signal.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The delivery flag.
    #[must_use]
    pub fn delivered(&self) -> bool {
        self.delivered
    }

    /// Process group that received the signal.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The target PGID.
    #[must_use]
    pub fn target_pgid(&self) -> i32 {
        self.target_pgid
    }
}

/// Owner-visible summary of one published PTY session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalSessionSnapshot {
    session_id: TerminalSessionId,
    name: Option<String>,
    backend_type: String,
    pid: Option<i32>,
    status: TerminalSessionStatus,
}

impl TerminalSessionSnapshot {
    /// Registry-minted identity used by every operation.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The branded id.
    #[must_use]
    pub fn session_id(&self) -> &TerminalSessionId {
        &self.session_id
    }

    /// Optional owner-local display name.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The name when present.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Backend type that created the session.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The backend type string.
    #[must_use]
    pub fn backend_type(&self) -> &str {
        &self.backend_type
    }

    /// Top-level process id when the backend has one.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The pid when present.
    #[must_use]
    pub fn pid(&self) -> Option<i32> {
        self.pid
    }

    /// Current top-level process status.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// A clone of the status.
    #[must_use]
    pub fn status(&self) -> TerminalSessionStatus {
        self.status.clone()
    }
}

/// Successful publication returned by [`TerminalSessionService::spawn`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalSpawnResult {
    session_id: TerminalSessionId,
    name: Option<String>,
    backend_type: String,
    pid: Option<i32>,
    status: TerminalSessionStatus,
    motd: String,
}

impl TerminalSpawnResult {
    /// Registry-minted identity used by every operation.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The branded id.
    #[must_use]
    pub fn session_id(&self) -> &TerminalSessionId {
        &self.session_id
    }

    /// Optional owner-local display name.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The name when present.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Backend type that created the session.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The backend type string.
    #[must_use]
    pub fn backend_type(&self) -> &str {
        &self.backend_type
    }

    /// Top-level process id when the backend has one.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The pid when present.
    #[must_use]
    pub fn pid(&self) -> Option<i32> {
        self.pid
    }

    /// Current top-level process status.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// A clone of the status.
    #[must_use]
    pub fn status(&self) -> TerminalSessionStatus {
        self.status.clone()
    }

    /// Initial bounded output captured before publication.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The MOTD text.
    #[must_use]
    pub fn motd(&self) -> &str {
        &self.motd
    }
}

/// Backend-owned live session retained by [`TerminalSessionService`].
pub trait TerminalBackendSession: Send + Sync {
    /// Initial bounded terminal output returned from `terminal_open`.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The MOTD text.
    fn motd(&self) -> &str;

    /// Top-level process id when one exists.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The pid when the backend has one.
    fn pid(&self) -> Option<i32>;

    /// Start one exclusive send operation.
    ///
    /// # Parameters
    ///
    /// * `request` - Text, submit flag, and any backend-specific wait policy.
    ///
    /// # Returns
    ///
    /// The live operation handle.
    fn start_send(&self, request: TerminalSendRequest) -> TerminalSendOperation;

    /// Read one bounded page from retained scrollback.
    ///
    /// # Parameters
    ///
    /// * `request` - Optional newest-relative offset and line count.
    ///
    /// # Returns
    ///
    /// The bounded page.
    fn read(&self, request: TerminalReadRequest) -> TerminalReadResult;

    /// Signal the verified foreground process group.
    ///
    /// # Parameters
    ///
    /// * `signal` - Allowed POSIX signal name.
    ///
    /// # Returns
    ///
    /// Object-safe boxed future that settles to the delivered foreground process-group identity.
    fn signal(
        &self,
        signal: TerminalSignal,
    ) -> Pin<Box<dyn Future<Output = TerminalSignalResult> + Send>>;

    /// Observe top-level process status.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The current status.
    fn status(&self) -> TerminalSessionStatus;

    /// Idempotently close the captured owned process tree and await quiescence.
    ///
    /// # Parameters
    ///
    /// * `reason` - Diagnostic cleanup reason.
    ///
    /// # Returns
    ///
    /// Object-safe boxed future that settles to `Ok(())` after cleanup.
    ///
    /// # Errors
    ///
    /// Backend cleanup failure.
    fn close(
        &self,
        reason: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), TerminalError>> + Send>>;
}

/// Object-safe boxed future returned by [`TerminalBackend::spawn`].
pub type TerminalBackendSpawnFuture =
    Pin<Box<dyn Future<Output = Result<Box<dyn TerminalBackendSession>, TerminalError>> + Send>>;

/// Replaceable provider for one PTY session type.
pub trait TerminalBackend: Send + Sync {
    /// Create an unpublished session or reject after cleaning partial resources.
    ///
    /// # Parameters
    ///
    /// * `spec` - Registry-minted id, owner, type, and optional name/cwd.
    ///
    /// # Returns
    ///
    /// Object-safe boxed future that settles to the unpublished backend session.
    ///
    /// # Errors
    ///
    /// Backend setup failure.
    fn spawn(&self, spec: TerminalBackendSpawnSpec) -> TerminalBackendSpawnFuture;
}

struct SessionRecord {
    id: TerminalSessionId,
    owner: SessionId,
    name: Option<String>,
    backend_type: String,
    session: Arc<dyn TerminalBackendSession>,
    active: bool,
    closing: Option<Arc<SharedClose>>,
}

struct SharedCloseInner {
    result: Option<Result<(), (String, TerminalErrorCode)>>,
    wakers: Vec<std::task::Waker>,
}

struct SharedClose {
    inner: Mutex<SharedCloseInner>,
}

struct SharedCloseWait {
    shared: Arc<SharedClose>,
}

impl SharedClose {
    fn new() -> Self {
        Self {
            inner: Mutex::new(SharedCloseInner {
                result: None,
                wakers: Vec::new(),
            }),
        }
    }

    fn wait(self: &Arc<Self>) -> SharedCloseWait {
        SharedCloseWait {
            shared: Arc::clone(self),
        }
    }

    fn complete(&self, result: Result<(), TerminalError>) {
        let stored = result.map_err(|err| (err.to_string(), err.code()));
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        inner.result = Some(stored);
        let wakers = std::mem::take(&mut inner.wakers);
        drop(inner);
        for waker in wakers {
            waker.wake();
        }
    }
}

impl Future for SharedCloseWait {
    type Output = Result<(), TerminalError>;

    fn poll(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let mut inner = self
            .shared
            .inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        inner.wakers.push(cx.waker().clone());
        match &inner.result {
            Some(Ok(())) => std::task::Poll::Ready(Ok(())),
            Some(Err((message, code))) => {
                std::task::Poll::Ready(Err(TerminalError::new(message.clone(), *code)))
            }
            None => std::task::Poll::Pending,
        }
    }
}

struct Inner {
    backends: HashMap<String, Arc<dyn TerminalBackend>>,
    sessions: HashMap<TerminalSessionId, SessionRecord>,
    order: Vec<TerminalSessionId>,
    pending: HashMap<String, u32>,
    next_id: u32,
}

/// In-process registry for replaceable PTY backends and owner-scoped sessions.
///
/// Clone shares the inner registry. Clone out of the provided
/// [`Mutex<TerminalSessionService>`] before awaiting [`Self::spawn`], [`Self::signal`],
/// or [`Self::kill`].
#[derive(Clone)]
pub struct TerminalSessionService {
    inner: Arc<Mutex<Inner>>,
}

impl TerminalSessionService {
    /// Empty registry. No PTY is allocated until a backend spawn succeeds.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// A service with no backends and no sessions.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                backends: HashMap::new(),
                sessions: HashMap::new(),
                order: Vec::new(),
                pending: HashMap::new(),
                next_id: 0,
            })),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Register one backend type for this service.
    ///
    /// # Parameters
    ///
    /// * `backend_type` - Non-empty type selected by [`TerminalSpawnRequest`].
    /// * `backend` - Provider implementation.
    ///
    /// # Returns
    ///
    /// `Ok(())` when the type is stored.
    ///
    /// # Errors
    ///
    /// Empty type, or [`TerminalErrorCode::DuplicateBackend`].
    pub fn register_backend(
        &self,
        backend_type: impl Into<String>,
        backend: Arc<dyn TerminalBackend>,
    ) -> Result<(), TerminalError> {
        let backend_type = backend_type.into();
        if backend_type.is_empty() {
            return Err(TerminalError::new(
                "pty backend type must be non-empty",
                TerminalErrorCode::NoBackend,
            ));
        }
        let mut inner = self.lock();
        if inner.backends.contains_key(&backend_type) {
            return Err(TerminalError::new(
                format!("a PTY backend named \"{backend_type}\" is already registered"),
                TerminalErrorCode::DuplicateBackend,
            ));
        }
        inner.backends.insert(backend_type, backend);
        Ok(())
    }

    /// Create and publish one owner-scoped session after backend setup succeeds.
    ///
    /// Mints `pty-1`, `pty-2`, … . [`Self::has_owner_activity`] is true from this
    /// unpublished reservation through close. Drops the inner mutex, awaits the
    /// backend spawn future, then re-locks to publish.
    ///
    /// # Parameters
    ///
    /// * `owner` - Owning [`SessionId`].
    /// * `request` - Backend type plus optional owner-local name and cwd.
    ///
    /// # Returns
    ///
    /// Published identity, metadata, status, and MOTD.
    ///
    /// # Errors
    ///
    /// [`TerminalErrorCode::NoBackend`], [`TerminalErrorCode::DuplicateName`], or a backend failure.
    pub async fn spawn(
        &self,
        owner: SessionId,
        request: TerminalSpawnRequest,
    ) -> Result<TerminalSpawnResult, TerminalError> {
        let backend;
        let session_id;
        {
            let mut inner = self.lock();
            backend = inner
                .backends
                .get(request.backend_type())
                .cloned()
                .ok_or_else(|| {
                    TerminalError::new(
                        format!(
                            "no PTY backend registered for \"{}\"",
                            request.backend_type()
                        ),
                        TerminalErrorCode::NoBackend,
                    )
                })?;
            if let Some(name) = request.name() {
                if name.is_empty() {
                    return Err(TerminalError::new(
                        "PTY session name must be non-empty",
                        TerminalErrorCode::DuplicateName,
                    ));
                }
                let taken = inner
                    .sessions
                    .values()
                    .any(|record| record.owner == owner && record.name.as_deref() == Some(name));
                if taken {
                    return Err(TerminalError::new(
                        format!("PTY session name \"{name}\" already exists for this owner"),
                        TerminalErrorCode::DuplicateName,
                    ));
                }
            }
            let pending = inner.pending.entry(owner.as_str().to_string()).or_insert(0);
            *pending = pending.saturating_add(1);
            inner.next_id = inner.next_id.saturating_add(1);
            session_id = TerminalSessionId::new(format!("pty-{}", inner.next_id));
        }
        let _pending = PendingRelease {
            inner: Arc::clone(&self.inner),
            owner: owner.clone(),
        };
        let spec = TerminalBackendSpawnSpec::new(
            session_id.clone(),
            owner.clone(),
            request.backend_type().to_string(),
            request.name().map(str::to_string),
            request.cwd().map(str::to_string),
        );
        let session: Arc<dyn TerminalBackendSession> = Arc::from(backend.spawn(spec).await?);
        let motd = session.motd().to_string();
        let pid = session.pid();
        let status = session.status();
        let mut inner = self.lock();
        inner.sessions.insert(
            session_id.clone(),
            SessionRecord {
                id: session_id.clone(),
                owner: owner.clone(),
                name: request.name().map(str::to_string),
                backend_type: request.backend_type().to_string(),
                session,
                active: false,
                closing: None,
            },
        );
        inner.order.push(session_id.clone());
        Ok(TerminalSpawnResult {
            session_id,
            name: request.name().map(str::to_string),
            backend_type: request.backend_type().to_string(),
            pid,
            status,
            motd,
        })
    }

    /// Test whether an owner has a published session or unpublished spawn.
    ///
    /// # Parameters
    ///
    /// * `owner` - Owner to inspect.
    ///
    /// # Returns
    ///
    /// `true` across the entire spawn-to-close interval, with no publication gap.
    #[must_use]
    pub fn has_owner_activity(&self, owner: &SessionId) -> bool {
        let inner = self.lock();
        if inner.pending.get(owner.as_str()).copied().unwrap_or(0) > 0 {
            return true;
        }
        inner.sessions.values().any(|record| record.owner == *owner)
    }

    /// Start one exclusive interactive send.
    ///
    /// # Parameters
    ///
    /// * `owner` - Session owner.
    /// * `id` - Target PTY identity.
    /// * `request` - Explicit text and submit behavior.
    ///
    /// # Returns
    ///
    /// Live operation handle for foreground await or task registration.
    ///
    /// # Errors
    ///
    /// [`TerminalErrorCode::NoSession`], [`TerminalErrorCode::ForeignSession`], or
    /// [`TerminalErrorCode::SendActive`].
    pub fn start_send(
        &self,
        owner: &SessionId,
        id: &TerminalSessionId,
        request: TerminalSendRequest,
    ) -> Result<TerminalSendOperation, TerminalError> {
        let mut inner = self.lock();
        let record = expect_owned(&mut inner, owner, id)?;
        if record.active {
            return Err(TerminalError::new(
                format!("PTY session {id} already has an active send"),
                TerminalErrorCode::SendActive,
            ));
        }
        record.active = true;
        let operation = record.session.start_send(request);
        let inner_slot = Arc::clone(&self.inner);
        let clear_id = id.clone();
        Ok(operation.with_on_settle(Box::new(move || {
            let mut inner = inner_slot.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(record) = inner.sessions.get_mut(&clear_id) {
                record.active = false;
            }
        })))
    }

    /// Read one bounded scrollback page from an owned session.
    ///
    /// # Parameters
    ///
    /// * `owner` - Session owner.
    /// * `id` - Target PTY identity.
    /// * `request` - Optional newest-relative offset and line count.
    ///
    /// # Returns
    ///
    /// Bounded retained text and pagination metadata.
    ///
    /// # Errors
    ///
    /// [`TerminalErrorCode::NoSession`] or [`TerminalErrorCode::ForeignSession`].
    pub fn read(
        &self,
        owner: &SessionId,
        id: &TerminalSessionId,
        request: TerminalReadRequest,
    ) -> Result<TerminalReadResult, TerminalError> {
        let inner = self.lock();
        let record = expect_owned_ref(&inner, owner, id)?;
        Ok(record.session.read(request))
    }

    /// Deliver an allowed signal through an owned backend session.
    ///
    /// Drops the inner mutex before awaiting the backend signal future.
    ///
    /// # Parameters
    ///
    /// * `owner` - Session owner.
    /// * `id` - Target PTY identity.
    /// * `signal` - Allowed POSIX signal name.
    ///
    /// # Returns
    ///
    /// Delivered foreground process-group identity.
    ///
    /// # Errors
    ///
    /// [`TerminalErrorCode::NoSession`] or [`TerminalErrorCode::ForeignSession`].
    pub async fn signal(
        &self,
        owner: &SessionId,
        id: &TerminalSessionId,
        signal: TerminalSignal,
    ) -> Result<TerminalSignalResult, TerminalError> {
        let session = {
            let inner = self.lock();
            Arc::clone(&expect_owned_ref(&inner, owner, id)?.session)
        };
        Ok(session.signal(signal).await)
    }

    /// Close one owned session and remove it after backend cleanup.
    ///
    /// Drops the inner mutex before awaiting the backend close future, then
    /// re-locks to unpublish. The first closer awaits backend `close`, unpublishes,
    /// and returns `true`. A concurrent closer awaits that same close and returns
    /// `false`. After unpublish, a late kill is `unknown PTY session {id}`.
    ///
    /// # Parameters
    ///
    /// * `owner` - Session owner.
    /// * `id` - Target PTY identity.
    /// * `reason` - Diagnostic cleanup reason.
    ///
    /// # Returns
    ///
    /// `true` for a newly closed session, `false` when the same close is already in flight.
    ///
    /// # Errors
    ///
    /// [`TerminalErrorCode::NoSession`], [`TerminalErrorCode::ForeignSession`], or backend close failure.
    pub async fn kill(
        &self,
        owner: &SessionId,
        id: &TerminalSessionId,
        reason: &str,
    ) -> Result<bool, TerminalError> {
        let session;
        let shared;
        let first;
        {
            let mut inner = self.lock();
            let record = expect_owned(&mut inner, owner, id)?;
            if let Some(in_flight) = record.closing.clone() {
                session = None;
                shared = in_flight;
                first = false;
            } else {
                let in_flight = Arc::new(SharedClose::new());
                record.closing = Some(Arc::clone(&in_flight));
                session = Some(Arc::clone(&record.session));
                shared = in_flight;
                first = true;
            }
        }
        if !first {
            shared.wait().await?;
            return Ok(false);
        }
        let session = session.expect("first closer owns the backend session");
        let close_result = session.close(reason).await;
        match &close_result {
            Ok(()) => {
                let mut inner = self.lock();
                inner.sessions.remove(id);
                inner.order.retain(|published| published != id);
            }
            Err(_) => {
                let mut inner = self.lock();
                if let Some(record) = inner.sessions.get_mut(id) {
                    let still_ours = record
                        .closing
                        .as_ref()
                        .is_some_and(|slot| Arc::ptr_eq(slot, &shared));
                    if still_ours {
                        record.closing = None;
                    }
                }
            }
        }
        let shared_result = match &close_result {
            Ok(()) => Ok(()),
            Err(err) => Err(TerminalError::new(err.to_string(), err.code())),
        };
        shared.complete(shared_result);
        close_result.map(|()| true)
    }

    /// List fresh snapshots for exactly one owner.
    ///
    /// # Parameters
    ///
    /// * `owner` - Owner whose sessions are visible.
    ///
    /// # Returns
    ///
    /// Owner-visible snapshots in publication order.
    #[must_use]
    pub fn list(&self, owner: &SessionId) -> Vec<TerminalSessionSnapshot> {
        let inner = self.lock();
        inner
            .order
            .iter()
            .filter_map(|id| inner.sessions.get(id))
            .filter(|record| record.owner == *owner)
            .map(|record| TerminalSessionSnapshot {
                session_id: record.id.clone(),
                name: record.name.clone(),
                backend_type: record.backend_type.clone(),
                pid: record.session.pid(),
                status: record.session.status(),
            })
            .collect()
    }

    /// Close every published session. Used by plugin dispose.
    pub(crate) async fn dispose_all(&self) {
        let records: Vec<SessionRecord> = {
            let mut inner = self.lock();
            let drained = inner.sessions.drain().map(|(_, record)| record).collect();
            inner.order.clear();
            inner.pending.clear();
            drained
        };
        for record in records {
            let _ = record.session.close("PTY service disposed").await;
        }
    }
}

impl Default for TerminalSessionService {
    fn default() -> Self {
        Self::new()
    }
}

struct PendingRelease {
    inner: Arc<Mutex<Inner>>,
    owner: SessionId,
}

impl Drop for PendingRelease {
    fn drop(&mut self) {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        release_pending_map(&mut inner.pending, &self.owner);
    }
}

fn release_pending_map(pending: &mut HashMap<String, u32>, owner: &SessionId) {
    let key = owner.as_str();
    let empty = match pending.get_mut(key) {
        Some(count) => {
            *count = count.saturating_sub(1);
            *count == 0
        }
        None => false,
    };
    if empty {
        pending.remove(key);
    }
}

fn expect_owned<'a>(
    inner: &'a mut Inner,
    owner: &SessionId,
    id: &TerminalSessionId,
) -> Result<&'a mut SessionRecord, TerminalError> {
    let Some(record) = inner.sessions.get_mut(id) else {
        return Err(TerminalError::new(
            format!("unknown PTY session {id}"),
            TerminalErrorCode::NoSession,
        ));
    };
    if record.owner != *owner {
        return Err(TerminalError::new(
            format!("PTY session {id} belongs to another agent"),
            TerminalErrorCode::ForeignSession,
        ));
    }
    Ok(record)
}

fn expect_owned_ref<'a>(
    inner: &'a Inner,
    owner: &SessionId,
    id: &TerminalSessionId,
) -> Result<&'a SessionRecord, TerminalError> {
    let Some(record) = inner.sessions.get(id) else {
        return Err(TerminalError::new(
            format!("unknown PTY session {id}"),
            TerminalErrorCode::NoSession,
        ));
    };
    if record.owner != *owner {
        return Err(TerminalError::new(
            format!("PTY session {id} belongs to another agent"),
            TerminalErrorCode::ForeignSession,
        ));
    }
    Ok(record)
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::sync::{Arc, Mutex, PoisonError};
    use std::task::{Context, Poll, Waker};
    use std::time::Duration;

    use dsh_session::SessionId;

    use super::{
        TerminalBackend, TerminalBackendSpawnFuture, TerminalBackendSpawnSpec, TerminalErrorCode,
        TerminalSendRequest, TerminalSessionId, TerminalSessionService, TerminalSessionStatus,
        TerminalSignal, TerminalSpawnRequest, TerminalWaitReason,
    };
    use crate::SnapshotBackend;

    fn expect_ready<T>(future: impl std::future::Future<Output = T>) -> T {
        let mut future = std::pin::pin!(future);
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("send done should already be ready"),
        }
    }

    fn service_with_shell() -> TerminalSessionService {
        let service = TerminalSessionService::new();
        service
            .register_backend("shell", Arc::new(SnapshotBackend))
            .expect("register snapshot backend");
        service
    }

    #[tokio::test]
    async fn mints_pty_1() {
        let service = service_with_shell();
        let owner = SessionId::new("owner-a");
        let first = service
            .spawn(owner.clone(), TerminalSpawnRequest::new("shell"))
            .await
            .expect("first spawn");
        assert_eq!(first.session_id().as_str(), "pty-1");
        assert!(service.has_owner_activity(&owner));
        let second = service
            .spawn(owner, TerminalSpawnRequest::new("shell"))
            .await
            .expect("second spawn");
        assert_eq!(second.session_id().as_str(), "pty-2");
    }

    #[tokio::test]
    async fn snapshot_backend_send_pty_ok() {
        let service = service_with_shell();
        let owner = SessionId::new("owner-a");
        let spawned = service
            .spawn(owner.clone(), TerminalSpawnRequest::new("shell"))
            .await
            .expect("spawn");
        assert_eq!(spawned.motd(), "dsh> ");
        let mut operation = service
            .start_send(
                &owner,
                spawned.session_id(),
                TerminalSendRequest::new("hi", true),
            )
            .expect("send");
        assert!(!operation.cancel());
        let result = expect_ready(operation.done());
        assert_eq!(result.viewport(), "hi\nPTY_OK\ndsh> ");
        assert_eq!(result.wait_reason(), TerminalWaitReason::StdinRead);
        assert!(matches!(
            result.session_status(),
            TerminalSessionStatus::Running
        ));
    }

    #[tokio::test]
    async fn start_send_send_active_display() {
        let service = service_with_shell();
        let owner = SessionId::new("owner-a");
        let spawned = service
            .spawn(owner.clone(), TerminalSpawnRequest::new("shell"))
            .await
            .expect("spawn");
        let _live = service
            .start_send(
                &owner,
                spawned.session_id(),
                TerminalSendRequest::new("one", true),
            )
            .expect("first send");
        let err = service
            .start_send(
                &owner,
                spawned.session_id(),
                TerminalSendRequest::new("two", true),
            )
            .expect_err("exclusive send");
        assert_eq!(err.code(), TerminalErrorCode::SendActive);
        assert_eq!(
            err.to_string(),
            format!(
                "PTY session {} already has an active send",
                spawned.session_id()
            )
        );
    }

    #[test]
    fn unknown_id_display() {
        let service = service_with_shell();
        let owner = SessionId::new("owner-a");
        let missing = TerminalSessionId::new("pty-99");
        let err = service
            .start_send(&owner, &missing, TerminalSendRequest::new("hi", true))
            .expect_err("unknown id");
        assert_eq!(err.code(), TerminalErrorCode::NoSession);
        assert_eq!(err.to_string(), "unknown PTY session pty-99");
    }

    #[tokio::test]
    async fn foreign_session_display() {
        let service = service_with_shell();
        let owner = SessionId::new("owner-a");
        let foreign = SessionId::new("owner-b");
        let spawned = service
            .spawn(owner, TerminalSpawnRequest::new("shell"))
            .await
            .expect("spawn");
        let err = service
            .start_send(
                &foreign,
                spawned.session_id(),
                TerminalSendRequest::new("hi", true),
            )
            .expect_err("foreign owner");
        assert_eq!(err.code(), TerminalErrorCode::ForeignSession);
        assert!(err.to_string().contains(spawned.session_id().as_str()));
    }

    #[tokio::test]
    async fn duplicate_backend_and_missing_type() {
        let service = service_with_shell();
        let err = service
            .register_backend("shell", Arc::new(SnapshotBackend))
            .expect_err("duplicate");
        assert_eq!(err.code(), TerminalErrorCode::DuplicateBackend);
        assert_eq!(
            err.to_string(),
            "a PTY backend named \"shell\" is already registered"
        );
        let empty = TerminalSessionService::new()
            .register_backend("", Arc::new(SnapshotBackend))
            .expect_err("empty type");
        assert!(empty.to_string().contains("must be non-empty"));
        let owner = SessionId::new("owner-a");
        let missing = TerminalSessionService::new()
            .spawn(owner, TerminalSpawnRequest::new("shell"))
            .await
            .expect_err("no backend");
        assert_eq!(missing.code(), TerminalErrorCode::NoBackend);
        assert_eq!(
            missing.to_string(),
            "no PTY backend registered for \"shell\""
        );
    }

    struct HoldSpawn {
        release: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
    }

    impl TerminalBackend for HoldSpawn {
        fn spawn(&self, spec: TerminalBackendSpawnSpec) -> TerminalBackendSpawnFuture {
            let rx = self
                .release
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
                .expect("one spawn");
            Box::pin(async move {
                let _ = rx.await;
                SnapshotBackend.spawn(spec).await
            })
        }
    }

    #[tokio::test]
    async fn spawn_keeps_pending_across_backend_await() {
        let service = TerminalSessionService::new();
        let (tx, rx) = tokio::sync::oneshot::channel();
        service
            .register_backend(
                "held",
                Arc::new(HoldSpawn {
                    release: Mutex::new(Some(rx)),
                }),
            )
            .expect("register held backend");
        let owner = SessionId::new("owner-a");
        let task = tokio::spawn({
            let service = service.clone();
            let owner = owner.clone();
            async move {
                service
                    .spawn(owner, TerminalSpawnRequest::new("held"))
                    .await
            }
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if service.has_owner_activity(&owner) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("pending reservation visible before backend spawn settles");
        assert!(service.list(&owner).is_empty());
        tx.send(()).expect("release spawn");
        let spawned = task.await.expect("join spawn").expect("spawn");
        assert_eq!(spawned.session_id().as_str(), "pty-1");
        assert_eq!(spawned.motd(), "dsh> ");
    }

    #[tokio::test]
    async fn snapshot_signal_and_kill_await() {
        let service = service_with_shell();
        let owner = SessionId::new("owner-a");
        let spawned = service
            .spawn(owner.clone(), TerminalSpawnRequest::new("shell"))
            .await
            .expect("spawn");
        let delivered = service
            .signal(&owner, spawned.session_id(), TerminalSignal::Sigint)
            .await
            .expect("signal");
        assert!(delivered.delivered());
        assert_eq!(delivered.target_pgid(), 1);
        assert!(
            service
                .kill(&owner, spawned.session_id(), "test close")
                .await
                .expect("kill")
        );
        assert!(!service.has_owner_activity(&owner));
    }

    struct HoldClose {
        started: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
        release: Arc<Mutex<Option<tokio::sync::oneshot::Receiver<()>>>>,
    }

    struct HoldSession {
        started: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
        release: Arc<Mutex<Option<tokio::sync::oneshot::Receiver<()>>>>,
    }

    impl super::TerminalBackendSession for HoldSession {
        fn motd(&self) -> &str {
            "held"
        }

        fn pid(&self) -> Option<i32> {
            None
        }

        fn start_send(&self, request: super::TerminalSendRequest) -> super::TerminalSendOperation {
            let result = super::TerminalSendResult::new(
                request.text().to_string(),
                super::TerminalWaitReason::Timeout,
                TerminalSessionStatus::Running,
                false,
            );
            super::TerminalSendOperation::new(
                Box::pin(std::future::ready(result)),
                Box::new(|| super::TerminalSendRead::new(String::new(), false)),
                Box::new(|| false),
            )
        }

        fn read(&self, _request: super::TerminalReadRequest) -> super::TerminalReadResult {
            super::TerminalReadResult::new("", 0, 0, 0, false)
        }

        fn signal(
            &self,
            _signal: TerminalSignal,
        ) -> Pin<Box<dyn std::future::Future<Output = super::TerminalSignalResult> + Send>>
        {
            Box::pin(std::future::ready(super::TerminalSignalResult::new(
                true, 1,
            )))
        }

        fn status(&self) -> TerminalSessionStatus {
            TerminalSessionStatus::Running
        }

        fn close(
            &self,
            _reason: &str,
        ) -> Pin<Box<dyn std::future::Future<Output = Result<(), super::TerminalError>> + Send>>
        {
            let started = self
                .started
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            let release = self
                .release
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            Box::pin(async move {
                if let Some(tx) = started {
                    let _ = tx.send(());
                }
                if let Some(rx) = release {
                    let _ = rx.await;
                }
                Ok(())
            })
        }
    }

    impl TerminalBackend for HoldClose {
        fn spawn(&self, _spec: TerminalBackendSpawnSpec) -> TerminalBackendSpawnFuture {
            let started = Arc::clone(&self.started);
            let release = Arc::clone(&self.release);
            Box::pin(async move {
                Ok(Box::new(HoldSession { started, release })
                    as Box<dyn super::TerminalBackendSession>)
            })
        }
    }

    #[tokio::test]
    async fn concurrent_kill_already_closing_returns_false() {
        let service = TerminalSessionService::new();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        service
            .register_backend(
                "held",
                Arc::new(HoldClose {
                    started: Arc::new(Mutex::new(Some(started_tx))),
                    release: Arc::new(Mutex::new(Some(release_rx))),
                }),
            )
            .expect("register held");
        let owner = SessionId::new("owner-a");
        let spawned = service
            .spawn(owner.clone(), TerminalSpawnRequest::new("held"))
            .await
            .expect("spawn");
        let first = tokio::spawn({
            let service = service.clone();
            let owner = owner.clone();
            let id = spawned.session_id().clone();
            async move { service.kill(&owner, &id, "model request").await }
        });
        tokio::time::timeout(Duration::from_secs(2), started_rx)
            .await
            .expect("first closer called backend close")
            .expect("close started");
        let mut second = tokio::spawn({
            let service = service.clone();
            let owner = owner.clone();
            let id = spawned.session_id().clone();
            async move { service.kill(&owner, &id, "model request").await }
        });
        tokio::select! {
            biased;
            result = &mut second => panic!("second kill finished before release: {result:?}"),
            () = tokio::task::yield_now() => {}
        }
        release_tx.send(()).expect("release close");
        assert!(first.await.expect("join first").expect("first kill"));
        let second = second.await.expect("join second").expect("second kill");
        assert!(!second);
        let late = service
            .kill(&owner, spawned.session_id(), "model request")
            .await
            .expect_err("unpublished");
        assert_eq!(late.code(), TerminalErrorCode::NoSession);
        assert_eq!(
            late.to_string(),
            format!("unknown PTY session {}", spawned.session_id())
        );
    }
}
