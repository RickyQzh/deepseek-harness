//! Fail-closed approval waterfall, per-session policy, and tools `Approver`.

mod auto_approve;
pub mod plugin;
mod service;

#[cfg(test)]
mod phase6_exit;

#[cfg(test)]
mod phase8_exit;

pub use dsh_tools::ApprovalOutcome;
pub use service::{
    ApprovalError, ApprovalPolicy, ApprovalQuestion, ApprovalRequest, ApprovalService,
    EVENT_APPROVAL_REQUEST, effective_approval_policy, has_open_turn, set_approval_policy,
};
