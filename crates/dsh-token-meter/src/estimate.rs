//! Fixed-density heuristic token pricing shared by the meter fold.

use dsh_session::{ContentBlock, EpochHeader, Message};
use serde_json::Value;

/// Fixed text-density estimate used until exact tokenization is needed.
pub const CHARS_PER_TOKEN: u32 = 4;

/// Per-block structural overhead for JSON framing and type tags.
pub const BLOCK_OVERHEAD: u32 = 4;

/// Role-field framing overhead added to every priced message and to system text.
pub const ROLE_OVERHEAD: u32 = 4;

fn ceil_chars(text: &str) -> u64 {
    (text.encode_utf16().count() as u64).div_ceil(u64::from(CHARS_PER_TOKEN))
}

fn json_token_len(value: &Value) -> u64 {
    let json = serde_json::to_string(value).expect("token-meter JSON");
    ceil_chars(&json)
}

/// Price content blocks recursively under the fixed density heuristic.
///
/// # Parameters
///
/// * `blocks` - content blocks to price without mutation.
///
/// # Returns
///
/// Heuristic tokens including per-block structural overhead.
#[must_use]
pub fn estimate_content(blocks: &[ContentBlock]) -> u64 {
    let mut tokens = 0;
    for block in blocks {
        match block {
            ContentBlock::Text { text } | ContentBlock::Reasoning { text } => {
                tokens += ceil_chars(text) + u64::from(BLOCK_OVERHEAD);
            }
            ContentBlock::ToolCall {
                name, arguments, ..
            } => {
                tokens += ceil_chars(name) + ceil_chars(arguments) + u64::from(BLOCK_OVERHEAD);
            }
            ContentBlock::ToolResult { content, .. } => {
                tokens += estimate_content(content) + u64::from(BLOCK_OVERHEAD);
            }
            ContentBlock::Image { .. } => {
                let json = serde_json::to_string(block).expect("token-meter JSON");
                tokens += u64::from(BLOCK_OVERHEAD) + ceil_chars(&json);
            }
        }
    }
    tokens
}

/// Heuristically price one model-visible message.
///
/// # Parameters
///
/// * `message` - message to price without mutation.
///
/// # Returns
///
/// Content and role-framing tokens under the fixed heuristic.
#[must_use]
pub fn estimate_message(message: &Message) -> u64 {
    estimate_content(&message.content) + u64::from(ROLE_OVERHEAD)
}

fn estimate_system_tokens(header: Option<&EpochHeader>) -> u64 {
    let Some(header) = header else {
        return 0;
    };
    let Some(system) = header.system.as_deref() else {
        return 0;
    };
    ceil_chars(system) + u64::from(ROLE_OVERHEAD)
}

fn estimate_tools_tokens(header: Option<&EpochHeader>) -> u64 {
    let Some(header) = header else {
        return 0;
    };
    let Some(tools) = header.tools.as_ref() else {
        return 0;
    };
    let empty = match tools {
        Value::Array(items) => items.is_empty(),
        Value::String(text) => text.is_empty(),
        Value::Null => true,
        Value::Bool(_) | Value::Number(_) | Value::Object(_) => false,
    };
    if empty {
        return 0;
    }
    json_token_len(tools) + u64::from(BLOCK_OVERHEAD)
}

/// Price the complete non-surface request envelope.
///
/// # Parameters
///
/// * `header` - canonical envelope, or `None` before any request.
///
/// # Returns
///
/// Heuristic system plus tool tokens; 0 when absent.
#[must_use]
pub fn estimate_header(header: Option<&EpochHeader>) -> u64 {
    estimate_system_tokens(header) + estimate_tools_tokens(header)
}

#[cfg(test)]
mod tests {
    use super::{estimate_content, estimate_header, estimate_message};
    use dsh_session::{
        CallId, ContentBlock, EpochHeader, LlmCallConfig, Message, MessageId, MessageRole,
        MessageSource,
    };
    use serde_json::json;

    fn user_text(text: &str) -> Message {
        Message {
            id: MessageId::new("m"),
            role: MessageRole::User,
            content: vec![ContentBlock::Text { text: text.into() }],
            source: MessageSource::User,
        }
    }

    fn call_config() -> LlmCallConfig {
        LlmCallConfig {
            provider: "mock".into(),
            model: "m".into(),
            reasoning_effort: None,
            temperature: None,
            max_tokens: None,
            stop: None,
        }
    }

    #[test]
    fn estimate_text_matches_ts_heuristic() {
        let msg = user_text("abcd");
        assert_eq!(estimate_message(&msg), 9);
    }

    #[test]
    fn estimate_empty_text_is_overheads_only() {
        assert_eq!(estimate_message(&user_text("")), 8);
    }

    #[test]
    fn estimate_five_chars_ceils() {
        assert_eq!(estimate_message(&user_text("abcde")), 10);
    }

    #[test]
    fn estimate_tool_call_prices_name_and_arguments() {
        let message = Message {
            id: MessageId::new("m"),
            role: MessageRole::Assistant,
            content: vec![ContentBlock::ToolCall {
                id: CallId::new("c"),
                name: "read".into(),
                arguments: "{\"x\":1}".into(),
            }],
            source: MessageSource::User,
        };
        assert_eq!(estimate_message(&message), 11);
    }

    #[test]
    fn estimate_tool_result_recurses_then_adds_block_overhead() {
        let message = Message {
            id: MessageId::new("m"),
            role: MessageRole::User,
            content: vec![ContentBlock::ToolResult {
                tool_call_id: CallId::new("c"),
                content: vec![ContentBlock::Text { text: "xy".into() }],
                is_error: None,
            }],
            source: MessageSource::User,
        };
        assert_eq!(estimate_message(&message), 13);
    }

    #[test]
    fn estimate_image_uses_json_stringify_plus_block_overhead() {
        let block = ContentBlock::Image {
            attachment: json!({"id": "a"}),
        };
        let json = serde_json::to_string(&block).expect("image JSON");
        let json_tokens = u64::try_from(json.encode_utf16().count())
            .expect("json len")
            .div_ceil(4);
        assert_eq!(estimate_content(&[block.clone()]), 4 + json_tokens);
        let message = Message {
            id: MessageId::new("m"),
            role: MessageRole::User,
            content: vec![block],
            source: MessageSource::User,
        };
        assert_eq!(estimate_message(&message), 8 + json_tokens);
    }

    #[test]
    fn estimate_header_system_and_tools() {
        let header = EpochHeader {
            config: call_config(),
            adapter_defaults: None,
            system: Some("abcd".into()),
            tools: Some(json!([{"n": 1}])),
        };
        assert_eq!(estimate_header(Some(&header)), 12);
        assert_eq!(estimate_header(None), 0);
    }
}
