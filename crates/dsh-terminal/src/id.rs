//! Local [`TerminalSessionId`] newtype around [`dsh_brand::Branded`].
//!
//! `new` is crate-private. Callers mint ids through [`crate::TerminalSessionService::spawn`].

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
