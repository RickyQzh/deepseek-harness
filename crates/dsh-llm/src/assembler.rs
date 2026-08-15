//! Incremental chunk-to-message fold used by the agent loop.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::atomic::{AtomicU64, Ordering};

use dsh_session::{
    CallId, ContentBlock, FinishReason, Message, MessageId, MessageRole, MessageSource,
    StreamChunk, TokenUsage,
};
use serde_json::Value;

struct PartialBlock {
    block_type: String,
    text: String,
    tool_call_id: Option<CallId>,
    tool_call_name: Option<String>,
    tool_call_arguments: String,
    block: Option<ContentBlock>,
}

impl PartialBlock {
    fn open(block_type: String) -> Self {
        Self {
            block_type,
            text: String::new(),
            tool_call_id: None,
            tool_call_name: None,
            tool_call_arguments: String::new(),
            block: None,
        }
    }
}

/// Incremental assembler from raw [`StreamChunk`]s into content blocks and a message.
pub struct BlockAssembler {
    partials: HashMap<u32, PartialBlock>,
    order: Vec<u32>,
    usage: Option<TokenUsage>,
    finish: Option<FinishReason>,
    replay_state: Option<Value>,
}

impl Default for BlockAssembler {
    fn default() -> Self {
        Self::new()
    }
}

impl BlockAssembler {
    /// Empty assembler with no blocks, usage, or finish.
    #[must_use]
    pub fn new() -> Self {
        Self {
            partials: HashMap::new(),
            order: Vec::new(),
            usage: None,
            finish: None,
            replay_state: None,
        }
    }

    /// Feed one chunk into the assembly state, in stream order.
    pub fn push(&mut self, chunk: StreamChunk) {
        match chunk {
            StreamChunk::BlockStart { index, block_type } => {
                if let Entry::Vacant(entry) = self.partials.entry(index) {
                    self.order.push(index);
                    entry.insert(PartialBlock::open(block_type));
                }
            }
            StreamChunk::TextDelta { index, text } => {
                let partial = self.ensure(index, "text");
                if partial.block.is_some() {
                    return;
                }
                partial.text.push_str(&text);
            }
            StreamChunk::ReasoningDelta { index, text } => {
                let partial = self.ensure(index, "reasoning");
                if partial.block.is_some() {
                    return;
                }
                partial.text.push_str(&text);
            }
            StreamChunk::ToolCallDelta {
                index,
                id,
                name,
                arguments_delta,
            } => {
                let partial = self.ensure(index, "tool-call");
                if partial.block.is_some() {
                    return;
                }
                partial.tool_call_id = Some(id);
                if let Some(name) = name {
                    partial.tool_call_name = Some(name);
                }
                partial.tool_call_arguments.push_str(&arguments_delta);
            }
            StreamChunk::BlockEnd { index, block } => {
                let block_type = content_block_type_name(&block);
                let partial = self.ensure(index, block_type);
                if partial.block.is_some() {
                    return;
                }
                partial.block = Some(block);
            }
            StreamChunk::Usage { usage } => {
                self.usage = Some(usage);
            }
            StreamChunk::Finish {
                reason,
                replay_state,
            } => {
                self.finish = Some(reason);
                self.replay_state = replay_state;
            }
        }
    }

    fn ensure(&mut self, index: u32, block_type: &str) -> &mut PartialBlock {
        match self.partials.entry(index) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                self.order.push(index);
                entry.insert(PartialBlock::open(block_type.to_string()))
            }
        }
    }

    fn assemble(&self, partial: &PartialBlock, index: u32) -> ContentBlock {
        if let Some(block) = &partial.block {
            return block.clone();
        }
        match partial.block_type.as_str() {
            "text" => ContentBlock::Text {
                text: partial.text.clone(),
            },
            "reasoning" => ContentBlock::Reasoning {
                text: partial.text.clone(),
            },
            "tool-call" => ContentBlock::ToolCall {
                id: partial
                    .tool_call_id
                    .clone()
                    .unwrap_or_else(|| CallId::new(format!("call-{index}"))),
                name: partial.tool_call_name.clone().unwrap_or_default(),
                arguments: partial.tool_call_arguments.clone(),
            },
            other => panic!("cannot assemble incomplete block of type \"{other}\""),
        }
    }

    fn must_get(&self, index: u32) -> &PartialBlock {
        self.partials.get(&index).unwrap_or_else(|| {
            panic!("BlockAssembler invariant violated: no partial for index {index}")
        })
    }

    /// Assemble all blocks seen so far, in stream order.
    ///
    /// Max-token truncation drops tool-call blocks that cannot be executed safely.
    /// An open block assembles from its accumulated deltas.
    #[must_use]
    pub fn blocks(&self) -> Vec<ContentBlock> {
        let blocks: Vec<ContentBlock> = self
            .order
            .iter()
            .map(|index| self.assemble(self.must_get(*index), *index))
            .collect();
        if matches!(self.finish(), FinishReason::MaxTokens) {
            blocks
                .into_iter()
                .filter(|block| !matches!(block, ContentBlock::ToolCall { .. }))
                .collect()
        } else {
            blocks
        }
    }

    /// Usage from the `usage` chunk; `None` until one arrives.
    #[must_use]
    pub fn usage(&self) -> Option<&TokenUsage> {
        self.usage.as_ref()
    }

    /// Finish reason from the `finish` chunk; [`FinishReason::Stop`] when none arrived.
    #[must_use]
    pub fn finish(&self) -> FinishReason {
        self.finish.clone().unwrap_or(FinishReason::Stop)
    }

    /// Adapter-private replay state from the terminal finish chunk, if any.
    #[must_use]
    pub fn replay_state(&self) -> Option<&Value> {
        self.replay_state.as_ref()
    }

    /// Assembled assistant-role message over [`Self::blocks`].
    #[must_use]
    pub fn message(&self, source: MessageSource) -> Message {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Message {
            id: MessageId::new(format!("msg-{}", NEXT.fetch_add(1, Ordering::Relaxed))),
            role: MessageRole::Assistant,
            content: self.blocks(),
            source,
        }
    }
}

fn content_block_type_name(block: &ContentBlock) -> &'static str {
    match block {
        ContentBlock::Text { .. } => "text",
        ContentBlock::Reasoning { .. } => "reasoning",
        ContentBlock::Image { .. } => "image",
        ContentBlock::ToolCall { .. } => "tool-call",
        ContentBlock::ToolResult { .. } => "tool-result",
    }
}

#[cfg(test)]
mod tests {
    use super::BlockAssembler;
    use dsh_session::{CallId, ContentBlock, FinishReason, StreamChunk, TokenUsage};

    #[test]
    fn max_tokens_drops_tool_call_blocks() {
        let mut assembler = BlockAssembler::new();
        assembler.push(StreamChunk::BlockStart {
            index: 0,
            block_type: "text".into(),
        });
        assembler.push(StreamChunk::TextDelta {
            index: 0,
            text: "safe".into(),
        });
        assembler.push(StreamChunk::BlockEnd {
            index: 0,
            block: ContentBlock::Text {
                text: "safe".into(),
            },
        });
        assembler.push(StreamChunk::BlockStart {
            index: 1,
            block_type: "tool-call".into(),
        });
        assembler.push(StreamChunk::ToolCallDelta {
            index: 1,
            id: CallId::new("c1"),
            name: Some("echo".into()),
            arguments_delta: "{\"x\":1}".into(),
        });
        assembler.push(StreamChunk::Usage {
            usage: TokenUsage {
                input_tokens: 1,
                output_tokens: 2,
                cache_read_tokens: None,
                cache_write_tokens: None,
                reasoning_tokens: None,
            },
        });
        assembler.push(StreamChunk::Finish {
            reason: FinishReason::MaxTokens,
            replay_state: None,
        });
        let blocks = assembler.blocks();
        assert_eq!(
            blocks,
            vec![ContentBlock::Text {
                text: "safe".into()
            }]
        );
        assert!(matches!(assembler.finish(), FinishReason::MaxTokens));
    }

    #[test]
    fn closed_block_ignores_straggler_delta() {
        let mut assembler = BlockAssembler::new();
        assembler.push(StreamChunk::BlockEnd {
            index: 0,
            block: ContentBlock::Text {
                text: "done".into(),
            },
        });
        assembler.push(StreamChunk::TextDelta {
            index: 0,
            text: "nope".into(),
        });
        assert_eq!(
            assembler.blocks(),
            vec![ContentBlock::Text {
                text: "done".into()
            }]
        );
    }
}
