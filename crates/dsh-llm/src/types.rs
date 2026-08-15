//! Provider-neutral request and model-metadata types owned by this crate.

use dsh_session::{Message, SessionId};
use dsh_tools::AbortFlag;
use serde::Serialize;

/// JSON Schema description of a tool, as sent to the model.
///
/// Declared here because it is part of [`GenerateOptions`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ToolSchema {
    /// Model-facing tool name.
    pub name: String,
    /// Model-facing tool description.
    pub description: String,
    /// JSON Schema parameters object.
    pub parameters: serde_json::Value,
}

/// Provider-neutral classification for an auxiliary model call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LlmPurpose {
    /// Compaction / summarization call.
    Compaction,
    /// Session-title generation call.
    SessionTitle,
}

/// One fully assembled model request.
#[derive(Clone, Debug)]
pub struct GenerateOptions {
    /// Registered provider route selecting the adapter instance.
    pub provider: String,
    /// Model id passed to the adapter.
    pub model: String,
    /// Adapter-owned reasoning effort for this exact model.
    pub reasoning_effort: Option<String>,
    /// Ordered conversation messages after the system slot.
    pub messages: Vec<Message>,
    /// System prompt text; adapters map this to the provider system slot.
    pub system: Option<String>,
    /// Tool schemas; adapters map this to the provider `tools` field.
    pub tools: Option<Vec<ToolSchema>>,
    /// Sampling temperature.
    pub temperature: Option<f64>,
    /// Output token cap.
    pub max_tokens: Option<u64>,
    /// Stop sequences; the stop string itself is not included in the output.
    pub stop: Option<Vec<String>>,
    /// Cooperative cancellation for this request.
    pub signal: AbortFlag,
    /// Session identity stamped by the loop for request routing.
    pub session_id: Option<SessionId>,
    /// Auxiliary-call classification; ordinary conversation leaves this unset.
    pub purpose: Option<LlmPurpose>,
}

/// Display metadata for one registered provider route.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LlmProviderInfo {
    /// Provider route key used by [`GenerateOptions::provider`].
    pub id: String,
    /// Human-readable provider name for selectors and diagnostics.
    pub name: String,
}

/// Provider-owned context capacity for one exact provider/model route.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LlmModelContext {
    /// Maximum combined request and response context in tokens.
    pub context_window: u64,
}

/// Display metadata for one adapter-owned reasoning effort.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LlmReasoningEffortInfo {
    /// Opaque stable value accepted by [`GenerateOptions::reasoning_effort`].
    pub id: String,
    /// Human-readable effort name for selectors and diagnostics.
    pub name: String,
}

/// Selectable reasoning efforts for one exact provider/model route.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LlmModelReasoningInfo {
    /// Supported efforts in adapter-preferred display order.
    pub efforts: Vec<LlmReasoningEffortInfo>,
    /// Adapter default materialized when callers omit an effort.
    pub default_effort: Option<String>,
}

/// Exact-route model metadata resolved by its owning adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LlmResolvedModelInfo {
    /// Provider route that owns this model.
    pub provider: String,
    /// Model id passed to [`GenerateOptions::model`].
    pub id: String,
    /// Human-readable model name for selectors.
    pub name: String,
    /// Provider-owned context capacity when known.
    pub context: Option<LlmModelContext>,
    /// Adapter-configured per-request output cap materialized when callers omit one.
    pub default_max_tokens: Option<u64>,
    /// Adapter-owned selectable reasoning levels when exposed.
    pub reasoning: Option<LlmModelReasoningInfo>,
}
