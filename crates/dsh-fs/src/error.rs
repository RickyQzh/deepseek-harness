//! Filesystem error type and stable `FS_*` routing codes.

/// Stable, machine-routable codes for filesystem failures.
///
/// `FS_SANDBOX_DENIED` is an in-process fence refusal. `FS_PERMISSION_DENIED` is
/// a kernel or OS permission failure. They are not interchangeable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FsErrorCode {
    /// `FS_NOT_FOUND`
    NotFound,
    /// `FS_NOT_DIRECTORY`
    NotDirectory,
    /// `FS_NOT_TEXT`
    NotText,
    /// `FS_NOT_REGULAR_FILE`
    NotRegularFile,
    /// `FS_TOO_LARGE`
    TooLarge,
    /// `FS_PERMISSION_DENIED`
    PermissionDenied,
    /// `FS_SANDBOX_DENIED`
    SandboxDenied,
    /// `FS_IO_ERROR`
    IoError,
    /// `FS_STALE_VERSION`
    StaleVersion,
    /// `FS_NOT_OBSERVED`
    NotObserved,
    /// `FS_AMBIGUOUS_EDIT`
    AmbiguousEdit,
    /// `FS_EDIT_NOT_FOUND`
    EditNotFound,
    /// `FS_ABORTED`
    Aborted,
}

impl FsErrorCode {
    /// TypeScript wire string for this code.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotFound => "FS_NOT_FOUND",
            Self::NotDirectory => "FS_NOT_DIRECTORY",
            Self::NotText => "FS_NOT_TEXT",
            Self::NotRegularFile => "FS_NOT_REGULAR_FILE",
            Self::TooLarge => "FS_TOO_LARGE",
            Self::PermissionDenied => "FS_PERMISSION_DENIED",
            Self::SandboxDenied => "FS_SANDBOX_DENIED",
            Self::IoError => "FS_IO_ERROR",
            Self::StaleVersion => "FS_STALE_VERSION",
            Self::NotObserved => "FS_NOT_OBSERVED",
            Self::AmbiguousEdit => "FS_AMBIGUOUS_EDIT",
            Self::EditNotFound => "FS_EDIT_NOT_FOUND",
            Self::Aborted => "FS_ABORTED",
        }
    }
}

/// Typed filesystem error carrying a stable [`FsErrorCode`].
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct FsError {
    /// Human-readable failure text.
    pub message: String,
    /// Stable `FS_*` routing code.
    pub code: FsErrorCode,
}

impl FsError {
    /// Build an error from `message` and `code`.
    #[must_use]
    pub fn new(message: impl Into<String>, code: FsErrorCode) -> Self {
        Self {
            message: message.into(),
            code,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FsError, FsErrorCode};

    #[test]
    fn codes_are_the_typescript_wire_strings() {
        assert_eq!(FsErrorCode::NotFound.as_str(), "FS_NOT_FOUND");
        assert_eq!(FsErrorCode::NotDirectory.as_str(), "FS_NOT_DIRECTORY");
        assert_eq!(FsErrorCode::NotText.as_str(), "FS_NOT_TEXT");
        assert_eq!(FsErrorCode::NotRegularFile.as_str(), "FS_NOT_REGULAR_FILE");
        assert_eq!(FsErrorCode::TooLarge.as_str(), "FS_TOO_LARGE");
        assert_eq!(
            FsErrorCode::PermissionDenied.as_str(),
            "FS_PERMISSION_DENIED"
        );
        assert_eq!(FsErrorCode::SandboxDenied.as_str(), "FS_SANDBOX_DENIED");
        assert_eq!(FsErrorCode::IoError.as_str(), "FS_IO_ERROR");
        assert_eq!(FsErrorCode::StaleVersion.as_str(), "FS_STALE_VERSION");
        assert_eq!(FsErrorCode::NotObserved.as_str(), "FS_NOT_OBSERVED");
        assert_eq!(FsErrorCode::AmbiguousEdit.as_str(), "FS_AMBIGUOUS_EDIT");
        assert_eq!(FsErrorCode::EditNotFound.as_str(), "FS_EDIT_NOT_FOUND");
        assert_eq!(FsErrorCode::Aborted.as_str(), "FS_ABORTED");
        assert_ne!(
            FsErrorCode::SandboxDenied.as_str(),
            FsErrorCode::PermissionDenied.as_str()
        );
    }

    #[test]
    fn error_displays_message_and_keeps_code() {
        let err = FsError::new(
            "edit requires reading \"a.txt\" first",
            FsErrorCode::NotObserved,
        );
        assert_eq!(err.code, FsErrorCode::NotObserved);
        assert_eq!(err.to_string(), "edit requires reading \"a.txt\" first");
    }
}
