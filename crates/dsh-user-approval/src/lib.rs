//! Fail-closed approval waterfall, per-session policy, and tools `Approver`.

mod auto_approve;
pub mod plugin;
mod service;

pub use dsh_tools::ApprovalOutcome;
pub use service::{
    ApprovalError, ApprovalPolicy, ApprovalRequest, ApprovalService, EVENT_APPROVAL_REQUEST,
    effective_approval_policy, has_open_turn, set_approval_policy,
};
