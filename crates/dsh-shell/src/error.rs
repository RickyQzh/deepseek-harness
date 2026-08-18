//! Shell request and execution failures.

/// Failures from an unusable request, subprocess spawn, or sandbox confinement.
#[derive(Debug, thiserror::Error)]
pub enum ShellError {
    /// Caller-supplied request or spec is unusable.
    #[error("{0}")]
    Invalid(String),
    /// Underlying subprocess failure.
    #[error(transparent)]
    Subprocess(#[from] dsh_subprocess::SubprocessError),
    /// Underlying sandbox failure.
    #[error(transparent)]
    Sandbox(#[from] dsh_sandbox::SandboxError),
}
