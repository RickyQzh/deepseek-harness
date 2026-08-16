//! Fail-closed sandbox modes, writable roots, escalation, and local confine wrapping for the Rust host.

mod error;
mod escalation;
mod landlock;
mod policy;
mod profiles;
mod provider;
mod roots;
mod types;

pub use error::{SANDBOX_UNAVAILABLE, SandboxError};
pub use escalation::{
    ESCALATION_TARGETS, EscalationApprover, EscalationOutcome, EscalationRequest,
    approve_escalation, escalation_hint_marker, sandbox_denial_marker, validate_escalation_args,
    wider_modes,
};
pub use landlock::{
    LAUNCHER_BIN, LAUNCHER_FAILURE_EXIT, LandlockEnforcement, grant_args, launcher_path, probe,
};
pub use policy::SandboxPolicyResolver;
pub use profiles::{bwrap_profile_args, landlock_profile_args, seatbelt_profile_args};
pub use provider::{LocalSandboxConfig, LocalSandboxProvider, RunnerKind, SandboxInternals};
pub use roots::{canonical_path, writable_roots};
pub use types::{
    ConfinedArgv, ConfinedSandboxMode, RunnerFailureRule, SandboxEnforcement,
    SandboxExecutionPolicy, SandboxMode, SandboxPolicy,
};
