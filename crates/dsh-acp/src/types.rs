//! ACP handshake, `session/new`, `session/prompt`, `session/cancel`, `session/update`, and `session/request_permission` wire types. [`InitializeResult`] field order is `protocolVersion`, `agentInfo`, `agentCapabilities`, `authMethods`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::codec::{AcpContentBlock, StopReason};

/// ACP protocol version this server advertises. Client `protocolVersion` is ignored.
pub const PROTOCOL_VERSION: u32 = 1;
/// `agentInfo.name` on `initialize`.
pub const AGENT_INFO_NAME: &str = "deepseek-harness-acp";
/// `agentInfo.version` on `initialize`.
pub const AGENT_INFO_VERSION: &str = "0.0.1";

/// Inbound `initialize` params. Unknown keys are ignored.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeRequest {
    #[serde(default)]
    #[allow(dead_code)]
    protocol_version: Option<u32>,
}

/// Inbound `authenticate` params. Unknown keys are ignored.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticateRequest {
    #[serde(default)]
    #[allow(dead_code)]
    method_id: Option<String>,
}

/// Outbound `initialize` result matching the handshake advertisement.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    protocol_version: u32,
    agent_info: AgentInfo,
    agent_capabilities: AgentCapabilities,
    auth_methods: Vec<AuthMethod>,
}

#[derive(Serialize)]
struct AgentInfo {
    name: &'static str,
    version: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentCapabilities {
    prompt_capabilities: PromptCapabilities,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PromptCapabilities {
    image: bool,
    audio: bool,
    embedded_context: bool,
}

#[derive(Serialize)]
struct AuthMethod {
    id: String,
    name: String,
}

impl InitializeResult {
    /// Locked automation-only advertisement: protocol `1`, empty auth methods, all prompt capabilities false.
    #[must_use]
    pub fn automation_only() -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            agent_info: AgentInfo {
                name: AGENT_INFO_NAME,
                version: AGENT_INFO_VERSION,
            },
            agent_capabilities: AgentCapabilities {
                prompt_capabilities: PromptCapabilities {
                    image: false,
                    audio: false,
                    embedded_context: false,
                },
            },
            auth_methods: Vec::new(),
        }
    }
}

/// Inbound `session/new` params. Unknown keys are ignored.
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NewSessionRequest {
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    additional_directories: Option<Vec<Value>>,
    #[serde(default)]
    mcp_servers: Option<Vec<Value>>,
}

impl NewSessionRequest {
    /// Primary workspace path, or `""` when `cwd` is missing.
    pub(crate) fn cwd(&self) -> &str {
        match &self.cwd {
            Some(cwd) => cwd,
            None => "",
        }
    }

    /// True when `additionalDirectories` is present and non-empty.
    pub(crate) fn additional_directories_unsupported(&self) -> bool {
        match &self.additional_directories {
            Some(dirs) => !dirs.is_empty(),
            None => false,
        }
    }

    /// True when `mcpServers` is present and non-empty. Missing treats as empty.
    pub(crate) fn mcp_servers_unsupported(&self) -> bool {
        match &self.mcp_servers {
            Some(servers) => !servers.is_empty(),
            None => false,
        }
    }
}

/// Outbound `session/new` result. Field order is `sessionId`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NewSessionResult {
    session_id: String,
}

impl NewSessionResult {
    /// Wrap a minted ACP session id for the JSON-RPC result.
    #[must_use]
    pub(crate) fn new(session_id: String) -> Self {
        Self { session_id }
    }
}

/// Inbound `session/prompt` params. Unknown keys are ignored.
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptRequest {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    prompt: Vec<AcpContentBlock>,
}

impl PromptRequest {
    /// Session id to prompt, or `""` when missing.
    pub(crate) fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Prompt content blocks in wire order.
    pub(crate) fn prompt(&self) -> &[AcpContentBlock] {
        &self.prompt
    }
}

/// Outbound `session/prompt` result. Field order is `stopReason`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptResult {
    stop_reason: StopReason,
}

impl PromptResult {
    /// Wrap the settled ACP stop reason.
    #[must_use]
    pub(crate) fn new(stop_reason: StopReason) -> Self {
        Self { stop_reason }
    }
}

/// Inbound `session/cancel` params. Unknown keys are ignored.
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CancelRequest {
    #[serde(default)]
    session_id: String,
}

impl CancelRequest {
    /// Session id to cancel, or `""` when missing.
    pub(crate) fn session_id(&self) -> &str {
        &self.session_id
    }
}

/// Outbound `session/update` params. Field order is `sessionId`, `update`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionUpdateParams {
    session_id: String,
    update: AgentMessageChunk,
}

/// `agent_message_chunk` payload. Field order is `sessionUpdate`, `content`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentMessageChunk {
    session_update: &'static str,
    content: TextContent,
}

/// Text content block. Field order is `type`, `text`.
#[derive(Serialize)]
struct TextContent {
    #[serde(rename = "type")]
    kind: &'static str,
    text: String,
}

impl SessionUpdateParams {
    /// One committed assistant text chunk for `session/update`.
    #[must_use]
    pub(crate) fn agent_message_chunk(session_id: String, text: String) -> Self {
        Self {
            session_id,
            update: AgentMessageChunk {
                session_update: "agent_message_chunk",
                content: TextContent { kind: "text", text },
            },
        }
    }
}

/// Outbound `session/request_permission` params. Field order is `sessionId`, `toolCall`, `options`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RequestPermissionParams {
    session_id: String,
    tool_call: PermissionToolCall,
    options: Vec<PermissionOption>,
}

/// `toolCall` payload. Field order is `toolCallId`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PermissionToolCall {
    tool_call_id: String,
}

/// One permission choice. Field order is `optionId`, `name`, `kind`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PermissionOption {
    option_id: String,
    name: String,
    kind: String,
}

impl RequestPermissionParams {
    /// One-shot `allow-once` / `reject-once` choices for `tool_call_id` on `session_id`.
    #[must_use]
    pub(crate) fn new(session_id: impl Into<String>, tool_call_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            tool_call: PermissionToolCall {
                tool_call_id: tool_call_id.into(),
            },
            options: vec![
                PermissionOption {
                    option_id: "allow-once".into(),
                    name: "Allow once".into(),
                    kind: "allow_once".into(),
                },
                PermissionOption {
                    option_id: "reject-once".into(),
                    name: "Reject".into(),
                    kind: "reject_once".into(),
                },
            ],
        }
    }
}
