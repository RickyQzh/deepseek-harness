//! Local [`TerminalSessionId`] newtype around [`dsh_brand::Branded`].
//!
//! `new` is crate-private. Callers mint ids through [`crate::TerminalSessionService::spawn`].
//! Tools reconstruct an existing id from JSON with [`TerminalSessionId::from_arg`].

use std::fmt;

use dsh_brand::Branded;

/// Tag for [`TerminalSessionId`].
pub struct TerminalSessionIdTag;

/// Registry-issued PTY session id (`pty-N`).
///
/// Ids are predictable; owner authorization is the access boundary.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TerminalSessionId(Branded<TerminalSessionIdTag>);

impl TerminalSessionId {
    /// Brand `value` as a PTY session id. No format check is performed.
    #[must_use]
    pub(crate) fn new(value: impl Into<String>) -> Self {
        Self(Branded::new(value))
    }

    /// Reconstruct a session id from tool JSON. Empty string fails; no format check.
    ///
    /// Minting new ids stays [`crate::TerminalSessionService::spawn`].
    ///
    /// # Parameters
    ///
    /// * `value` - `sessionId` argument text.
    ///
    /// # Returns
    ///
    /// The branded id.
    ///
    /// # Errors
    ///
    /// [`crate::TerminalErrorCode::NoSession`] when `value` is empty
    /// (`sessionId must be a non-empty string`).
    pub fn from_arg(value: impl Into<String>) -> Result<Self, crate::TerminalError> {
        let value = value.into();
        if value.is_empty() {
            return Err(crate::TerminalError::new(
                "sessionId must be a non-empty string",
                crate::TerminalErrorCode::NoSession,
            ));
        }
        Ok(Self::new(value))
    }

    /// Borrow the inner string.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The registry-issued id text such as `pty-1`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for TerminalSessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl AsRef<str> for TerminalSessionId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::TerminalSessionId;
    use crate::TerminalErrorCode;

    #[test]
    fn from_arg_rejects_empty_string() {
        let err = TerminalSessionId::from_arg("").expect_err("empty");
        assert_eq!(err.to_string(), "sessionId must be a non-empty string");
        assert_eq!(err.code(), TerminalErrorCode::NoSession);
    }

    #[test]
    fn from_arg_brands_without_format_check() {
        let id = TerminalSessionId::from_arg("pty-99").expect("brand");
        assert_eq!(id.as_str(), "pty-99");
        let weird = TerminalSessionId::from_arg("not-a-pty").expect("brand");
        assert_eq!(weird.as_str(), "not-a-pty");
    }
}
