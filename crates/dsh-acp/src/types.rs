//! ACP handshake and `session/new` wire types. [`InitializeResult`] field order is `protocolVersion`, `agentInfo`, `agentCapabilities`, `authMethods`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
