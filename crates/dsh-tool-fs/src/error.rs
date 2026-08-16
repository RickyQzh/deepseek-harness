//! Model-facing remediation for guarded-mutation filesystem failures.

use dsh_fs::{FsError, FsErrorCode};
use dsh_tools::ToolError;

const STALE_REMEDY: &str = " — re-read the file, then retry";
const UNOBSERVED_REMEDY: &str = " — read the file, then retry";

/// Append the recovery instruction for stale and unobserved mutation failures.
///
/// `FS_STALE_VERSION` gains ` — re-read the file, then retry`.
/// `FS_NOT_OBSERVED` gains ` — read the file, then retry`.
/// Other codes are returned unchanged. A message that already ends with the
/// suffix is left as-is.
#[must_use]
pub fn remediate_fs_error(error: FsError) -> FsError {
    let suffix = match error.code {
        FsErrorCode::StaleVersion => STALE_REMEDY,
        FsErrorCode::NotObserved => UNOBSERVED_REMEDY,
        _ => return error,
    };
    if error.message.ends_with(suffix) {
        return error;
    }
    FsError::new(format!("{}{suffix}", error.message), error.code)
}

/// Map a filesystem error to [`ToolError::Coded`] after remediation.
#[must_use]
pub(crate) fn coded_fs_error(error: FsError) -> ToolError {
    let err = remediate_fs_error(error);
    ToolError::Coded {
        message: err.message.clone(),
        name: "FsError".into(),
        code: err.code.as_str().into(),
    }
}

#[cfg(test)]
mod tests {
    use super::remediate_fs_error;
    use dsh_fs::{FsError, FsErrorCode};

    #[test]
    fn stale_version_appends_reread_once() {
        let err = FsError::new(
            "cannot write \"a.txt\": file changed since it was read",
            FsErrorCode::StaleVersion,
        );
        let once = remediate_fs_error(err);
        assert_eq!(
            once.message,
            "cannot write \"a.txt\": file changed since it was read — re-read the file, then retry"
        );
        let twice = remediate_fs_error(once);
        assert_eq!(
            twice.message,
            "cannot write \"a.txt\": file changed since it was read — re-read the file, then retry"
        );
    }

    #[test]
    fn not_observed_appends_read_once() {
        let err = FsError::new(
            "edit requires reading \"a.txt\" first",
            FsErrorCode::NotObserved,
        );
        let once = remediate_fs_error(err);
        assert_eq!(
            once.message,
            "edit requires reading \"a.txt\" first — read the file, then retry"
        );
        assert_eq!(once.code, FsErrorCode::NotObserved);
        let twice = remediate_fs_error(once);
        assert_eq!(
            twice.message,
            "edit requires reading \"a.txt\" first — read the file, then retry"
        );
    }

    #[test]
    fn other_codes_are_unchanged() {
        let err = FsError::new("cannot read \"a.txt\": not found", FsErrorCode::NotFound);
        let out = remediate_fs_error(err);
        assert_eq!(out.message, "cannot read \"a.txt\": not found");
        assert_eq!(out.code, FsErrorCode::NotFound);
    }
}
