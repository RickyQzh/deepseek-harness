//! Log-only compaction lock, surface replace, and overflow retry.

mod config;
mod engine;
mod plugin;
mod pruner;
mod summarize;

pub use config::{
    BasicCompactionConfig, ConfigError, ModelCompactPolicy, ResolvedCompactSpec,
    resolve_compact_spec, resolve_config, resolve_target_policy,
};
pub use engine::{BasicCompactionEngine, install_compaction_auto};
pub use plugin::register;
pub use pruner::{
    PRUNE_MARKER, ToolResultPruneConfig, ToolResultPruner, register as register_pruner,
};
pub use summarize::{
    CHECKPOINT_PREAMBLE, SUMMARY_CLOSE_TAG, SUMMARY_OPEN_TAG, SummarizationInput, SummaryResult,
    frame_summary,
};
