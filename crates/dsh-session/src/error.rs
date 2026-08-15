//! Session format and validation errors.

use thiserror::Error;

use crate::SESSION_FORMAT_VERSION;

/// The stored log is intact but this runtime cannot faithfully interpret it.
#[derive(Clone, Debug, Eq, PartialEq, Error)]
pub enum SessionFormatError {
    /// Unsupported format version or unknown required event type.
    #[error("{0}")]
    Unsupported(String),
}

/// Session validation, surface, or decode failure.
#[derive(Clone, Debug, Eq, PartialEq, Error)]
pub enum SessionError {
    /// Format version or unknown required type.
    #[error(transparent)]
    Format(#[from] SessionFormatError),
    /// Malformed known event or storage row.
    #[error("{0}")]
    Corrupt(String),
    /// Surface eligibility, provenance, range, or tool-result rewrite failure.
    #[error("{0}")]
    Surface(String),
    /// Event `seq` is not the next contiguous log index.
    #[error("{0}")]
    Append(String),
}

/// Direction-aware refusal text for a stored session whose format version this build does not read.
#[must_use]
pub fn session_format_version_refusal(id: &str, version: i64) -> String {
    if version > i64::from(SESSION_FORMAT_VERSION) {
        format!(
            "session \"{id}\" uses log format v{version}, but this harness reads only v{SESSION_FORMAT_VERSION}: the log was written by a newer harness — upgrade the harness to open it"
        )
    } else {
        format!(
            "session \"{id}\" uses log format v{version}, older than the supported v{SESSION_FORMAT_VERSION}, and this build ships no upgrade path for it"
        )
    }
}

/// Refusal text for an unrecognized required event type.
#[must_use]
pub fn unknown_required_event_refusal(id: &str, type_name: &str, seq: u64) -> String {
    format!(
        "session \"{id}\" contains event type \"{type_name}\" (seq {seq}) unknown to this harness and not marked ignorable; refusing to interpret the log — it was likely written by a newer harness"
    )
}
