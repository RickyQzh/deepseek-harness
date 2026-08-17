//! ACP handshake wire types. [`InitializeResult`] field order is `protocolVersion`, `agentInfo`, `agentCapabilities`, `authMethods`.

use serde::{Deserialize, Serialize};

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
