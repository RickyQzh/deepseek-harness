//! Strictly-wider ladder, argument pairing, markers, and fail-closed approval.

use std::future::Future;
use std::pin::Pin;

use crate::error::SandboxError;
use crate::types::SandboxMode;

/// Closed escalation-target modes. Nothing escalates to `read-only`.
pub const ESCALATION_TARGETS: &[SandboxMode] =
    &[SandboxMode::WorkspaceWrite, SandboxMode::DangerFullAccess];

/// Modes strictly wider than `mode`. Empty for [`SandboxMode::DangerFullAccess`].
#[must_use]
pub fn wider_modes(mode: SandboxMode) -> &'static [SandboxMode] {
    match mode {
        SandboxMode::ReadOnly => ESCALATION_TARGETS,
        SandboxMode::WorkspaceWrite => &[SandboxMode::DangerFullAccess],
        SandboxMode::DangerFullAccess => &[],
    }
}

/// Model-facing denial marker for a file effect refused under `mode`.
#[must_use]
pub fn sandbox_denial_marker(mode: SandboxMode) -> String {
    format!("[sandbox: file access denied under {} mode]", mode.as_str())
}

/// Same-turn escalation hint that rides a denial when escalation fields are advertised.
#[must_use]
pub fn escalation_hint_marker(subject: &str) -> String {
    format!(
        "[sandbox: escalation available — retry this exact {subject} once with sandbox_permissions (the narrowest wider mode that suffices) + justification; the approval prompt asks the user]"
    )
}

/// Validate that `sandbox_permissions` and `justification` travel together as a non-empty sentence.
///
/// # Errors
///
/// Returns [`SandboxError::Invalid`] when only one argument is present, or when `justification`
/// is present but empty after trim.
pub fn validate_escalation_args(
    sandbox_permissions: Option<&str>,
    justification: Option<&str>,
) -> Result<(), SandboxError> {
    if sandbox_permissions.is_some() && justification.is_none() {
        return Err(SandboxError::Invalid(
            "invalid escalation: sandbox_permissions requires a justification".into(),
        ));
    }
    if justification.is_some() && sandbox_permissions.is_none() {
        return Err(SandboxError::Invalid(
            "invalid escalation: justification is only valid together with sandbox_permissions"
                .into(),
        ));
    }
    if justification.is_some_and(|text| text.trim().is_empty()) {
        return Err(SandboxError::Invalid(
            "invalid justification: expected a non-empty sentence".into(),
        ));
    }
    Ok(())
}

/// Closed outcome of one escalation ask.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EscalationOutcome {
    /// The human allowed this one call.
    AllowedOnce,
    /// The human refused the escalation.
    Rejected,
    /// The prompt was dismissed without a decision.
    Cancelled,
    /// No approval channel could answer.
    Unavailable,
}

/// One escalation request as [`approve_escalation`] judges it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EscalationRequest {
    /// Requested target mode (kebab-case). Used verbatim in error text.
    pub requested_mode: String,
    /// Model's one-sentence reason, shown inside the audit reason.
    pub justification: String,
    /// Call's effective mode; the request must strictly widen it.
    pub effective_mode: SandboxMode,
    /// Family noun for user-facing texts (`command` for bash, `operation` for fs).
    pub subject: String,
}

/// Minimal approval requester [`approve_escalation`] needs. The tool layer holds agent identity.
pub trait EscalationApprover: Send + Sync {
    /// Ask the human to approve one action.
    ///
    /// `reason` is the audit string `escalate sandbox to {mode}: {justification}`.
    fn request(&self, reason: String) -> Pin<Box<dyn Future<Output = EscalationOutcome> + Send>>;
}

/// Resolve a sandbox-escalation request before anything executes.
///
/// Order: not strictly wider, missing approver, `has_agent == false`, then `approver.request`.
/// A granted mode applies to this call only.
///
/// # Errors
///
/// Returns [`SandboxError::Invalid`] for a non-widening request, a missing approval service,
/// an agent-less call, rejection, cancellation, or an unanswerable ask.
pub async fn approve_escalation(
    request: EscalationRequest,
    approver: Option<&dyn EscalationApprover>,
    has_agent: bool,
) -> Result<SandboxMode, SandboxError> {
    let EscalationRequest {
        requested_mode,
        justification,
        effective_mode,
        subject,
    } = request;
    let Some(granted) = wider_modes(effective_mode)
        .iter()
        .copied()
        .find(|mode| mode.as_str() == requested_mode)
    else {
        return Err(SandboxError::Invalid(format!(
            "sandbox escalation to \"{requested_mode}\" is not strictly wider than this call's current \"{}\" mode",
            effective_mode.as_str()
        )));
    };
    let Some(approver) = approver else {
        return Err(SandboxError::Invalid(format!(
            "sandbox escalation to \"{requested_mode}\" requires approval, but no approval service is composed"
        )));
    };
    if !has_agent {
        return Err(SandboxError::Invalid(format!(
            "sandbox escalation to \"{requested_mode}\" requires approval, but the call has no agent to route it through"
        )));
    }
    let reason = format!("escalate sandbox to {requested_mode}: {justification}");
    match approver.request(reason).await {
        EscalationOutcome::AllowedOnce => Ok(granted),
        EscalationOutcome::Rejected => Err(SandboxError::Invalid(format!(
            "the user rejected escalating this {subject} to \"{requested_mode}\""
        ))),
        EscalationOutcome::Cancelled => Err(SandboxError::Invalid(format!(
            "approval for escalating to \"{requested_mode}\" was cancelled"
        ))),
        EscalationOutcome::Unavailable => Err(SandboxError::Invalid(format!(
            "sandbox escalation to \"{requested_mode}\" requires approval, but no approval channel is available"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ESCALATION_TARGETS, EscalationApprover, EscalationOutcome, EscalationRequest,
        approve_escalation, escalation_hint_marker, sandbox_denial_marker,
        validate_escalation_args, wider_modes,
    };
    use crate::SandboxMode;
    use std::future::Future;
    use std::pin::Pin;

    struct AllowOnce;
    impl EscalationApprover for AllowOnce {
        fn request(
            &self,
            reason: String,
        ) -> Pin<Box<dyn Future<Output = EscalationOutcome> + Send>> {
            assert!(reason.contains("escalate sandbox to workspace-write:"));
            Box::pin(async { EscalationOutcome::AllowedOnce })
        }
    }
    struct Reject;
    impl EscalationApprover for Reject {
        fn request(&self, _: String) -> Pin<Box<dyn Future<Output = EscalationOutcome> + Send>> {
            Box::pin(async { EscalationOutcome::Rejected })
        }
    }

    struct Cancel;
    impl EscalationApprover for Cancel {
        fn request(&self, _: String) -> Pin<Box<dyn Future<Output = EscalationOutcome> + Send>> {
            Box::pin(async { EscalationOutcome::Cancelled })
        }
    }

    struct ChannelUnavailable;
    impl EscalationApprover for ChannelUnavailable {
        fn request(&self, _: String) -> Pin<Box<dyn Future<Output = EscalationOutcome> + Send>> {
            Box::pin(async { EscalationOutcome::Unavailable })
        }
    }

    #[test]
    fn wider_ladder_and_targets() {
        assert_eq!(
            wider_modes(SandboxMode::ReadOnly),
            &[SandboxMode::WorkspaceWrite, SandboxMode::DangerFullAccess]
        );
        assert_eq!(
            wider_modes(SandboxMode::WorkspaceWrite),
            &[SandboxMode::DangerFullAccess]
        );
        assert!(wider_modes(SandboxMode::DangerFullAccess).is_empty());
        assert_eq!(
            ESCALATION_TARGETS,
            &[SandboxMode::WorkspaceWrite, SandboxMode::DangerFullAccess]
        );
    }

    #[test]
    fn pairing_validation() {
        validate_escalation_args(None, None).unwrap();
        validate_escalation_args(
            Some("workspace-write"),
            Some("because the workspace needs it"),
        )
        .unwrap();
        assert!(
            validate_escalation_args(Some("workspace-write"), None)
                .unwrap_err()
                .to_string()
                .contains("requires a justification")
        );
        assert!(
            validate_escalation_args(None, Some("orphan reason"))
                .unwrap_err()
                .to_string()
                .contains("only valid together with sandbox_permissions")
        );
        assert!(
            validate_escalation_args(Some("workspace-write"), Some("   "))
                .unwrap_err()
                .to_string()
                .contains("non-empty sentence")
        );
    }

    #[test]
    fn markers_are_verbatim() {
        assert_eq!(
            sandbox_denial_marker(SandboxMode::ReadOnly),
            "[sandbox: file access denied under read-only mode]"
        );
        assert_eq!(
            escalation_hint_marker("command"),
            "[sandbox: escalation available — retry this exact command once with sandbox_permissions (the narrowest wider mode that suffices) + justification; the approval prompt asks the user]"
        );
    }

    #[tokio::test]
    async fn approve_escalation_fail_closed_paths() {
        let req = EscalationRequest {
            requested_mode: "workspace-write".into(),
            justification: "need writes".into(),
            effective_mode: SandboxMode::ReadOnly,
            subject: "command".into(),
        };
        let granted = approve_escalation(req.clone(), Some(&AllowOnce), true)
            .await
            .unwrap();
        assert_eq!(granted, SandboxMode::WorkspaceWrite);
        let err = approve_escalation(req.clone(), None, true)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no approval service is composed"));
        let err = approve_escalation(req.clone(), Some(&AllowOnce), false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no agent to route it through"));
        let err = approve_escalation(req.clone(), Some(&Reject), true)
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("the user rejected escalating this command")
        );
        let not_wider = EscalationRequest {
            requested_mode: "read-only".into(),
            justification: "nope".into(),
            effective_mode: SandboxMode::WorkspaceWrite,
            subject: "command".into(),
        };
        let err = approve_escalation(not_wider, Some(&AllowOnce), true)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("is not strictly wider"));
    }

    #[tokio::test]
    async fn approve_escalation_maps_cancelled_and_unavailable() {
        let req = EscalationRequest {
            requested_mode: "workspace-write".into(),
            justification: "need writes".into(),
            effective_mode: SandboxMode::ReadOnly,
            subject: "command".into(),
        };
        let err = approve_escalation(req.clone(), Some(&Cancel), true)
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("approval for escalating to \"workspace-write\" was cancelled")
        );
        let err = approve_escalation(req, Some(&ChannelUnavailable), true)
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("requires approval, but no approval channel is available")
        );
    }
}
