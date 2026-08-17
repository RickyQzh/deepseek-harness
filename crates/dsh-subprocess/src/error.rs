//! Subprocess errors.

/// Upper bound for [`crate::types::SubprocessSpawnSpec::grace_ms`].
pub const MAX_GRACE_MS: u64 = 2_147_483_647;

/// Spawn, lookup, and platform failures for this crate.
#[derive(Clone, Debug, thiserror::Error)]
pub enum SubprocessError {
    /// `argv` is empty or `argv[0]` is empty.
    #[error("invalid argv: expected a non-empty program name at argv[0]")]
    InvalidArgv,
    /// `grace_ms` is zero or greater than [`MAX_GRACE_MS`].
    #[error("subprocess graceMs must be a positive finite number no greater than {MAX_GRACE_MS}")]
    InvalidGrace,
    /// The abort flag was already set when spawn was requested.
    #[error("aborted before spawn")]
    AbortedBeforeSpawn,
    /// The resolved executable path is empty.
    #[error("subprocess-local: executable must be non-empty")]
    EmptyExecutable,
    /// The command is a relative path rather than an absolute path or a bare PATH name.
    #[error(
        "subprocess-local: command {0:?} is a relative path; use an absolute path or a bare PATH name"
    )]
    RelativePath(String),
    /// The command names a path that is not an executable file.
    #[error("subprocess-local: command {0:?} is not an executable file")]
    NotExecutable(String),
    /// The bare command name was not found on PATH.
    #[error("subprocess-local: command {0:?} was not found on PATH")]
    NotOnPath(String),
    /// The OS refused to spawn the process.
    #[error("spawn failed: {0}")]
    Spawn(String),
    /// This host does not implement process-tree spawn.
    #[error("unsupported on this platform")]
    UnsupportedPlatform,
    /// Terminal process-table inspection is unsupported on this OS.
    #[error("subprocess-local: terminal inspection is unsupported on platform {platform}")]
    UnsupportedInspection {
        /// OS name that has no inspector (`std::env::consts::OS` or a test platform string).
        platform: String,
    },
}
