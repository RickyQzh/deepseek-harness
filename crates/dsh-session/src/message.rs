//! Model-visible message and stream types stored in the session log.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{CallId, MessageId};

/// Plain text visible to the end user or the model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextBlock {
    /// Block discriminant.
    #[serde(rename = "type")]
    pub kind: TextBlockType,
    /// Exact text.
    pub text: String,
}

/// Discriminant for [`TextBlock`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TextBlockType {
    /// Visible text.
    Text,
}

/// Reasoning / thinking content, distinct from visible text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReasoningBlock {
    /// Block discriminant.
    #[serde(rename = "type")]
    pub kind: ReasoningBlockType,
    /// Exact reasoning text.
    pub text: String,
}

/// Discriminant for [`ReasoningBlock`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReasoningBlockType {
    /// Reasoning text.
    Reasoning,
}

/// A tool invocation requested by the model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallBlock {
    /// Block discriminant.
    #[serde(rename = "type")]
    pub kind: ToolCallBlockType,
    /// Provider-issued call id.
    pub id: CallId,
    /// Tool name.
    pub name: String,
    /// Raw JSON string as produced by the model.
    pub arguments: String,
}

/// Discriminant for [`ToolCallBlock`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolCallBlockType {
    /// Tool call.
    #[serde(rename = "tool-call")]
    ToolCall,
}

/// The result of a tool invocation, sent back to the model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolResultBlock {
    /// Block discriminant.
    #[serde(rename = "type")]
    pub kind: ToolResultBlockType,
    /// Correlated call id.
    #[serde(rename = "toolCallId")]
    pub tool_call_id: CallId,
    /// Nested content blocks.
    pub content: Vec<ContentBlock>,
    /// Whether the tool reported failure.
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "isError")]
    pub is_error: Option<bool>,
}

/// Discriminant for [`ToolResultBlock`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolResultBlockType {
    /// Tool result.
    #[serde(rename = "tool-result")]
    ToolResult,
}

/// Image block; attachment bytes stay opaque JSON in this phase.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageBlock {
    /// Block discriminant.
    #[serde(rename = "type")]
    pub kind: ImageBlockType,
    /// Attachment reference as stored in the log.
    pub attachment: Value,
}

/// Discriminant for [`ImageBlock`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ImageBlockType {
    /// Image.
    Image,
}

/// One model-facing content block.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ContentBlock {
    /// Visible text.
    Text {
        /// Exact text.
        text: String,
    },
    /// Reasoning text.
    Reasoning {
        /// Exact text.
        text: String,
    },
    /// Image attachment.
    Image {
        /// Opaque attachment reference.
        attachment: Value,
    },
    /// Model tool request.
    #[serde(rename = "tool-call")]
    ToolCall {
        /// Call id.
        id: CallId,
        /// Tool name.
        name: String,
        /// Raw arguments JSON string.
        arguments: String,
    },
    /// Tool outcome.
    #[serde(rename = "tool-result")]
    ToolResult {
        /// Correlated call id.
        #[serde(rename = "toolCallId")]
        tool_call_id: CallId,
        /// Nested content.
        content: Vec<ContentBlock>,
        /// Failure flag.
        #[serde(default, skip_serializing_if = "Option::is_none", rename = "isError")]
        is_error: Option<bool>,
    },
}

/// Who produced a message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum MessageSource {
    /// Direct human prompt.
    User,
    /// Plugin-injected context.
    Plugin {
        /// Plugin package name.
        plugin: String,
        /// Optional semantic form.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        form: Option<String>,
        /// Snapshot sections when `form` is `snapshot`.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        sections: Vec<ContextSnapshotSection>,
        /// Notice summary when `form` is `notice`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
        /// Compaction transaction id when this plugin message is a checkpoint.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            rename = "compactionId"
        )]
        compaction_id: Option<String>,
        /// Initiating command id when this checkpoint was manual.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            rename = "sourceCommandId"
        )]
        source_command_id: Option<String>,
    },
    /// Model output.
    Model {
        /// Provider route.
        provider: String,
        /// Model id.
        model: String,
        /// Adapter-private replay state.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            rename = "replayState"
        )]
        replay_state: Option<Value>,
    },
    /// Tool result source.
    Tool {
        /// Correlated call id.
        #[serde(rename = "callId")]
        call_id: CallId,
    },
    /// Workspace instruction baseline or later instruction delta.
    AgentInstructions {
        /// Context form; always `instructions`.
        form: String,
        /// Marks the complete startup/resume baseline rather than a later delta.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        baseline: Option<bool>,
        /// Discovery, precedence, budget, and loaded-file identity for resume checks.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            rename = "baselineIdentity"
        )]
        baseline_identity: Option<String>,
        /// Structured instruction transitions represented by this message.
        #[serde(default)]
        changes: Vec<Value>,
    },
    /// Durable model-facing skill catalog published for a session.
    SkillCatalog {
        /// Context form; always `catalog`.
        form: String,
        /// Marks a replacement catalog rather than this session's first publication.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        update: Option<bool>,
        /// Exactly the entries this message published, in catalog order.
        entries: Vec<SkillCatalogEntry>,
    },
    /// A user-explicit skill invocation injected by the host.
    SkillInvocation {
        /// Invoked skill name, validated user-invocable at the injecting boundary.
        name: String,
        /// Injected skill bodies are instructions for the model to follow.
        form: String,
    },
}

/// One name/description pair published in a [`MessageSource::SkillCatalog`] message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillCatalogEntry {
    /// Kebab-case skill name.
    pub name: String,
    /// Normalized, length-capped description as published (unescaped).
    pub description: String,
}

/// One named contribution to a snapshot-form context.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextSnapshotSection {
    /// Contributing subsystem name.
    pub name: String,
    /// Model-facing text.
    pub text: String,
}

/// Conversation role.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MessageRole {
    /// System slot (rare in the log; reserved).
    System,
    /// User-role message.
    User,
    /// Assistant-role message.
    Assistant,
}

/// One immutable message shared by delivery, durable history, and model requests.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    /// Stable identity.
    pub id: MessageId,
    /// Provider-neutral role.
    pub role: MessageRole,
    /// Exact model-facing blocks.
    pub content: Vec<ContentBlock>,
    /// Producer source.
    pub source: MessageSource,
}

/// Token accounting for one model call. Counts are disjoint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TokenUsage {
    /// Uncached input tokens.
    #[serde(rename = "inputTokens")]
    pub input_tokens: u64,
    /// Output tokens.
    #[serde(rename = "outputTokens")]
    pub output_tokens: u64,
    /// Cached input read.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "cacheReadTokens"
    )]
    pub cache_read_tokens: Option<u64>,
    /// Cached input write.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "cacheWriteTokens"
    )]
    pub cache_write_tokens: Option<u64>,
    /// Reasoning tokens.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "reasoningTokens"
    )]
    pub reasoning_tokens: Option<u64>,
}

/// Provider, model, reasoning effort, and sampling scalars.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LlmCallConfig {
    /// Provider route.
    pub provider: String,
    /// Model id.
    pub model: String,
    /// Reasoning effort id (string in v0 logs: `high`, `off`, …).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "reasoningEffort"
    )]
    pub reasoning_effort: Option<String>,
    /// Sampling temperature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    /// Output token cap.
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "maxTokens")]
    pub max_tokens: Option<u64>,
    /// Stop sequences.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
}

/// Adapter-default markers on a logged header.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LlmCallConfigAdapterDefaults {
    /// `true` when reasoning effort came from the adapter.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "reasoningEffort"
    )]
    pub reasoning_effort: Option<bool>,
    /// `true` when max tokens came from the adapter.
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "maxTokens")]
    pub max_tokens: Option<bool>,
}

/// Logged request state outside derived history.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EpochHeader {
    /// Call configuration.
    pub config: LlmCallConfig,
    /// Adapter-default markers.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "adapterDefaults"
    )]
    pub adapter_defaults: Option<LlmCallConfigAdapterDefaults>,
    /// Rendered system prompt, or a snapshot token such as `{{system}}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    /// Tool schemas array, or the snapshot token string `{{tools}}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Value>,
}

/// Why a `request/header` snapshot was appended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RequestHeaderReason {
    /// First header in the log.
    Initial,
    /// First request after resume/fork.
    Resume,
    /// Later request used a different header.
    Change,
}

/// Route metadata for the next request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RequestContext {
    /// Provider route.
    pub provider: String,
    /// Model id.
    pub model: String,
    /// Advertised context window.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "contextWindow"
    )]
    pub context_window: Option<u64>,
}

/// Why a model response stopped.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum FinishReason {
    /// Natural stop.
    Stop,
    /// Model requested tools.
    #[serde(rename = "tool-calls")]
    ToolCalls,
    /// Output token ceiling.
    #[serde(rename = "max-tokens")]
    MaxTokens,
    /// Aborted with failure facts.
    Aborted {
        /// Failure facts.
        failure: LlmFailure,
    },
    /// Error with failure facts.
    Error {
        /// Failure facts.
        failure: LlmFailure,
    },
}

/// Serializable provider or transport failure facts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LlmFailure {
    /// Human-readable failure.
    pub message: String,
    /// Stable machine-routing code.
    pub code: String,
    /// HTTP status when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// Provider-requested delay.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "providerRetryAfterMs"
    )]
    pub provider_retry_after_ms: Option<u64>,
    /// Provider request id.
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "requestId")]
    pub request_id: Option<String>,
}

/// Raw streaming protocol emitted by adapters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum StreamChunk {
    /// Opens a stream block.
    #[serde(rename = "block-start")]
    BlockStart {
        /// Block index.
        index: u32,
        /// Block type name (`text`, `tool-call`, …).
        #[serde(rename = "blockType")]
        block_type: String,
    },
    /// Visible text delta.
    #[serde(rename = "text-delta")]
    TextDelta {
        /// Block index.
        index: u32,
        /// Delta text.
        text: String,
    },
    /// Reasoning delta.
    #[serde(rename = "reasoning-delta")]
    ReasoningDelta {
        /// Block index.
        index: u32,
        /// Delta text.
        text: String,
    },
    /// Tool-call arguments delta.
    #[serde(rename = "tool-call-delta")]
    ToolCallDelta {
        /// Block index.
        index: u32,
        /// Call id.
        id: CallId,
        /// Tool name when present on every member of a packable run.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        /// Raw arguments fragment.
        #[serde(rename = "argumentsDelta")]
        arguments_delta: String,
    },
    /// Closes a stream block with the assembled block.
    #[serde(rename = "block-end")]
    BlockEnd {
        /// Block index.
        index: u32,
        /// Assembled block.
        block: ContentBlock,
    },
    /// Token usage.
    Usage {
        /// Usage payload.
        usage: TokenUsage,
    },
    /// Terminal finish.
    Finish {
        /// Stop reason.
        reason: FinishReason,
        /// Adapter-private replay state.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            rename = "replayState"
        )]
        replay_state: Option<Value>,
    },
}

/// Why a turn ended.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum TurnEndReason {
    /// Success.
    Completed,
    /// Cancellation.
    Aborted {
        /// Cancellation cause.
        reason: Value,
    },
    /// Blocked.
    Blocked,
    /// Failed.
    Error {
        /// Structured failure.
        error: LlmFailure,
    },
    /// Output token ceiling.
    #[serde(rename = "max-tokens")]
    MaxTokens,
    /// Crash-orphaned turn closed on reload.
    Interrupted,
}

/// Why a `request/header` / inbox splice / title exists — inbox target.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InboxTarget {
    /// Claimed at the next turn.
    #[serde(rename = "next-turn")]
    NextTurn,
    /// Claimed at the next step.
    #[serde(rename = "next-step")]
    NextStep,
}

/// Durable inbox mutation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InboxSplicedData {
    /// Which pending list.
    pub target: InboxTarget,
    /// Splice start index.
    pub start: u64,
    /// Removed count when present.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "removedCount"
    )]
    pub removed_count: Option<u64>,
    /// Inserted user-role messages.
    pub inserted: Vec<Message>,
    /// Optional canceled outcome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
}

/// `turn/start` payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnStartData {
    /// Turn number.
    pub turn: u64,
}

/// `turn/end` payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TurnEndData {
    /// Turn number.
    pub turn: u64,
    /// End reason.
    pub reason: TurnEndReason,
}

/// `step/start` / `step/end` payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepBoundaryData {
    /// Turn number.
    pub turn: u64,
    /// Step number.
    pub step: u64,
}

/// `assistant/chunk` payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AssistantChunkData {
    /// Turn number.
    pub turn: u64,
    /// Step number.
    pub step: u64,
    /// Stream chunk.
    pub chunk: StreamChunk,
}

/// `assistant/message` payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AssistantMessageData {
    /// Turn number.
    pub turn: u64,
    /// Step number.
    pub step: u64,
    /// Assembled assistant message.
    pub message: Message,
    /// Usage when the adapter reported it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
}

/// `tool/call` payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallData {
    /// Turn number.
    pub turn: u64,
    /// Step number.
    pub step: u64,
    /// Call id.
    #[serde(rename = "callId")]
    pub call_id: CallId,
    /// Tool name.
    pub name: String,
    /// Raw arguments JSON string.
    pub arguments: String,
}

/// `tool/result` payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolResultData {
    /// Turn number.
    pub turn: u64,
    /// Step number.
    pub step: u64,
    /// Tool-result message.
    pub message: Message,
    /// Internal failure identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ToolResultError>,
    /// Tool-private presentation payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

/// Internal tool-result failure identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResultError {
    /// Error class name.
    pub name: String,
    /// Stable code.
    pub code: String,
}

/// `request/header` payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RequestHeaderData {
    /// Full header snapshot.
    pub header: EpochHeader,
    /// Why it was appended.
    pub reason: RequestHeaderReason,
}

/// `session/title` payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionTitleData {
    /// Normalized title text.
    pub title: String,
    /// Human `user/message` seqs used to derive it.
    #[serde(rename = "messageSeqs")]
    pub message_seqs: Vec<u64>,
    /// Title source.
    pub source: Value,
}

/// `permission/preset` payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionPresetData {
    /// Preset name.
    pub preset: String,
}

/// `sandbox/mode` payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxModeData {
    /// Mode name.
    pub mode: String,
    /// Optional source (`delegation`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// `approval/policy` payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalPolicyData {
    /// Policy name.
    pub policy: String,
    /// Optional source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::MessageSource;
    use serde_json::json;

    #[test]
    fn plugin_source_omits_absent_compaction_fields() {
        let source = MessageSource::Plugin {
            plugin: "runtime-context".into(),
            form: None,
            sections: Vec::new(),
            summary: None,
            compaction_id: None,
            source_command_id: None,
        };
        let value = serde_json::to_value(&source).expect("serialize");
        assert_eq!(value["kind"], "plugin");
        assert_eq!(value["plugin"], "runtime-context");
        assert!(value.get("compactionId").is_none());
        assert!(value.get("sourceCommandId").is_none());
    }

    #[test]
    fn plugin_source_decodes_without_compaction_fields() {
        let source: MessageSource = serde_json::from_value(json!({
            "kind": "plugin",
            "plugin": "runtime-context",
        }))
        .expect("deserialize");
        match source {
            MessageSource::Plugin {
                plugin,
                compaction_id,
                source_command_id,
                ..
            } => {
                assert_eq!(plugin, "runtime-context");
                assert!(compaction_id.is_none());
                assert!(source_command_id.is_none());
            }
            other => panic!("expected plugin source, got {other:?}"),
        }
    }

    #[test]
    fn plugin_source_round_trips_compaction_fields() {
        let source = MessageSource::Plugin {
            plugin: "compact".into(),
            form: None,
            sections: Vec::new(),
            summary: None,
            compaction_id: Some("cmp-1".into()),
            source_command_id: Some("cmd-9".into()),
        };
        let value = serde_json::to_value(&source).expect("serialize");
        assert_eq!(value["compactionId"], "cmp-1");
        assert_eq!(value["sourceCommandId"], "cmd-9");
        let back: MessageSource = serde_json::from_value(value).expect("deserialize");
        match back {
            MessageSource::Plugin {
                compaction_id,
                source_command_id,
                ..
            } => {
                assert_eq!(compaction_id.as_deref(), Some("cmp-1"));
                assert_eq!(source_command_id.as_deref(), Some("cmd-9"));
            }
            other => panic!("expected plugin source, got {other:?}"),
        }
    }

    #[test]
    fn agent_instructions_source_uses_kebab_kind_and_camel_identity() {
        let source = MessageSource::AgentInstructions {
            form: "instructions".into(),
            baseline: Some(true),
            baseline_identity: Some("ident-1".into()),
            changes: vec![json!({"action":"set","path":"AGENTS.md"})],
        };
        let value = serde_json::to_value(&source).expect("serialize");
        assert_eq!(value["kind"], "agent-instructions");
        assert_eq!(value["form"], "instructions");
        assert_eq!(value["baseline"], true);
        assert_eq!(value["baselineIdentity"], "ident-1");
        let back: MessageSource = serde_json::from_value(value).expect("deserialize");
        match back {
            MessageSource::AgentInstructions {
                form,
                baseline,
                baseline_identity,
                changes,
            } => {
                assert_eq!(form, "instructions");
                assert_eq!(baseline, Some(true));
                assert_eq!(baseline_identity.as_deref(), Some("ident-1"));
                assert_eq!(changes.len(), 1);
            }
            other => panic!("expected agent-instructions source, got {other:?}"),
        }
    }

    #[test]
    fn skill_catalog_source_uses_kebab_kind_and_optional_update() {
        let source = MessageSource::SkillCatalog {
            form: "catalog".into(),
            update: None,
            entries: vec![super::SkillCatalogEntry {
                name: "demo-skill".into(),
                description: "Demo.".into(),
            }],
        };
        let value = serde_json::to_value(&source).expect("serialize");
        assert_eq!(value["kind"], "skill-catalog");
        assert_eq!(value["form"], "catalog");
        assert!(value.get("update").is_none());
        assert_eq!(value["entries"][0]["name"], "demo-skill");
        let update = MessageSource::SkillCatalog {
            form: "catalog".into(),
            update: Some(true),
            entries: vec![],
        };
        let update_value = serde_json::to_value(&update).expect("serialize");
        assert_eq!(update_value["update"], true);
        let back: MessageSource = serde_json::from_value(value).expect("deserialize");
        match back {
            MessageSource::SkillCatalog {
                form,
                update,
                entries,
            } => {
                assert_eq!(form, "catalog");
                assert!(update.is_none());
                assert_eq!(entries.len(), 1);
            }
            other => panic!("expected skill-catalog source, got {other:?}"),
        }
    }

    #[test]
    fn skill_invocation_source_uses_kebab_kind() {
        let source = MessageSource::SkillInvocation {
            name: "demo-skill".into(),
            form: "instructions".into(),
        };
        let value = serde_json::to_value(&source).expect("serialize");
        assert_eq!(value["kind"], "skill-invocation");
        assert_eq!(value["name"], "demo-skill");
        assert_eq!(value["form"], "instructions");
    }
}
