//! Registry errors.

use dsh_agent_loop::LoopError;

/// Failure to look up or drive a live agent.
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    /// No agent is registered under this session id.
    #[error("unknown session `{0}`")]
    UnknownSession(String),
    /// Inbox or loop failure.
    #[error(transparent)]
    Loop(#[from] LoopError),
    /// [`AgentRegistry::resume`] was called with a session whose id does not match `options`.
    #[error("resume session id `{0}` does not match options `{1}`")]
    ResumeIdMismatch(String, String),
}
