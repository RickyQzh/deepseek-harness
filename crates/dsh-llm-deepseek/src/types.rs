//! DeepSeek chat-completions wire types.

use serde::{Deserialize, Serialize};

/// Thinking-mode toggle sent as `thinking.type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingMode {
    /// Provider thinking is on.
    Enabled,
    /// Provider thinking is off.
    Disabled,
}

/// Wire `thinking` object: `{ "type": "enabled" | "disabled" }`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ThinkingField {
    /// `enabled` or `disabled`.
    #[serde(rename = "type")]
    pub kind: ThinkingMode,
}

/// Always-on stream options for chat completions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StreamOptions {
    /// Request a usage object on the stream.
    pub include_usage: bool,
}

/// Function-call payload on a history or tools entry.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WireFunction {
    /// Tool name.
    pub name: String,
    /// Raw JSON arguments string.
    pub arguments: String,
}

/// Completed tool call replayed on an assistant history message.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WireToolCall {
    /// Provider-issued call id.
    pub id: String,
    /// Always `"function"`.
    #[serde(rename = "type")]
    pub kind: &'static str,
    /// Name and arguments.
    pub function: WireFunction,
}

/// JSON Schema wrapper for one advertised tool.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WireToolFunction {
    /// Tool name.
    pub name: String,
    /// Tool description.
    pub description: String,
    /// JSON Schema parameters object.
    pub parameters: serde_json::Value,
}

/// One entry of the request `tools` array.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WireTool {
    /// Always `"function"`.
    #[serde(rename = "type")]
    pub kind: &'static str,
    /// Name, description, and parameters.
    pub function: WireToolFunction,
}

/// One chat-completions history message, discriminated on `role`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum WireMessage {
    /// System-role instructions.
    System {
        /// Flattened text.
        content: String,
    },
    /// User-role text.
    User {
        /// Flattened text.
        content: String,
    },
    /// Assistant-role history. `content` is always a JSON string, including `""`.
    Assistant {
        /// Joined text blocks; never JSON `null`.
        content: String,
        /// Thinking-mode passback; only on tool-call turns with non-empty reasoning.
        #[serde(skip_serializing_if = "Option::is_none")]
        reasoning_content: Option<String>,
        /// Completed tool calls from this turn.
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_calls: Option<Vec<WireToolCall>>,
    },
    /// Tool-result message keyed by call id.
    Tool {
        /// Correlated call id.
        tool_call_id: String,
        /// Flattened tool text, or `"(no output)"`.
        content: String,
    },
}

/// Request body for `POST {base_url}/chat/completions`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WireRequest {
    /// Wire model id.
    pub model: String,
    /// Serialized conversation, including an optional system slot.
    pub messages: Vec<WireMessage>,
    /// Always `true`.
    pub stream: bool,
    /// Always `{ include_usage: true }`.
    pub stream_options: StreamOptions,
    /// Omitted when the adapter leaves thinking unspecified.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingField>,
    /// `"high"` or `"max"`; never `"off"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    /// Omitted when no tools are advertised.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<WireTool>>,
    /// Sampling temperature.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    /// Output token cap.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    /// Stop sequences.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
}

/// OpenAI-compat cached-token detail on prompt usage.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct PromptTokensDetails {
    /// Cache-hit count when the provider uses this spelling.
    pub cached_tokens: Option<u64>,
}

/// Reasoning-token detail on completion usage.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct CompletionTokensDetails {
    /// Reasoning tokens when the provider reports them.
    pub reasoning_tokens: Option<u64>,
}

/// Wire token accounting. `prompt_tokens` includes cache hits.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct WireUsage {
    /// Prompt tokens, including cache hits.
    pub prompt_tokens: u64,
    /// Completion tokens.
    pub completion_tokens: u64,
    /// DeepSeek cache-hit spelling.
    pub prompt_cache_hit_tokens: Option<u64>,
    /// OpenAI-compat cache-hit spelling.
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    /// Optional reasoning-token detail.
    pub completion_tokens_details: Option<CompletionTokensDetails>,
}

/// Streamed fragment of one tool call.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct WireToolCallDelta {
    pub(crate) index: u32,
    pub(crate) id: Option<String>,
    pub(crate) function: Option<WireFunctionDelta>,
}

/// Incremental name/arguments on a tool-call delta.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct WireFunctionDelta {
    pub(crate) name: Option<String>,
    pub(crate) arguments: Option<String>,
}

/// Incremental content of one streamed choice.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct WireDelta {
    pub(crate) content: Option<String>,
    pub(crate) reasoning_content: Option<String>,
    pub(crate) tool_calls: Option<Vec<WireToolCallDelta>>,
}

/// One streamed choice.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct WireChoice {
    pub(crate) delta: Option<WireDelta>,
    pub(crate) finish_reason: Option<String>,
}

/// One parsed SSE `data:` payload (`chat.completion.chunk`).
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct WireChunk {
    #[serde(default)]
    pub(crate) choices: Vec<WireChoice>,
    pub(crate) usage: Option<WireUsage>,
}
