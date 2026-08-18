//! Native text projection of MCP tool-result content blocks.

use serde_json::Value;

/// Project MCP content blocks to one Native text string.
///
/// Non-objects become `[unsupported content type: unknown]`. `text` contributes
/// `text` when present. `image` / `audio` become
/// `[image|audio: {mimeType or unknown}, content discarded]`. `resource` and
/// `resource_link` become `[resource: content discarded]`. Any other `type`
/// becomes `[unsupported content type: {type}]`.
///
/// # Parameters
///
/// * `mcp_content` - MCP `content` array values in wire order.
/// * `tool_name` - Name used only in the empty-content fallback message.
///
/// # Returns
///
/// Parts joined with `\n`. When that string is empty,
/// `({tool_name} returned no text content)`.
#[must_use]
pub fn extract_text(mcp_content: &[Value], tool_name: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    for value in mcp_content {
        let Some(block) = value.as_object() else {
            parts.push("[unsupported content type: unknown]".to_string());
            continue;
        };
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    parts.push(text.to_string());
                }
            }
            Some("image") => {
                parts.push(format!(
                    "[image: {}, content discarded]",
                    mime_or_unknown(block.get("mimeType"))
                ));
            }
            Some("audio") => {
                parts.push(format!(
                    "[audio: {}, content discarded]",
                    mime_or_unknown(block.get("mimeType"))
                ));
            }
            Some("resource" | "resource_link") => {
                parts.push("[resource: content discarded]".to_string());
            }
            Some(other) => {
                parts.push(format!("[unsupported content type: {other}]"));
            }
            None => {
                let label = match block.get("type") {
                    None => "undefined".to_string(),
                    Some(value) => value.to_string(),
                };
                parts.push(format!("[unsupported content type: {label}]"));
            }
        }
    }
    let joined = parts.join("\n");
    if joined.is_empty() {
        format!("({tool_name} returned no text content)")
    } else {
        joined
    }
}

fn mime_or_unknown(value: Option<&Value>) -> &str {
    match value.and_then(Value::as_str) {
        Some(mime) => mime,
        None => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::extract_text;
    use serde_json::json;

    #[test]
    fn extract_text_joins_text_blocks_with_newline() {
        let content = [
            json!({"type": "text", "text": "hello"}),
            json!({"type": "text", "text": "world"}),
        ];
        assert_eq!(extract_text(&content, "tool"), "hello\nworld");
    }

    #[test]
    fn extract_text_image_placeholder() {
        let content = [json!({"type": "image", "mimeType": "image/png"})];
        assert_eq!(
            extract_text(&content, "tool"),
            "[image: image/png, content discarded]"
        );
    }

    #[test]
    fn extract_text_empty_uses_no_text_message() {
        assert_eq!(
            extract_text(&[], "empty_tool"),
            "(empty_tool returned no text content)"
        );
    }

    #[test]
    fn extract_text_unknown_type_placeholder() {
        let content = [json!({"type": "video"})];
        assert_eq!(
            extract_text(&content, "tool"),
            "[unsupported content type: video]"
        );
    }
}
