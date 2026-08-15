//! Persist codec errors.

use thiserror::Error;

/// Failure while encoding or decoding a session log artifact.
#[derive(Debug, Error)]
pub enum PersistError {
    /// Format version or unknown required event type.
    #[error(transparent)]
    Format(#[from] dsh_session::SessionFormatError),
    /// Malformed known event or storage row.
    #[error(transparent)]
    Session(#[from] dsh_session::SessionError),
    /// Corrupt header, event line, or Zstandard frame.
    #[error("{0}")]
    Corrupt(String),
}
