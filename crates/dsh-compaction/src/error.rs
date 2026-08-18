//! Compaction failure codes and the engine error type.

use serde::{Deserialize, Serialize};

/// Expected failure classes for an explicit idle-session compaction request.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ManualCompactionErrorCode {
    /// A live unmatched `compaction/start` already holds the lock.
    Busy,
    /// The request was cancelled.
    Cancelled,
    /// The selected span changed before commit.
    Changed,
    /// Summarization failed or the summary did not shrink.
    Summary,
    /// Commit-stage failure after summarization.
    Commit,
    /// In-memory bracket closed but the explicit flush failed.
    Persistence,
}

/// Classified compaction failure with a backend diagnostic.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct CompactionError {
    code: ManualCompactionErrorCode,
    message: String,
}

impl CompactionError {
    /// Create one classified compaction failure.
    #[must_use]
    pub fn new(code: ManualCompactionErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Stable failure class.
    #[must_use]
    pub fn code(&self) -> ManualCompactionErrorCode {
        self.code
    }
}
