//! Optional model-free tool-result pruner.

use std::sync::Arc;

use dsh_boot::{PLUGIN_TOOL_RESULT_PRUNER, PluginRegistry, PluginSetup};
use dsh_kernel::KernelError;
use dsh_session::{
    ContentBlock, LogEvent, Session, SessionEvent, SurfaceOp, ToolResultData, derive_event_message,
};
use dsh_token_meter::TokenMeter;
use serde_json::{Value, json};

/// Marker substituted for every removed middle span.
pub const PRUNE_MARKER: &str = "\n\n[... tool result middle pruned ...]\n\n";

/// Resolved character budgets for tool-result pruning.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolResultPruneConfig {
    /// Unicode-scalar threshold that qualifies a result for pruning.
    pub threshold_chars: u64,
    /// Leading scalars kept verbatim.
    pub head_chars: u64,
    /// Trailing scalars kept verbatim.
    pub tail_chars: u64,
}

impl Default for ToolResultPruneConfig {
    fn default() -> Self {
        Self {
            threshold_chars: 8192,
            head_chars: 4096,
            tail_chars: 1024,
        }
    }
}

/// Replay-safe tool-result pruner provided as `toolResultPruner`.
pub struct ToolResultPruner {
    config: ToolResultPruneConfig,
    meter: Arc<TokenMeter>,
}

impl ToolResultPruner {
    /// Bind budgets and the conversation token meter.
    #[must_use]
    pub fn new(config: ToolResultPruneConfig, meter: Arc<TokenMeter>) -> Self {
        Self { config, meter }
    }

    /// Unicode scalar count of text blocks; non-text blocks cost zero.
    #[must_use]
    pub fn measure_content(&self, blocks: &[ContentBlock]) -> u64 {
        let mut chars = 0_u64;
        for block in blocks {
            if let ContentBlock::Text { text } = block {
                chars += text.chars().count() as u64;
            }
        }
        chars
    }

    /// Replace an over-budget text middle while retaining rich-block order.
    #[must_use]
    pub fn prune_content(&self, blocks: &[ContentBlock]) -> Option<Vec<ContentBlock>> {
        let total_chars = self.measure_content(blocks);
        if total_chars <= self.config.threshold_chars {
            return None;
        }
        let removed_start = self.config.head_chars;
        let removed_end = total_chars.saturating_sub(self.config.tail_chars);
        let mut pruned = Vec::new();
        let mut consumed = 0_u64;
        let mut marker_inserted = false;
        for block in blocks {
            let ContentBlock::Text { text } = block else {
                pruned.push(block.clone());
                continue;
            };
            let points: Vec<char> = text.chars().collect();
            let block_start = consumed;
            let block_end = block_start + points.len() as u64;
            let head_end =
                (points.len() as u64).min(removed_start.saturating_sub(block_start)) as usize;
            let tail_start =
                (points.len() as u64).min(removed_end.saturating_sub(block_start)) as usize;
            let intersects_removed = block_start < removed_end && block_end > removed_start;
            let marker = if intersects_removed && !marker_inserted {
                marker_inserted = true;
                PRUNE_MARKER
            } else {
                ""
            };
            let text = points[..head_end].iter().collect::<String>()
                + marker
                + &points[tail_start.min(points.len())..]
                    .iter()
                    .collect::<String>();
            if !text.is_empty() {
                pruned.push(ContentBlock::Text { text });
            }
            consumed = block_end;
        }
        Some(pruned)
    }

    /// Prune every over-budget tool result on the current surface.
    ///
    /// # Panics
    ///
    /// Panics when a replacement fails session validation; earlier replacements remain.
    pub fn prune_session(&self, session: &mut Session) {
        let candidates: Vec<u64> = session
            .surface_nodes()
            .iter()
            .copied()
            .filter(|&seq| {
                matches!(
                    session.events().get(seq as usize),
                    Some(LogEvent::Known(SessionEvent::ToolResult { .. }))
                )
            })
            .collect();
        for seq in candidates {
            let event = match session.events().get(seq as usize) {
                Some(LogEvent::Known(event)) => event.clone(),
                Some(LogEvent::Leftover(_)) | None => continue,
            };
            let SessionEvent::ToolResult { data, .. } = event else {
                continue;
            };
            let Some(ContentBlock::ToolResult {
                tool_call_id,
                content,
                is_error,
            }) = data.message.content.first().cloned()
            else {
                continue;
            };
            let Some(pruned_content) = self.prune_content(&content) else {
                continue;
            };
            let shadowed_tokens = derive_event_message(&SessionEvent::ToolResult {
                seq,
                time: seq as i64,
                data: data.clone(),
                surface_op: Some(SurfaceOp::Append),
                source_event_seqs: None,
                ignorable: None,
            })
            .map(|message| self.meter.estimate_message(&message))
            .unwrap_or(0);
            let prune_seq = session.events().len() as u64;
            session
                .append(SessionEvent::CompactionPrune {
                    seq: prune_seq,
                    time: prune_seq as i64,
                    data: json!({
                        "shadowedRange": { "start": seq, "end": seq },
                        "shadowedSeqs": [seq],
                        "shadowedTokenCount": shadowed_tokens,
                    }),
                    ignorable: None,
                })
                .expect("compaction/prune");
            let mut message = data.message.clone();
            message.content = vec![ContentBlock::ToolResult {
                tool_call_id,
                content: pruned_content,
                is_error,
            }];
            let replace_seq = session.events().len() as u64;
            session
                .append(SessionEvent::ToolResult {
                    seq: replace_seq,
                    time: replace_seq as i64,
                    data: ToolResultData {
                        turn: data.turn,
                        step: data.step,
                        message,
                        error: data.error.clone(),
                        meta: data.meta.clone(),
                    },
                    surface_op: Some(SurfaceOp::Replace {
                        start: seq,
                        end: seq,
                    }),
                    source_event_seqs: Some(vec![seq]),
                    ignorable: None,
                })
                .expect("tool/result replace");
        }
    }
}

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

fn resolve_pruner_config(value: &Value) -> Result<ToolResultPruneConfig, KernelError> {
    match value {
        Value::Null => Ok(ToolResultPruneConfig::default()),
        Value::Object(map) if map.is_empty() => Ok(ToolResultPruneConfig::default()),
        Value::Object(map) => {
            for key in map.keys() {
                if !matches!(key.as_str(), "thresholdChars" | "headChars" | "tailChars") {
                    return Err(setup_err(format!(
                        "ToolResultPruneConfig: unknown key \"{key}\" \
                         (allowed: thresholdChars, headChars, tailChars)"
                    )));
                }
            }
            let config = ToolResultPruneConfig {
                threshold_chars: value
                    .get("thresholdChars")
                    .and_then(Value::as_u64)
                    .unwrap_or(8192),
                head_chars: value
                    .get("headChars")
                    .and_then(Value::as_u64)
                    .unwrap_or(4096),
                tail_chars: value
                    .get("tailChars")
                    .and_then(Value::as_u64)
                    .unwrap_or(1024),
            };
            if config.threshold_chars == 0 {
                return Err(setup_err(
                    "ToolResultPruneConfig: thresholdChars must be a positive integer",
                ));
            }
            let emitted =
                config.head_chars + PRUNE_MARKER.chars().count() as u64 + config.tail_chars;
            if emitted > config.threshold_chars {
                return Err(setup_err(format!(
                    "ToolResultPruneConfig: headChars + marker + tailChars ({emitted}) \
                     must be at most thresholdChars ({})",
                    config.threshold_chars
                )));
            }
            Ok(config)
        }
        _ => Err(setup_err("ToolResultPruneConfig: config must be an object")),
    }
}

/// Register YAML `@deepseek-ai/dsh-compaction-tool-result-pruner`.
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            let meter = ctx.inject::<TokenMeter>("tokenMeter").await?;
            let resolved = resolve_pruner_config(&config)?;
            ctx.provide("toolResultPruner", ToolResultPruner::new(resolved, meter))
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_TOOL_RESULT_PRUNER, setup);
}
