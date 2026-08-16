//! Spawn spec types.

use dsh_tools::AbortFlag;

use crate::env::EnvEntry;

/// stdin disposition. `Ignore` leaves fd 0 on `/dev/null`; `Pipe` exposes the child stdin for ongoing writes; `Data` writes the bytes and closes.
#[derive(Clone, Debug)]
pub enum SubprocessStdin {
    /// Leave stdin on `/dev/null`.
    Ignore,
    /// Expose a writable stdin pipe to the caller.
    Pipe,
    /// Write these bytes and close stdin.
    Data(String),
}

/// Bounded in-memory collection for one output stream, with an optional full-stream spill file.
#[derive(Clone, Debug)]
pub struct SubprocessCollect {
    /// In-memory cap in bytes; overflow keeps the tail.
    pub max_bytes: usize,
    /// Whole-stream spill cap in bytes; `None` disables spilling.
    pub spill_max_bytes: Option<usize>,
}

/// stdout/stderr disposition. This crate applies no defaults: the caller picks `Pipe`, `Inherit`, or `Collect`.
#[derive(Clone, Debug)]
pub enum SubprocessOutput {
    /// Expose the raw readable stream to the caller.
    Pipe,
    /// Pass the parent's descriptor through.
    Inherit,
    /// Buffer boundedly with offset-based reads.
    Collect(SubprocessCollect),
}

/// Per-stream stdio dispositions. This seam applies no defaults: every stream is explicit.
#[derive(Clone, Debug)]
pub struct SubprocessStdio {
    /// stdin disposition.
    pub stdin: SubprocessStdin,
    /// stdout disposition.
    pub stdout: SubprocessOutput,
    /// stderr disposition.
    pub stderr: SubprocessOutput,
}

/// A fully specified spawn request. This seam applies no defaults: every stdio disposition and `grace_ms` is explicit.
#[derive(Clone, Debug)]
pub struct SubprocessSpawnSpec {
    /// Executable and arguments; `argv[0]` is the program. Never a shell string.
    pub argv: Vec<String>,
    /// Working directory for the child.
    pub cwd: String,
    /// Per-stream stdio dispositions.
    pub stdio: SubprocessStdio,
    /// Positive grace period in milliseconds, no greater than [`crate::MAX_GRACE_MS`].
    pub grace_ms: u64,
    /// Cooperative cancellation; abort starts terminate escalation. `None` means no abort signal.
    pub signal: Option<AbortFlag>,
    /// Explicit environment overlay merged after the scrubbed parent; `None` means no overlay.
    pub env: Option<Vec<EnvEntry>>,
}

/// Exit facts of one closed process. Carries no timeout or cancellation classification and no output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubprocessOutcome {
    /// Exit code; `None` when the process died from a signal.
    pub exit_code: Option<i32>,
    /// Terminating signal number; `None` on normal exit.
    pub signal: Option<i32>,
}

/// One incremental collected-output read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubprocessOutputRead {
    /// Stream text from the requested offset (the whole retained tail when lossy).
    pub text: String,
    /// Whole-stream offset to resume from on the next read.
    pub next_offset: u64,
    /// True when the requested offset slid out of the in-memory tail window.
    pub lossy: bool,
    /// Path to the full-stream spill file, when one was created and remains intact.
    pub spill_path: Option<String>,
}

/// One captured stream: the (possibly truncated) text plus recovery info.
///
/// [`crate::OutputCollector::finalize`] produces this after the stream ends.
pub struct CollectedOutput {
    /// Collected text — the tail of the stream when truncated.
    pub text: String,
    /// True when bytes were dropped from `text`.
    pub truncated: bool,
    /// Path to a file holding the complete stream, when truncated and available.
    pub spill_path: Option<String>,
}
