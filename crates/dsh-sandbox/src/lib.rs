//! Fail-closed sandbox modes, writable roots, and escalation for the Rust host.

mod error;
mod escalation;
mod policy;
mod roots;
mod types;

pub use error::{SANDBOX_UNAVAILABLE, SandboxError};
pub use escalation::{
    ESCALATION_TARGETS, EscalationApprover, EscalationOutcome, EscalationRequest,
    approve_escalation, escalation_hint_marker, sandbox_denial_marker, validate_escalation_args,
    wider_modes,
};
pub use policy::SandboxPolicyResolver;
pub use roots::{canonical_path, writable_roots};
pub use types::{
    ConfinedArgv, ConfinedSandboxMode, RunnerFailureRule, SandboxEnforcement,
    SandboxExecutionPolicy, SandboxMode, SandboxPolicy,
};
