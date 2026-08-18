//! Shared job types for producers, the registry, and controllers.

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use dsh_session::SessionId;

use crate::JobId;

/// Lifecycle: `running`, optionally `stopping`, then exactly one terminal status.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum JobStatus {
    /// Producer work is live.
    Running,
    /// Cancellation was requested; the producer has not settled yet.
    Stopping,
    /// Producer finished successfully.
    Completed,
    /// Producer ended after cancellation.
    Killed,
    /// Producer broke, or `done` panicked.
    Failed,
}

impl JobStatus {
    /// Whether this status is terminal (`completed`, `killed`, or `failed`).
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Killed | Self::Failed)
    }

    /// Lowercase status token used in model-facing text.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Completed => "completed",
            Self::Killed => "killed",
            Self::Failed => "failed",
        }
    }
}

impl fmt::Display for JobStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Producer kind; also the id prefix (`bash` / `subagent` / `pty-send`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum JobKind {
    /// Shell / bash producer namespace.
    Bash,
    /// Subagent producer namespace.
    Subagent,
    /// Persistent-terminal send producer namespace.
    PtySend,
}

impl JobKind {
    /// Id prefix for this kind.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Subagent => "subagent",
            Self::PtySend => "pty-send",
        }
    }
}

impl fmt::Display for JobKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Terminal result supplied by a producer through [`JobHooks::done`].
pub struct JobOutcome {
    /// How the job ended. Non-terminal values are recorded as [`JobStatus::Failed`].
    pub status: JobStatus,
    /// Kind-specific detail rendered into status lines.
    pub detail: Option<String>,
    /// Final output for jobs without `read_output`.
    pub output: Option<String>,
}

/// Hooks through which the registry controls and observes producer work.
pub struct JobHooks {
    /// Request termination. Must be synchronous, idempotent, and eventually settle [`Self::done`].
    pub cancel: Box<dyn Fn(Option<String>) + Send>,
    /// Resolves after the producer releases its resources. A panicked future is recorded as failed.
    pub done: Pin<Box<dyn Future<Output = JobOutcome> + Send>>,
    /// Consume output produced since the previous call. Absence marks a final-output-only job.
    pub read_output: Option<Box<dyn Fn() -> String + Send>>,
}

/// Producer declaration passed to [`JobRegistry::start`].
pub struct JobStart {
    /// Producer kind — also the id prefix.
    pub kind: JobKind,
    /// One-line model-facing label.
    pub label: String,
    /// Owning session used for authorization. `None` is an unowned job, open to any caller.
    pub owner_session: Option<SessionId>,
    /// Start the work after admission and synchronously return its hooks. Called once.
    ///
    /// Must not re-enter the registry that is starting this job. The process-local
    /// provider holds its mutex through `run()`.
    pub run: Box<dyn FnOnce() -> JobHooks + Send>,
}

/// Read-only projection of one job. Fresh per call; never live registry state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobSnapshot {
    id: JobId,
    kind: JobKind,
    label: String,
    status: JobStatus,
    detail: Option<String>,
    started_at: i64,
    finished_at: Option<i64>,
    reported: bool,
    owner_session: Option<SessionId>,
}

impl JobSnapshot {
    /// Build a snapshot from record fields.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: JobId,
        kind: JobKind,
        label: String,
        status: JobStatus,
        detail: Option<String>,
        started_at: i64,
        finished_at: Option<i64>,
        reported: bool,
        owner_session: Option<SessionId>,
    ) -> Self {
        Self {
            id,
            kind,
            label,
            status,
            detail,
            started_at,
            finished_at,
            reported,
            owner_session,
        }
    }

    /// Registry-issued id.
    #[must_use]
    pub fn id(&self) -> &JobId {
        &self.id
    }

    /// Producer kind.
    #[must_use]
    pub fn kind(&self) -> JobKind {
        self.kind
    }

    /// Producer-supplied one-line label.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Current lifecycle status.
    #[must_use]
    pub fn status(&self) -> JobStatus {
        self.status
    }

    /// Kind-specific status detail, when the producer supplied one.
    #[must_use]
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    /// Wall-clock epoch milliseconds when the job was registered.
    #[must_use]
    pub fn started_at(&self) -> i64 {
        self.started_at
    }

    /// Wall-clock epoch milliseconds when the job settled; absent while live.
    #[must_use]
    pub fn finished_at(&self) -> Option<i64> {
        self.finished_at
    }

    /// Whether a kill, read, wait, or teardown cancel has reported or committed to report the terminal state.
    #[must_use]
    pub fn reported(&self) -> bool {
        self.reported
    }

    /// Owner session used for authorization; absent for unowned jobs.
    #[must_use]
    pub fn owner_session(&self) -> Option<&SessionId> {
        self.owner_session.as_ref()
    }
}

/// Output and post-read state returned by [`JobRegistry::read`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobRead {
    /// Stream kinds: consuming delta. Final-output kinds: empty while live, terminal output once settled.
    pub text: String,
    /// Job state at read time.
    pub snapshot: JobSnapshot,
}

/// Result of [`JobRegistry::kill`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KillResult {
    /// Cancellation was forwarded to a live producer.
    Requested,
    /// The job was already terminal.
    AlreadyFinished,
}

/// Registry and access failure. [`Display`](std::fmt::Display) is the model-facing message.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct JobError {
    /// Human-readable failure summary.
    pub message: String,
}

impl JobError {
    /// Construct a failure with `message`.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Abstract background job registry. The process-local provider is `dsh-jobs-local`.
pub trait JobRegistry: Send + Sync {
    /// Admit, invoke `run`, and register the job. Returns the issued `<kind>-N` id.
    ///
    /// `run` is synchronous and must not re-enter this registry.
    ///
    /// # Errors
    ///
    /// Empty label, per-owner active limit, or other admission failure. A panic from `run` is not caught.
    fn start(&self, spec: JobStart) -> Result<JobId, JobError>;

    /// List caller-owned and unowned jobs in registration order.
    ///
    /// `caller` `None` sees only unowned jobs.
    fn list(&self, caller: Option<&SessionId>) -> Vec<JobSnapshot>;

    /// Non-consuming snapshot. Does not change the read cursor or notice state.
    ///
    /// # Errors
    ///
    /// Unknown id, or a job owned by another session.
    fn get(&self, id: &JobId, caller: Option<&SessionId>) -> Result<JobSnapshot, JobError>;

    /// Read the next stream delta, or the idempotent final output after settlement.
    ///
    /// A terminal read marks the job reported.
    ///
    /// # Errors
    ///
    /// Unknown id, or a job owned by another session.
    fn read(&self, id: &JobId, caller: Option<&SessionId>) -> Result<JobRead, JobError>;

    /// Request cancellation, then mark the job stopping and reported.
    ///
    /// A panic from producer `cancel` leaves lifecycle unchanged and is returned as [`JobError`].
    ///
    /// # Errors
    ///
    /// Unknown id, a job owned by another session, or a panicking `cancel`.
    fn kill(
        &self,
        id: &JobId,
        caller: Option<&SessionId>,
        reason: Option<String>,
    ) -> Result<KillResult, JobError>;
}

#[cfg(test)]
mod tests {
    use super::JobKind;

    #[test]
    fn job_kind_pty_send_prefix() {
        assert_eq!(JobKind::PtySend.as_str(), "pty-send");
        assert_eq!(JobKind::PtySend.to_string(), "pty-send");
    }
}
