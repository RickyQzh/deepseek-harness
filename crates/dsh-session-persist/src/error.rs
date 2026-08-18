//! Persist codec errors.

use thiserror::Error;

/// Failure while encoding, decoding, or writing a session log artifact.
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
    /// Filesystem failure while creating directories or writing the log.
    #[error("session store io: {0}")]
    Io(String),
}
