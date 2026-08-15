//! Serialize harness messages into DeepSeek chat-completions requests.

use dsh_llm::{GenerateOptions, LlmError, LlmPurpose};
use dsh_session::{ContentBlock, Message, MessageRole};

use crate::types::{
    StreamOptions, ThinkingField, ThinkingMode, WireFunction, WireMessage, WireRequest, WireTool,
    WireToolCall, WireToolFunction,
};

/// Adapter-level thinking defaults applied when a request omits an effort.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RequestDefaults {
    /// Deployment thinking lock; `None` leaves thinking off the wire.
    pub thinking: Option<ThinkingMode>,
    /// Deployment effort: `"off"`, `"high"`, or `"max"`.
    pub reasoning_effort: Option<String>,
}

struct ResolvedThinking {
    thinking: Option<ThinkingMode>,
    reasoning_effort: Option<String>,
}

fn reasoning_effort(effort: &str) -> Result<&str, LlmError> {
    match effort {
        "off" | "high" | "max" => Ok(effort),
        other => Err(LlmError::new(
            format!("DeepSeek does not support reasoning effort \"{other}\""),
            "UNSUPPORTED_REASONING_EFFORT",
        )),
    }
}

fn resolve_thinking(
    options: &GenerateOptions,
    defaults: &RequestDefaults,
) -> Result<ResolvedThinking, LlmError> {
    if options.purpose == Some(LlmPurpose::SessionTitle) {
        return Ok(ResolvedThinking {
            thinking: Some(ThinkingMode::Disabled),
            reasoning_effort: None,
        });
    }
    let effort = match options.reasoning_effort.as_deref() {
        None => defaults.reasoning_effort.as_deref(),
        Some(value) => Some(reasoning_effort(value)?),
    };
    if let Some(effort) = effort {
        if defaults.thinking == Some(ThinkingMode::Disabled) && effort != "off" {
            return Err(LlmError::new(
                format!("DeepSeek deployment does not support reasoning effort \"{effort}\""),
                "UNSUPPORTED_REASONING_EFFORT",
            ));
        }
        if effort == "off" {
            return Ok(ResolvedThinking {
                thinking: Some(ThinkingMode::Disabled),
                reasoning_effort: None,
            });
        }
        if effort == "high" || effort == "max" {
            return Ok(ResolvedThinking {
                thinking: Some(ThinkingMode::Enabled),
                reasoning_effort: Some(effort.to_string()),
            });
        }
    }
    Ok(ResolvedThinking {
        thinking: defaults.thinking,
        reasoning_effort: None,
    })
}

fn flatten_text(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn content_has_image(blocks: &[ContentBlock]) -> bool {
    blocks.iter().any(|block| match block {
        ContentBlock::Image { .. } => true,
        ContentBlock::ToolResult { content, .. } => content_has_image(content),
        _ => false,
    })
}

fn serialize_assistant(message: &Message) -> WireMessage {
    let text = flatten_text(&message.content);
    let reasoning: String = message
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Reasoning { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    let tool_calls: Vec<WireToolCall> = message
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::ToolCall {
                id,
                name,
                arguments,
            } => Some(WireToolCall {
                id: id.as_str().to_string(),
                kind: "function",
                function: WireFunction {
                    name: name.clone(),
                    arguments: arguments.clone(),
                },
            }),
            _ => None,
        })
        .collect();
    WireMessage::Assistant {
        content: text,
        reasoning_content: if !tool_calls.is_empty() && !reasoning.is_empty() {
            Some(reasoning)
        } else {
            None
        },
        tool_calls: if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls)
        },
    }
}

/// Serialize the conversation. Tool-result blocks become `{role: tool}` messages.
///
/// # Errors
///
/// Returns [`LlmError`] with code `UNSUPPORTED_CONTENT` when any message carries
/// an image block (including nested tool-result content).
pub fn serialize_messages(messages: &[Message]) -> Result<Vec<WireMessage>, LlmError> {
    let mut wire = Vec::new();
    for message in messages {
        if content_has_image(&message.content) {
            return Err(LlmError::new(
                "The DeepSeek chat-completions adapter does not support image content.",
                "UNSUPPORTED_CONTENT",
            ));
        }
        match message.role {
            MessageRole::System => {
                wire.push(WireMessage::System {
                    content: flatten_text(&message.content),
                });
            }
            MessageRole::Assistant => wire.push(serialize_assistant(message)),
            MessageRole::User => {
                let tool_results: Vec<_> = message
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        ContentBlock::ToolResult {
                            tool_call_id,
                            content,
                            ..
                        } => Some((tool_call_id, content)),
                        _ => None,
                    })
                    .collect();
                let text = flatten_text(&message.content);
                if !text.is_empty() || tool_results.is_empty() {
                    wire.push(WireMessage::User { content: text });
                }
                for (tool_call_id, content) in tool_results {
                    let output = flatten_text(content);
                    wire.push(WireMessage::Tool {
                        tool_call_id: tool_call_id.as_str().to_string(),
                        content: if output.is_empty() {
                            "(no output)".into()
                        } else {
                            output
                        },
                    });
                }
            }
        }
    }
    Ok(wire)
}

/// Build the streaming chat-completions request body.
///
/// Always sets `stream: true` and `stream_options.include_usage: true`. Optional
/// fields are omitted rather than sent as JSON `null`.
///
/// # Errors
///
/// Returns [`LlmError`] `UNSUPPORTED_CONTENT` from [`serialize_messages`], or
/// `UNSUPPORTED_REASONING_EFFORT` when the effort is unknown or locked off.
pub fn serialize_request(
    options: &GenerateOptions,
    defaults: &RequestDefaults,
) -> Result<WireRequest, LlmError> {
    let mut messages = Vec::new();
    if let Some(system) = &options.system {
        messages.push(WireMessage::System {
            content: system.clone(),
        });
    }
    messages.extend(serialize_messages(&options.messages)?);
    let tools = options.tools.as_ref().and_then(|tools| {
        if tools.is_empty() {
            None
        } else {
            Some(
                tools
                    .iter()
                    .map(|tool| WireTool {
                        kind: "function",
                        function: WireToolFunction {
                            name: tool.name.clone(),
                            description: tool.description.clone(),
                            parameters: tool.parameters.clone(),
                        },
                    })
                    .collect(),
            )
        }
    });
    let resolved = resolve_thinking(options, defaults)?;
    Ok(WireRequest {
        model: options.model.clone(),
        messages,
        stream: true,
        stream_options: StreamOptions {
            include_usage: true,
        },
        thinking: resolved.thinking.map(|kind| ThinkingField { kind }),
        reasoning_effort: resolved.reasoning_effort,
        tools,
        temperature: options.temperature,
        max_tokens: options.max_tokens,
        stop: options.stop.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::{RequestDefaults, serialize_messages, serialize_request};
    use dsh_llm::{GenerateOptions, LlmPurpose};
    use dsh_session::{CallId, ContentBlock, Message, MessageId, MessageRole, MessageSource};
    use dsh_tools::AbortFlag;
    use serde_json::{Value, json};

    fn user(text: &str) -> Message {
        Message {
            id: MessageId::new("u1"),
            role: MessageRole::User,
            content: vec![ContentBlock::Text { text: text.into() }],
            source: MessageSource::User,
        }
    }

    fn options(messages: Vec<Message>) -> GenerateOptions {
        GenerateOptions {
            provider: "deepseek".into(),
            model: "deepseek-chat".into(),
            reasoning_effort: None,
            messages,
            system: Some("sys".into()),
            tools: None,
            temperature: None,
            max_tokens: None,
            stop: None,
            signal: AbortFlag::new(),
            session_id: None,
            purpose: None,
        }
    }

    #[test]
    fn assistant_content_is_empty_string_never_null() {
        let assistant = Message {
            id: MessageId::new("a1"),
            role: MessageRole::Assistant,
            content: vec![ContentBlock::ToolCall {
                id: CallId::new("c1"),
                name: "echo".into(),
                arguments: "{}".into(),
            }],
            source: MessageSource::Model {
                provider: "deepseek".into(),
                model: "deepseek-chat".into(),
                replay_state: None,
            },
        };
        let wire = serialize_messages(&[assistant]).unwrap();
        let value = serde_json::to_value(&wire[0]).unwrap();
        assert_eq!(value["content"], json!(""));
        assert_ne!(value["content"], Value::Null);
        assert!(value.get("reasoning_content").is_none());
    }

    #[test]
    fn reasoning_passback_only_on_tool_call_turns() {
        let with_tools = Message {
            id: MessageId::new("a1"),
            role: MessageRole::Assistant,
            content: vec![
                ContentBlock::Reasoning {
                    text: "think".into(),
                },
                ContentBlock::ToolCall {
                    id: CallId::new("c1"),
                    name: "echo".into(),
                    arguments: "{}".into(),
                },
            ],
            source: MessageSource::Model {
                provider: "p".into(),
                model: "m".into(),
                replay_state: None,
            },
        };
        let plain = Message {
            id: MessageId::new("a2"),
            role: MessageRole::Assistant,
            content: vec![
                ContentBlock::Reasoning {
                    text: "think".into(),
                },
                ContentBlock::Text { text: "hi".into() },
            ],
            source: MessageSource::Model {
                provider: "p".into(),
                model: "m".into(),
                replay_state: None,
            },
        };
        let tool_wire =
            serde_json::to_value(&serialize_messages(&[with_tools]).unwrap()[0]).unwrap();
        let plain_wire = serde_json::to_value(&serialize_messages(&[plain]).unwrap()[0]).unwrap();
        assert_eq!(tool_wire["reasoning_content"], json!("think"));
        assert!(plain_wire.get("reasoning_content").is_none());
    }

    #[test]
    fn empty_tool_output_becomes_no_output() {
        let message = Message {
            id: MessageId::new("t1"),
            role: MessageRole::User,
            content: vec![ContentBlock::ToolResult {
                tool_call_id: CallId::new("c1"),
                content: vec![],
                is_error: None,
            }],
            source: MessageSource::Tool {
                call_id: CallId::new("c1"),
            },
        };
        let wire = serialize_messages(&[message]).unwrap();
        let value = serde_json::to_value(&wire[0]).unwrap();
        assert_eq!(value["role"], json!("tool"));
        assert_eq!(value["content"], json!("(no output)"));
    }

    #[test]
    fn thinking_and_effort_mapping() {
        let mut off = options(vec![user("hi")]);
        off.reasoning_effort = Some("off".into());
        let body =
            serde_json::to_value(serialize_request(&off, &RequestDefaults::default()).unwrap())
                .unwrap();
        assert_eq!(body["thinking"]["type"], json!("disabled"));
        assert!(body.get("reasoning_effort").is_none());

        let mut high = options(vec![user("hi")]);
        high.reasoning_effort = Some("high".into());
        let body =
            serde_json::to_value(serialize_request(&high, &RequestDefaults::default()).unwrap())
                .unwrap();
        assert_eq!(body["thinking"]["type"], json!("enabled"));
        assert_eq!(body["reasoning_effort"], json!("high"));

        let mut title = options(vec![user("hi")]);
        title.purpose = Some(LlmPurpose::SessionTitle);
        title.reasoning_effort = Some("high".into());
        let body =
            serde_json::to_value(serialize_request(&title, &RequestDefaults::default()).unwrap())
                .unwrap();
        assert_eq!(body["thinking"]["type"], json!("disabled"));
        assert!(body.get("reasoning_effort").is_none());
    }

    #[test]
    fn image_content_is_unsupported() {
        let message = Message {
            id: MessageId::new("u1"),
            role: MessageRole::User,
            content: vec![ContentBlock::Image {
                attachment: json!({"id": "img"}),
            }],
            source: MessageSource::User,
        };
        let error = serialize_messages(&[message]).expect_err("image");
        assert_eq!(error.code, "UNSUPPORTED_CONTENT");
    }
}
