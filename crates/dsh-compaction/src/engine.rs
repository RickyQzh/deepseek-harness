//! Compaction result types and the abstract engine.

use std::future::Future;
use std::pin::Pin;

use dsh_agent_loop::LoopOptions;
use dsh_brand::Branded;
use dsh_session::{ContentBlock, Session};
use dsh_tools::AbortFlag;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::CompactionError;

/// Tag for [`CompactionId`].
pub struct CompactionIdTag;

/// Stable identity shared by one compact start/summary/checkpoint/end transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompactionId(Branded<CompactionIdTag>);

impl CompactionId {
    /// Brand `value` as a compaction identity. No validation is performed.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(Branded::new(value))
    }

    /// Borrow the inner string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl Serialize for CompactionId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CompactionId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::new)
    }
}

/// Why automatic policy is asking a backend to consider compaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompactionTrigger {
    /// After a successful step, before the next request.
    Pressure,
    /// Provider-confirmed context overflow recovery.
    ContextOverflow,
}

/// Inclusive first and last surface-node seqs of a shadowed range.
///
/// This is a surface-position span, not a numeric seq interval: after a prior
/// replace, `start` can be greater than `end`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShadowedRange {
    /// Seq of the first shadowed surface node.
    pub start: u64,
    /// Seq of the last shadowed surface node.
    pub end: u64,
}

/// Result of a successful compaction operation.
#[derive(Clone, Debug, PartialEq)]
pub struct CompactionResult {
    compaction_id: CompactionId,
    start_seq: u64,
    summary_seq: u64,
    end_seq: u64,
    summary: Vec<ContentBlock>,
    shadowed_range: ShadowedRange,
    shadowed_seqs: Vec<u64>,
    shadowed_token_count: u64,
}

impl CompactionResult {
    /// Assemble one successful compaction result.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        compaction_id: CompactionId,
        start_seq: u64,
        summary_seq: u64,
        end_seq: u64,
        summary: Vec<ContentBlock>,
        shadowed_range: ShadowedRange,
        shadowed_seqs: Vec<u64>,
        shadowed_token_count: u64,
    ) -> Self {
        Self {
            compaction_id,
            start_seq,
            summary_seq,
            end_seq,
            summary,
            shadowed_range,
            shadowed_seqs,
            shadowed_token_count,
        }
    }

    /// Stable identity shared by this compaction's complete durable lifecycle.
    #[must_use]
    pub fn compaction_id(&self) -> &CompactionId {
        &self.compaction_id
    }

    /// Seq of the appended `compaction/start` event.
    #[must_use]
    pub fn start_seq(&self) -> u64 {
        self.start_seq
    }

    /// Seq of the appended `compaction/summary` event.
    #[must_use]
    pub fn summary_seq(&self) -> u64 {
        self.summary_seq
    }

    /// Seq of the appended `compaction/end` event.
    #[must_use]
    pub fn end_seq(&self) -> u64 {
        self.end_seq
    }

    /// Summary content blocks produced by the backend.
    #[must_use]
    pub fn summary(&self) -> &[ContentBlock] {
        &self.summary
    }

    /// Surface-boundary pair that was shadowed.
    #[must_use]
    pub fn shadowed_range(&self) -> ShadowedRange {
        self.shadowed_range
    }

    /// Seq of every shadowed surface node, in surface order.
    #[must_use]
    pub fn shadowed_seqs(&self) -> &[u64] {
        &self.shadowed_seqs
    }

    /// Estimated token count of the shadowed content.
    #[must_use]
    pub fn shadowed_token_count(&self) -> u64 {
        self.shadowed_token_count
    }
}

/// Abstract compaction backend. Implementations own trigger policy, retention, and summarization.
pub trait CompactionEngine: Send + Sync {
    /// Consider automatic compaction for one explicit trigger.
    ///
    /// Pressure policy uses the latest durable routed request. Context-overflow
    /// policy may force a useful balanced reduction even below the normal
    /// threshold. Return `Ok(None)` when no safe range can be compacted.
    ///
    /// # Parameters
    ///
    /// * `session` - session whose surface may be replaced.
    /// * `options` - loop route and scheduler defaults guiding summarization.
    /// * `trigger` - normal pressure or provider-confirmed context overflow.
    /// * `signal` - cancellation flag; model-backed implementations must observe it.
    ///
    /// # Returns
    ///
    /// The compaction result, or `None` if no compaction was needed.
    /// The returned future borrows `self`, `session`, `options`, and `signal`.
    fn compact_if_needed<'a>(
        &'a self,
        session: &'a mut Session,
        options: &'a LoopOptions,
        trigger: CompactionTrigger,
        signal: &'a AbortFlag,
    ) -> Pin<Box<dyn Future<Output = Result<Option<CompactionResult>, CompactionError>> + Send + 'a>>;

    /// Forcibly compact an inclusive surface-position span into one summary node.
    ///
    /// `start` and `end` name surface positions, not numeric seq order. Both
    /// edges must be tool-pairing balanced.
    ///
    /// # Parameters
    ///
    /// * `start` - first surface seq, inclusive.
    /// * `end` - last surface seq, inclusive.
    /// * `session` - session mutated by the replacement.
    /// * `options` - loop route and scheduler defaults guiding summarization.
    /// * `signal` - cancellation flag; model-backed implementations must observe it.
    ///
    /// # Returns
    ///
    /// The appended event seqs, summary, replaced range, and token accounting.
    /// The returned future borrows `self`, `session`, `options`, and `signal`.
    fn compact_region<'a>(
        &'a self,
        start: u64,
        end: u64,
        session: &'a mut Session,
        options: &'a LoopOptions,
        signal: &'a AbortFlag,
    ) -> Pin<Box<dyn Future<Output = Result<CompactionResult, CompactionError>> + Send + 'a>>;
}
