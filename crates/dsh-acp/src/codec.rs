//! Pure translation between the harness lifecycle and the automation-only ACP wire.

use dsh_session::TurnEndReason;
use serde::{Deserialize, Serialize};

/// ACP prompt content block. Unknown and non-baseline types deserialize as [`Self::Other`].
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "type")]
pub enum AcpContentBlock {
    /// Baseline text block.
    #[serde(rename = "text")]
    Text {
        /// Verbatim prompt text.
        text: String,
    },
    /// Baseline resource link.
    #[serde(rename = "resource_link")]
    ResourceLink {
        /// Resource display name.
        name: String,
        /// Resource URI.
        uri: String,
    },
    /// Image, audio, embedded resource, or any other non-baseline type.
    #[serde(other)]
    Other,
}

/// ACP prompt `stopReason` wire token.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// Normal quiescence.
    EndTurn,
    /// Output token ceiling. Codec maps here; [`prompt_stop_reason`] reports `end_turn`.
    MaxTokens,
    /// Explicit cancel, disposal, or a turnless prompt slot.
    Cancelled,
    /// ACP `refusal` token.
    Refusal,
    /// ACP `max_turn_requests` token.
    MaxTurnRequests,
}

impl StopReason {
    /// ACP wire spelling of this stop reason.
    ///
    /// # Returns
    ///
    /// Snake-case `stopReason` token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EndTurn => "end_turn",
            Self::MaxTokens => "max_tokens",
            Self::Cancelled => "cancelled",
            Self::Refusal => "refusal",
            Self::MaxTurnRequests => "max_turn_requests",
        }
    }
}

/// Map a harness turn ending to ACP's terminal reason vocabulary.
///
/// `cancelled` is reserved for explicit client cancellation (`session/cancel`)
/// and disposal; a turn aborted by a hook or another owner reports `end_turn`.
///
/// # Parameters
///
/// * `reason` - harness turn outcome.
///
/// # Returns
///
/// The closest legal ACP stop reason.
#[must_use]
pub fn turn_end_to_stop_reason(reason: &TurnEndReason) -> StopReason {
    match reason {
        TurnEndReason::Completed => StopReason::EndTurn,
        TurnEndReason::MaxTokens => StopReason::MaxTokens,
        TurnEndReason::Aborted { .. } => StopReason::EndTurn,
        TurnEndReason::Interrupted => StopReason::Cancelled,
        TurnEndReason::Blocked | TurnEndReason::Error { .. } => StopReason::EndTurn,
    }
}

/// Prompt-RPC settlement stop reason.
///
/// Token-limit turn endings do not become prompt-level ACP stop reasons.
///
/// # Parameters
///
/// * `end` - last non-error `TurnEnd` for the in-flight prompt, if any.
///
/// # Returns
///
/// `cancelled` when `end` is `None`; `end_turn` when `end` is `MaxTokens`;
/// otherwise [`turn_end_to_stop_reason`].
#[must_use]
pub fn prompt_stop_reason(end: Option<&TurnEndReason>) -> StopReason {
    match end {
        None => StopReason::Cancelled,
        Some(TurnEndReason::MaxTokens) => StopReason::EndTurn,
        Some(reason) => turn_end_to_stop_reason(reason),
    }
}

/// Flatten an ACP prompt's baseline blocks to text.
///
/// Text blocks concatenate verbatim. Resource links become explicit textual
/// references so a baseline client can point at files without the bridge
/// silently dropping that context. Unsupported blocks contribute nothing.
///
/// # Parameters
///
/// * `prompt` - ACP prompt blocks in wire order.
///
/// # Returns
///
/// Text in wire order, with resource links rendered as bracketed references.
#[must_use]
pub fn acp_prompt_to_text(prompt: &[AcpContentBlock]) -> String {
    let mut out = String::new();
    for block in prompt {
        match block {
            AcpContentBlock::Text { text } => out.push_str(text),
            AcpContentBlock::ResourceLink { name, uri } => {
                out.push_str(
                    &("\n[resource_link name=".to_string()
                        + &serde_json::to_string(name).unwrap()
                        + " uri="
                        + &serde_json::to_string(uri).unwrap()
                        + "]\n"),
                );
            }
            AcpContentBlock::Other => {}
        }
    }
    out
}

/// Whether a prompt carries content beyond the ACP baseline.
///
/// Every agent must accept `text` and `resource_link`. Richer inline payloads
/// (image, audio, embedded resource) are optional capabilities this bridge does
/// not advertise, so they are rejected rather than silently dropped.
///
/// # Parameters
///
/// * `prompt` - ACP prompt blocks to inspect.
///
/// # Returns
///
/// `true` when any block is neither `text` nor `resource_link`.
#[must_use]
pub fn prompt_has_unsupported_content(prompt: &[AcpContentBlock]) -> bool {
    prompt.iter().any(|block| {
        !matches!(
            block,
            AcpContentBlock::Text { .. } | AcpContentBlock::ResourceLink { .. }
        )
    })
}

#[cfg(test)]
mod tests {
    use dsh_session::{LlmFailure, TurnEndReason};
    use serde_json::json;

    use super::{
        AcpContentBlock, acp_prompt_to_text, prompt_has_unsupported_content, prompt_stop_reason,
        turn_end_to_stop_reason,
    };

    fn llm_failure() -> LlmFailure {
        LlmFailure {
            message: "failed".into(),
            code: "UNKNOWN".into(),
            status: None,
            provider_retry_after_ms: None,
            request_id: None,
        }
    }

    #[test]
    fn turn_end_completed_is_end_turn() {
        assert_eq!(
            turn_end_to_stop_reason(&TurnEndReason::Completed).as_str(),
            "end_turn"
        );
    }

    #[test]
    fn turn_end_max_tokens_codec_is_max_tokens() {
        assert_eq!(
            turn_end_to_stop_reason(&TurnEndReason::MaxTokens).as_str(),
            "max_tokens"
        );
    }

    #[test]
    fn turn_end_aborted_is_end_turn() {
        assert_eq!(
            turn_end_to_stop_reason(&TurnEndReason::Aborted {
                reason: json!({"kind": "user"}),
            })
            .as_str(),
            "end_turn"
        );
    }

    #[test]
    fn turn_end_interrupted_is_cancelled() {
        assert_eq!(
            turn_end_to_stop_reason(&TurnEndReason::Interrupted).as_str(),
            "cancelled"
        );
    }

    #[test]
    fn turn_end_blocked_is_end_turn() {
        assert_eq!(
            turn_end_to_stop_reason(&TurnEndReason::Blocked).as_str(),
            "end_turn"
        );
    }

    #[test]
    fn turn_end_error_codec_is_end_turn() {
        assert_eq!(
            turn_end_to_stop_reason(&TurnEndReason::Error {
                error: llm_failure(),
            })
            .as_str(),
            "end_turn"
        );
    }

    #[test]
    fn prompt_stop_reason_max_tokens_is_end_turn() {
        assert_eq!(
            prompt_stop_reason(Some(&TurnEndReason::MaxTokens)).as_str(),
            "end_turn"
        );
    }

    #[test]
    fn prompt_stop_reason_none_is_cancelled() {
        assert_eq!(prompt_stop_reason(None).as_str(), "cancelled");
    }

    #[test]
    fn concatenates_text_blocks_verbatim() {
        let prompt = [
            AcpContentBlock::Text {
                text: "first".into(),
            },
            AcpContentBlock::Text {
                text: " second".into(),
            },
        ];
        assert_eq!(acp_prompt_to_text(&prompt), "first second");
    }

    #[test]
    fn renders_resource_link_as_bracketed_json_strings() {
        let name = "notes.md";
        let uri = "file:///tmp/notes.md";
        let prompt = [AcpContentBlock::ResourceLink {
            name: name.into(),
            uri: uri.into(),
        }];
        let expected = "\n[resource_link name=".to_string()
            + &serde_json::to_string(name).unwrap()
            + " uri="
            + &serde_json::to_string(uri).unwrap()
            + "]\n";
        assert_eq!(acp_prompt_to_text(&prompt), expected);
    }

    #[test]
    fn unsupported_prompt_blocks_flatten_to_empty() {
        let block: AcpContentBlock = serde_json::from_value(json!({
            "type": "image",
            "data": "",
            "mimeType": "image/png",
        }))
        .unwrap();
        assert_eq!(acp_prompt_to_text(&[block]), "");
    }

    #[test]
    fn prompt_has_unsupported_content_is_true_for_image() {
        let block: AcpContentBlock = serde_json::from_value(json!({
            "type": "image",
            "data": "",
            "mimeType": "image/png",
        }))
        .unwrap();
        assert!(prompt_has_unsupported_content(&[block]));
    }
}
