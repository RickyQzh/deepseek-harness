//! Sandbox error types and the `SANDBOX_UNAVAILABLE` routing code.

use crate::types::SandboxMode;

/// Structured routing code when a requested confined mode has no usable backend.
pub const SANDBOX_UNAVAILABLE: &str = "SANDBOX_UNAVAILABLE";

/// Fail-closed sandbox failure: missing backend or malformed escalation / policy.
#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    /// Requested confined mode has no usable backend on this host.
    #[error("{message}")]
    Unavailable {
        /// Refusal text, including the Windows ACL clause and optional runner detail.
        message: String,
    },
    /// Malformed escalation arguments, a non-widening request, or a non-confinable policy.
    #[error("{0}")]
    Invalid(String),
}

impl SandboxError {
    /// Build the TypeScript unavailable refusal for `mode`, appending runner `detail` when present.
    #[must_use]
    pub fn unavailable(mode: SandboxMode, detail: Option<&str>) -> Self {
        let mut message = format!(
            "sandbox mode \"{}\" is requested but no sandbox backend is usable on this host; refusing to run the command unconfined. Install bubblewrap or run a Landlock-enforcing kernel (Linux), ensure sandbox-exec is usable (macOS), or ensure the ACL restricted-token runner can start (Windows) — otherwise switch the consumer to danger-full-access.",
            mode.as_str()
        );
        if let Some(detail) = detail {
            message.push_str(" Runner failure: ");
            message.push_str(detail);
        }
        Self::Unavailable { message }
    }

    /// Structured routing code. [`SANDBOX_UNAVAILABLE`] for [`Self::Unavailable`] only.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unavailable { .. } => SANDBOX_UNAVAILABLE,
            Self::Invalid(_) => "",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SANDBOX_UNAVAILABLE, SandboxError, SandboxMode};

    #[test]
    fn unavailable_carries_the_structured_code() {
        let err = SandboxError::unavailable(SandboxMode::ReadOnly, None);
        assert_eq!(err.code(), SANDBOX_UNAVAILABLE);
        let message = err.to_string();
        assert!(message.contains("\"read-only\""));
        assert!(message.contains("danger-full-access"));
        assert!(!message.contains("Runner failure"));
    }

    #[test]
    fn unavailable_appends_runner_detail() {
        let err = SandboxError::unavailable(
            SandboxMode::ReadOnly,
            Some("landlock-run: landlock is not enforced by this kernel"),
        );
        assert_eq!(err.code(), SANDBOX_UNAVAILABLE);
        assert!(
            err.to_string()
                .contains("Runner failure: landlock-run: landlock is not enforced by this kernel")
        );
    }

    #[test]
    fn invalid_does_not_carry_the_unavailable_code() {
        let err = SandboxError::Invalid("invalid escalation: expected a non-empty sentence".into());
        assert_ne!(err.code(), SANDBOX_UNAVAILABLE);
        assert!(err.to_string().contains("non-empty sentence"));
    }
}
