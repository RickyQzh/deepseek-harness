//! Scripted adapter and chunk helpers for tests and later loop tasks.

use std::pin::Pin;
use std::sync::Mutex;

use dsh_session::{CallId, ContentBlock, FinishReason, StreamChunk, TokenUsage};
use futures::Stream;
use futures::StreamExt;

use crate::LlmAdapter;
use crate::error::{ABORTED_CODE, LlmError};
use crate::types::{GenerateOptions, LlmModelReasoningInfo, LlmResolvedModelInfo};

/// One scripted model-call outcome consumed by [`MockAdapter`].
pub enum MockScript {
    /// Yield these chunks, then end.
    Chunks(Vec<StreamChunk>),
    /// Yield a partial text block, wait until the request is aborted, then fail with [`ABORTED_CODE`].
    Hang,
    /// Fail immediately with this error; [`crate::LlmRuntime`] turns it into a terminal finish.
    Fail(LlmError),
}

/// Scripted [`LlmAdapter`] that records every request it receives.
pub struct MockAdapter {
    /// Requests observed by [`LlmAdapter::stream`], in call order.
    pub requests: Mutex<Vec<GenerateOptions>>,
    script: Mutex<Vec<MockScript>>,
    reasoning: Option<LlmModelReasoningInfo>,
    default_max_tokens: Option<u64>,
}

impl MockAdapter {
    /// Drive successive `stream` calls from `script`, consuming one entry per call.
    #[must_use]
    pub fn new(script: Vec<MockScript>) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            script: Mutex::new(script),
            reasoning: None,
            default_max_tokens: None,
        }
    }

    /// Attach adapter-owned call defaults used by [`LlmAdapter::resolve_model`].
    #[must_use]
    pub fn with_defaults(
        mut self,
        default_max_tokens: Option<u64>,
        reasoning: Option<LlmModelReasoningInfo>,
    ) -> Self {
        self.default_max_tokens = default_max_tokens;
        self.reasoning = reasoning;
        self
    }
}

impl LlmAdapter for MockAdapter {
    fn resolve_model(&self, provider: &str, model: &str) -> LlmResolvedModelInfo {
        LlmResolvedModelInfo {
            provider: provider.into(),
            id: model.into(),
            name: model.into(),
            context: None,
            default_max_tokens: self.default_max_tokens,
            reasoning: self.reasoning.clone(),
        }
    }

    fn stream(
        &self,
        options: GenerateOptions,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, LlmError>> + Send + '_>> {
        self.requests
            .lock()
            .expect("mock requests mutex")
            .push(options.clone());
        let entry = {
            let mut script = self.script.lock().expect("mock script mutex");
            if script.is_empty() {
                None
            } else {
                Some(script.remove(0))
            }
        };
        match entry {
            None => Box::pin(futures::stream::iter([Err(LlmError::new(
                "MockAdapter: script exhausted",
                "UNKNOWN",
            ))])),
            Some(MockScript::Chunks(chunks)) => {
                Box::pin(futures::stream::iter(chunks.into_iter().map(Ok)))
            }
            Some(MockScript::Fail(error)) => Box::pin(futures::stream::iter([Err(error)])),
            Some(MockScript::Hang) => {
                let signal = options.signal.clone();
                let start = StreamChunk::BlockStart {
                    index: 0,
                    block_type: "text".into(),
                };
                let delta = StreamChunk::TextDelta {
                    index: 0,
                    text: "partial".into(),
                };
                Box::pin(futures::stream::iter([Ok(start), Ok(delta)]).chain(
                    futures::stream::once(async move {
                        signal.cancelled().await;
                        Err(LlmError::new("aborted", ABORTED_CODE))
                    }),
                ))
            }
        }
    }
}

/// Per-character text deltas, then usage and a `stop` finish.
#[must_use]
pub fn text_response(text: &str) -> Vec<StreamChunk> {
    text_chunks(text, FinishReason::Stop)
}

/// Like [`text_response`] but the stream ends with [`FinishReason::MaxTokens`].
#[must_use]
pub fn max_tokens_response(text: &str) -> Vec<StreamChunk> {
    text_chunks(text, FinishReason::MaxTokens)
}

fn text_chunks(text: &str, reason: FinishReason) -> Vec<StreamChunk> {
    let mut chunks = vec![StreamChunk::BlockStart {
        index: 0,
        block_type: "text".into(),
    }];
    for ch in text.chars() {
        chunks.push(StreamChunk::TextDelta {
            index: 0,
            text: ch.to_string(),
        });
    }
    chunks.push(StreamChunk::BlockEnd {
        index: 0,
        block: ContentBlock::Text {
            text: text.to_string(),
        },
    });
    chunks.push(StreamChunk::Usage {
        usage: TokenUsage {
            input_tokens: 10,
            output_tokens: text.chars().count() as u64,
            cache_read_tokens: None,
            cache_write_tokens: None,
            reasoning_tokens: None,
        },
    });
    chunks.push(StreamChunk::Finish {
        reason,
        replay_state: None,
    });
    chunks
}

/// Optional leading text block, then a tool-call whose arguments split at 5 bytes, usage, and `tool-calls`.
#[must_use]
pub fn tool_call_response(
    call_id: &str,
    name: &str,
    args: &serde_json::Value,
    text: Option<&str>,
) -> Vec<StreamChunk> {
    let call_id = CallId::new(call_id);
    let arguments_json = args.to_string();
    let mut chunks = Vec::new();
    let mut index = 0_u32;
    if let Some(text) = text {
        chunks.push(StreamChunk::BlockStart {
            index,
            block_type: "text".into(),
        });
        chunks.push(StreamChunk::TextDelta {
            index,
            text: text.to_string(),
        });
        chunks.push(StreamChunk::BlockEnd {
            index,
            block: ContentBlock::Text {
                text: text.to_string(),
            },
        });
        index += 1;
    }
    let (head, tail) = split_at_bytes(&arguments_json, 5);
    chunks.push(StreamChunk::BlockStart {
        index,
        block_type: "tool-call".into(),
    });
    chunks.push(StreamChunk::ToolCallDelta {
        index,
        id: call_id.clone(),
        name: Some(name.to_string()),
        arguments_delta: head.to_string(),
    });
    chunks.push(StreamChunk::ToolCallDelta {
        index,
        id: call_id.clone(),
        name: None,
        arguments_delta: tail.to_string(),
    });
    chunks.push(StreamChunk::BlockEnd {
        index,
        block: ContentBlock::ToolCall {
            id: call_id,
            name: name.to_string(),
            arguments: arguments_json,
        },
    });
    chunks.push(StreamChunk::Usage {
        usage: TokenUsage {
            input_tokens: 10,
            output_tokens: 5,
            cache_read_tokens: None,
            cache_write_tokens: None,
            reasoning_tokens: None,
        },
    });
    chunks.push(StreamChunk::Finish {
        reason: FinishReason::ToolCalls,
        replay_state: None,
    });
    chunks
}

fn split_at_bytes(value: &str, n: usize) -> (&str, &str) {
    let mut split = n.min(value.len());
    while split > 0 && !value.is_char_boundary(split) {
        split -= 1;
    }
    value.split_at(split)
}
