//! Filesystem identity, metadata, write/edit outcomes, and observation types.

use dsh_brand::Branded;

/// Tag for [`FsTargetKey`].
pub struct FsTargetKeyTag;

/// Opaque key for stale guards and target lookup.
///
/// Branding does not trim or validate; the inner string is stored as given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FsTargetKey(Branded<FsTargetKeyTag>);

impl FsTargetKey {
    /// Brand `value` as a target key.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(Branded::new(value))
    }

    /// Borrow the inner string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// Tag for [`FsVersion`].
pub struct FsVersionTag;

/// Opaque freshness token a write or edit guards against.
///
/// Branding does not trim or validate; the inner string is stored as given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FsVersion(Branded<FsVersionTag>);

impl FsVersion {
    /// Brand `value` as a version token.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(Branded::new(value))
    }

    /// Borrow the inner string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// A path resolved by a backend into a stable identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FsTarget {
    /// Opaque key for stale guards and target lookup.
    pub target_key: FsTargetKey,
    /// Path for model/UI-facing output.
    pub display_path: String,
}

/// One authoritative observation of a target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FsObservation {
    /// The target exists at `version`.
    Present {
        /// Freshness token used by guarded replacement.
        version: FsVersion,
    },
    /// Confirmed absence; authorizes a guarded create, never an edit.
    Absent,
}

/// Kind of filesystem object reported by metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FsInfoType {
    /// Regular file.
    File,
    /// Directory.
    Directory,
    /// Neither a regular file nor a directory.
    Other,
}

/// Metadata about a target; never content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FsInfo {
    /// Opaque freshness token of the target right now.
    pub version: FsVersion,
    /// Whether the target is a regular file, a directory, or something else.
    pub kind: FsInfoType,
    /// Byte size of a regular file, when the backend can report it.
    pub size: Option<u64>,
}

/// Guarded write intent. Omitting an intent is unconditional create-or-overwrite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FsWriteIntent {
    /// Create a missing target; reject an existing one with `FS_NOT_OBSERVED`.
    CreateIfAbsent,
    /// Replace only at the observed version; otherwise `FS_STALE_VERSION`.
    ReplaceIfVersion {
        /// Observed version the write must match.
        version: FsVersion,
    },
}

/// Outcome of a full-file write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FsWriteOutcome {
    /// `"create"` or `"update"`.
    pub operation: &'static str,
    /// Opaque version of the file after the write.
    pub version: FsVersion,
    /// Content before the write, when the backend supplies a diff basis.
    pub before: Option<String>,
    /// Content after the write.
    pub after: String,
}

/// A literal-replacement edit request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FsEditRequest {
    /// Literal text to replace.
    pub old_string: String,
    /// Literal replacement text.
    pub new_string: String,
    /// Replace every match instead of requiring exactly one.
    pub replace_all: bool,
}

/// Outcome of a literal edit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FsEditOutcome {
    /// Opaque version of the file after the edit.
    pub version: FsVersion,
    /// Content before the edit.
    pub before: String,
    /// Content after the edit.
    pub after: String,
}

#[cfg(test)]
mod tests {
    use super::FsTargetKey;

    #[test]
    fn target_key_new_round_trips_without_trimming() {
        assert_eq!(FsTargetKey::new(" /tmp/a ").as_str(), " /tmp/a ");
    }
}
