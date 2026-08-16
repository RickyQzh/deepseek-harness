//! Sandbox mode, policy, and confined-argv types.

use dsh_session::SessionId;

use crate::error::SandboxError;

/// File-effect mode for one execution. Network and process visibility are outside this enum.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxMode {
    /// Required sinks only; no workspace or temp writes.
    ReadOnly,
    /// Workspace root plus platform temp areas may be written.
    WorkspaceWrite,
    /// No confinement; callers must not wrap argv.
    DangerFullAccess,
}

impl SandboxMode {
    /// Kebab-case spelling used on the wire and in diagnostics.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
            Self::DangerFullAccess => "danger-full-access",
        }
    }

    /// Parse a kebab-case mode spelling. Unknown strings return [`None`].
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "read-only" => Some(Self::ReadOnly),
            "workspace-write" => Some(Self::WorkspaceWrite),
            "danger-full-access" => Some(Self::DangerFullAccess),
            _ => None,
        }
    }
}

/// A confining mode. [`SandboxPolicy::confined`] rejects [`SandboxMode::DangerFullAccess`].
pub type ConfinedSandboxMode = SandboxMode;

/// How completely a selected backend enforces the policy's file effects.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxEnforcement {
    /// Every promised file effect is governed.
    Full,
    /// An active backend or older kernel ABI cannot govern every promised file effect.
    Partial,
}

/// Complete file-effect policy for one capability call, including `danger-full-access`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxExecutionPolicy {
    /// File-effect mode this execution runs under.
    pub mode: SandboxMode,
    /// Absolute root `workspace-write` may write under. Carried under every mode.
    pub workspace_root: String,
    /// Calling session identity; absent for agentless calls.
    pub session_id: Option<SessionId>,
}

/// Confined file-effect policy: [`mode`](Self::mode) is never [`SandboxMode::DangerFullAccess`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxPolicy {
    /// File-effect mode this confined execution runs under.
    pub mode: SandboxMode,
    /// Absolute root `workspace-write` may write under.
    pub workspace_root: String,
    /// Calling session identity; absent for agentless calls.
    pub session_id: Option<SessionId>,
}

impl SandboxPolicy {
    /// Accept a policy whose mode is confinable.
    ///
    /// # Errors
    ///
    /// Returns [`SandboxError::Invalid`] when `policy.mode` is [`SandboxMode::DangerFullAccess`].
    /// Callers must not wrap argv for that mode.
    pub fn confined(policy: SandboxExecutionPolicy) -> Result<Self, SandboxError> {
        match policy.mode {
            SandboxMode::DangerFullAccess => Err(SandboxError::Invalid(
                "danger-full-access cannot be confined; callers must not call confine() for that mode"
                    .into(),
            )),
            SandboxMode::ReadOnly | SandboxMode::WorkspaceWrite => Ok(Self {
                mode: policy.mode,
                workspace_root: policy.workspace_root,
                session_id: policy.session_id,
            }),
        }
    }
}

/// Evidence that identifies a sandbox runner failing before it executes the wrapped command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunnerFailureRule {
    /// Nonzero process exit codes on which this rule may match; [`None`] permits any nonzero exit.
    pub allowed_exit_codes: Option<Vec<i32>>,
    /// Non-empty substrings identifying a fatal runner diagnostic on one stderr line.
    pub fatal_signatures: Vec<String>,
    /// Benign stderr lines excluded by exact full-line equality before fatal matching.
    pub informational_lines: Vec<String>,
}

/// Argv to spawn in place of the caller's own, plus the selected backend's enforcement facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfinedArgv {
    /// Wrapped argv (runner, profile, separator, then the caller's argv).
    pub argv: Vec<String>,
    /// How completely the selected backend enforces the policy's file effects.
    pub enforcement: SandboxEnforcement,
    /// Case-insensitive stderr substrings this backend produces on a denied file effect.
    pub denial_signatures: Vec<String>,
    /// Structured runner-failure evidence rules for this backend.
    pub runner_failure_rules: Vec<RunnerFailureRule>,
}

#[cfg(test)]
mod tests {
    use super::{SandboxExecutionPolicy, SandboxMode, SandboxPolicy};

    #[test]
    fn as_str_and_parse_are_kebab_case() {
        assert_eq!(SandboxMode::ReadOnly.as_str(), "read-only");
        assert_eq!(SandboxMode::WorkspaceWrite.as_str(), "workspace-write");
        assert_eq!(SandboxMode::DangerFullAccess.as_str(), "danger-full-access");
        assert_eq!(SandboxMode::parse("read-only"), Some(SandboxMode::ReadOnly));
        assert_eq!(
            SandboxMode::parse("workspace-write"),
            Some(SandboxMode::WorkspaceWrite)
        );
        assert_eq!(
            SandboxMode::parse("danger-full-access"),
            Some(SandboxMode::DangerFullAccess)
        );
        assert_eq!(SandboxMode::parse("ReadOnly"), None);
    }

    #[test]
    fn confined_rejects_danger_full_access() {
        let ok = SandboxPolicy::confined(SandboxExecutionPolicy {
            mode: SandboxMode::ReadOnly,
            workspace_root: "/ws".into(),
            session_id: None,
        })
        .unwrap();
        assert_eq!(ok.mode, SandboxMode::ReadOnly);
        assert_eq!(ok.workspace_root, "/ws");
        assert!(ok.session_id.is_none());

        let write = SandboxPolicy::confined(SandboxExecutionPolicy {
            mode: SandboxMode::WorkspaceWrite,
            workspace_root: "/ws".into(),
            session_id: None,
        })
        .unwrap();
        assert_eq!(write.mode, SandboxMode::WorkspaceWrite);

        let err = SandboxPolicy::confined(SandboxExecutionPolicy {
            mode: SandboxMode::DangerFullAccess,
            workspace_root: "/ws".into(),
            session_id: None,
        })
        .unwrap_err();
        assert!(err.to_string().contains("danger-full-access"));
    }
}
