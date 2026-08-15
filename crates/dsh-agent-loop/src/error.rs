//! Loop, session, and prompt failures.

/// Failure from inbox mutation, phase transitions, prompt assembly, or the session log.
#[derive(Debug, thiserror::Error)]
pub enum LoopError {
    /// Session append, surface, or replay failure.
    #[error("{0}")]
    Session(#[from] dsh_session::SessionError),
    /// Prompt assembly or render failure.
    #[error("{0}")]
    Prompt(String),
    /// Invalid phase, splice, or a tool-call finish before Task 28.
    #[error("{0}")]
    Invalid(String),
}
