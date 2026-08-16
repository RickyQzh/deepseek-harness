//! Subagent Service Definition: named-provider registry, one-shot start, and continuable settlement.
//!
//! This crate is not a spawn/fork implementation. Load
//! `dsh-subagent-in-process` to register the in-process providers on `subagents`.

mod child;
mod continuation;
mod descriptor;
pub mod plugin;
mod runtime;
mod types;

pub use child::{DEFAULT_SUBAGENT_MAX_DEPTH, assert_subagent_max_depth, delegation_depth_of};
pub use continuation::{ContinuableChildInfo, ContinuableSetup, ContinuableStartSpec};
pub use descriptor::{
    SUBAGENT_DESCRIPTOR_VERSION, fold_subagent_descriptor, snapshot_continuable_descriptor,
    snapshot_one_shot_descriptor,
};
pub use plugin::register;
pub use runtime::SubagentRuntime;
pub use types::{
    ContinuableCreateSpec, EVENT_SUBAGENT_END, EVENT_SUBAGENT_START, SubagentCapabilities,
    SubagentError, SubagentProvider, SubagentResult, SubagentRunEndInfo, SubagentRunId,
    SubagentRunIdTag, SubagentRunInfo, SubagentStartRequest, SubagentStopReason,
};
