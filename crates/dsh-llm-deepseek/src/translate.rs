//! Translate DeepSeek SSE payloads into harness [`dsh_session::StreamChunk`] values.

use std::collections::HashMap;

use dsh_llm::{EMPTY_RESPONSE_CODE, LlmError};
use dsh_session::{CallId, ContentBlock, FinishReason, LlmFailure, StreamChunk, TokenUsage};
use futures::Stream;
use futures::StreamExt;

use crate::sse::DONE;
use crate::types::{WireChunk, WireUsage};

struct OpenBlock {
    index: u32,
    kind: OpenKind,
    text: String,
    call_id: Option<String>,
    name: Option<String>,
}

enum OpenKind {
    Text,
    Reasoning,
    ToolCall,
}

fn open_block(next_index: &mut u32, order: &mut Vec<OpenBlock>, kind: OpenKind) -> usize {
    let index = *next_index;
    *next_index += 1;
    order.push(OpenBlock {
        index,
        kind,
        text: String::new(),
        call_id: None,
        name: None,
    });
    order.len() - 1
}

fn close_block(block: OpenBlock) -> ContentBlock {
    match block.kind {
        OpenKind::Text => ContentBlock::Text { text: block.text },
        OpenKind::Reasoning => ContentBlock::Reasoning { text: block.text },
        OpenKind::ToolCall => ContentBlock::ToolCall {
            id: CallId::new(block.call_id.unwrap_or_default()),
            name: block.name.unwrap_or_default(),
            arguments: block.text,
        },
    }
}

/// Map the wire `finish_reason` string to a harness [`FinishReason`].
///
/// Unrecognized values become [`FinishReason::Error`] with `code` equal to the
/// uppercased reason.
#[must_use]
pub fn map_finish_reason(reason: &str) -> FinishReason {
    match reason {
        "stop" => FinishReason::Stop,
        "tool_calls" => FinishReason::ToolCalls,
        "length" => FinishReason::MaxTokens,
        other => FinishReason::Error {
            failure: LlmFailure {
                message: format!("model stopped: {other}"),
                code: other.to_ascii_uppercase(),
                status: None,
                provider_retry_after_ms: None,
                request_id: None,
            },
        },
    }
}

/// Map wire usage to disjoint harness counts.
///
/// `input_tokens = prompt_tokens - cache_read`, where `cache_read` is
/// `prompt_tokens_details.cached_tokens` falling back to `prompt_cache_hit_tokens`.
#[must_use]
pub fn map_usage(usage: &WireUsage) -> TokenUsage {
    let cache_read = usage
        .prompt_tokens_details
        .as_ref()
        .and_then(|details| details.cached_tokens)
        .or(usage.prompt_cache_hit_tokens);
    let reasoning = usage
        .completion_tokens_details
        .as_ref()
        .and_then(|details| details.reasoning_tokens);
    TokenUsage {
        input_tokens: usage.prompt_tokens.saturating_sub(cache_read.unwrap_or(0)),
        output_tokens: usage.completion_tokens,
        cache_read_tokens: cache_read,
        cache_write_tokens: None,
        reasoning_tokens: reasoning,
    }
}

/// Consume SSE data payloads (ending with `[DONE]`) and collect [`StreamChunk`]s.
///
/// Empty initial reasoning does not open a block. Usage and finish are deferred
/// until `[DONE]`. A stop (or absent) finish with no opened blocks becomes an
/// `EMPTY_RESPONSE` error finish.
///
/// # Errors
///
/// Returns [`LlmError`] `MALFORMED_RESPONSE` when a payload is not JSON, or
/// `STREAM_CLOSED` when the payload stream ends without `[DONE]`.
pub async fn translate(
    mut payloads: impl Stream<Item = String> + Unpin,
) -> Result<Vec<StreamChunk>, LlmError> {
    let mut next_index = 0_u32;
    let mut text_idx: Option<usize> = None;
    let mut reasoning_idx: Option<usize> = None;
    let mut tool_blocks: HashMap<u32, usize> = HashMap::new();
    let mut order: Vec<OpenBlock> = Vec::new();
    let mut pending_finish: Option<FinishReason> = None;
    let mut pending_usage: Option<TokenUsage> = None;
    let mut chunks = Vec::new();

    while let Some(payload) = payloads.next().await {
        if payload == DONE {
            let opened = order.len();
            for block in order {
                chunks.push(StreamChunk::BlockEnd {
                    index: block.index,
                    block: close_block(block),
                });
            }
            if let Some(usage) = pending_usage {
                chunks.push(StreamChunk::Usage { usage });
            }
            let reason = pending_finish.unwrap_or(FinishReason::Stop);
            let reason = if opened == 0 && matches!(reason, FinishReason::Stop) {
                FinishReason::Error {
                    failure: LlmFailure {
                        message: "model returned a completed response with no content".into(),
                        code: EMPTY_RESPONSE_CODE.into(),
                        status: None,
                        provider_retry_after_ms: None,
                        request_id: None,
                    },
                }
            } else {
                reason
            };
            chunks.push(StreamChunk::Finish {
                reason,
                replay_state: None,
            });
            return Ok(chunks);
        }

        let chunk: WireChunk = serde_json::from_str(&payload).map_err(|_| {
            let preview: String = payload.chars().take(120).collect();
            LlmError::new(
                format!("malformed SSE payload: {preview}"),
                "MALFORMED_RESPONSE",
            )
        })?;

        for choice in chunk.choices {
            let delta = choice.delta;
            if let Some(reasoning) = delta
                .as_ref()
                .and_then(|delta| delta.reasoning_content.as_deref())
            {
                if !reasoning.is_empty() {
                    let idx = match reasoning_idx {
                        Some(idx) => idx,
                        None => {
                            let idx = open_block(&mut next_index, &mut order, OpenKind::Reasoning);
                            reasoning_idx = Some(idx);
                            chunks.push(StreamChunk::BlockStart {
                                index: order[idx].index,
                                block_type: "reasoning".into(),
                            });
                            idx
                        }
                    };
                    order[idx].text.push_str(reasoning);
                    chunks.push(StreamChunk::ReasoningDelta {
                        index: order[idx].index,
                        text: reasoning.to_string(),
                    });
                }
            }

            if let Some(content) = delta.as_ref().and_then(|delta| delta.content.as_deref()) {
                if !content.is_empty() {
                    let idx = match text_idx {
                        Some(idx) => idx,
                        None => {
                            let idx = open_block(&mut next_index, &mut order, OpenKind::Text);
                            text_idx = Some(idx);
                            chunks.push(StreamChunk::BlockStart {
                                index: order[idx].index,
                                block_type: "text".into(),
                            });
                            idx
                        }
                    };
                    order[idx].text.push_str(content);
                    chunks.push(StreamChunk::TextDelta {
                        index: order[idx].index,
                        text: content.to_string(),
                    });
                }
            }

            for call in delta
                .as_ref()
                .and_then(|delta| delta.tool_calls.as_ref())
                .into_iter()
                .flatten()
            {
                let idx = match tool_blocks.get(&call.index).copied() {
                    Some(idx) => idx,
                    None => {
                        let idx = open_block(&mut next_index, &mut order, OpenKind::ToolCall);
                        tool_blocks.insert(call.index, idx);
                        chunks.push(StreamChunk::BlockStart {
                            index: order[idx].index,
                            block_type: "tool-call".into(),
                        });
                        idx
                    }
                };
                if let Some(id) = &call.id {
                    order[idx].call_id = Some(id.clone());
                }
                if let Some(name) = call
                    .function
                    .as_ref()
                    .and_then(|function| function.name.clone())
                {
                    order[idx].name = Some(name);
                }
                let fragment = call
                    .function
                    .as_ref()
                    .and_then(|function| function.arguments.as_deref())
                    .unwrap_or("");
                order[idx].text.push_str(fragment);
                chunks.push(StreamChunk::ToolCallDelta {
                    index: order[idx].index,
                    id: CallId::new(order[idx].call_id.clone().unwrap_or_default()),
                    name: order[idx].name.clone(),
                    arguments_delta: fragment.to_string(),
                });
            }

            if let Some(reason) = choice.finish_reason.as_deref() {
                pending_finish = Some(map_finish_reason(reason));
            }
        }

        if let Some(usage) = chunk.usage {
            pending_usage = Some(map_usage(&usage));
        }
    }

    Err(LlmError::new(
        "SSE payload stream ended without [DONE]",
        "STREAM_CLOSED",
    ))
}

#[cfg(test)]
mod tests {
    use super::{map_finish_reason, map_usage, translate};
    use crate::sse::DONE;
    use crate::types::WireUsage;
    use dsh_llm::EMPTY_RESPONSE_CODE;
    use dsh_session::{FinishReason, StreamChunk};
    use futures::stream;

    #[tokio::test]
    async fn empty_reasoning_does_not_open_a_block_and_usage_precedes_finish() {
        let payloads = stream::iter([
            r#"{"choices":[{"delta":{"reasoning_content":""}}]}"#.to_string(),
            r#"{"choices":[{"delta":{"content":"hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":2,"prompt_cache_hit_tokens":4}}"#.to_string(),
            DONE.to_string(),
        ]);
        let chunks = translate(payloads).await.unwrap();
        assert!(chunks.iter().all(|c| !matches!(c, StreamChunk::BlockStart { block_type, .. } if block_type == "reasoning")));
        let usage_at = chunks
            .iter()
            .position(|c| matches!(c, StreamChunk::Usage { .. }))
            .unwrap();
        let finish_at = chunks
            .iter()
            .position(|c| matches!(c, StreamChunk::Finish { .. }))
            .unwrap();
        assert!(usage_at < finish_at);
        assert_eq!(finish_at, chunks.len() - 1);
        match &chunks[usage_at] {
            StreamChunk::Usage { usage } => {
                assert_eq!(usage.input_tokens, 6);
                assert_eq!(usage.cache_read_tokens, Some(4));
                assert_eq!(usage.output_tokens, 2);
            }
            _ => panic!("usage"),
        }
    }

    #[tokio::test]
    async fn empty_stop_is_empty_response() {
        let payloads = stream::iter([
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#.to_string(),
            DONE.to_string(),
        ]);
        let chunks = translate(payloads).await.unwrap();
        match chunks.last() {
            Some(StreamChunk::Finish {
                reason: FinishReason::Error { failure },
                ..
            }) => {
                assert_eq!(failure.code, EMPTY_RESPONSE_CODE);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn finish_reason_and_usage_maps() {
        assert!(matches!(
            map_finish_reason("length"),
            FinishReason::MaxTokens
        ));
        assert!(matches!(
            map_finish_reason("tool_calls"),
            FinishReason::ToolCalls
        ));
        let usage = map_usage(&WireUsage {
            prompt_tokens: 10,
            completion_tokens: 3,
            prompt_cache_hit_tokens: Some(4),
            prompt_tokens_details: None,
            completion_tokens_details: None,
        });
        assert_eq!(usage.input_tokens, 6);
        assert_eq!(usage.cache_read_tokens, Some(4));
    }
}
