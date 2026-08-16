//! Compaction engine types, checkpoint source, and tool-pairing helpers.

mod checkpoint;
mod engine;
mod error;
mod pairing;

#[cfg(test)]
mod phase6_exit;

pub use checkpoint::{compact_checkpoint_source, is_compact_checkpoint_source};
pub use engine::{
    CompactionEngine, CompactionId, CompactionIdTag, CompactionResult, CompactionTrigger,
    ShadowedRange,
};
pub use error::{CompactionError, ManualCompactionErrorCode};
pub use pairing::{tool_pairing_balanced_after, tool_pairing_balanced_before};
