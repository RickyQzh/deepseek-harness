//! ACP stdio adapter for the DeepSeek Harness Rust host.

mod bridge;
mod codec;
mod error;
mod plugin;
mod rpc;
mod types;

pub use bridge::AcpBridge;
pub use codec::{
    AcpContentBlock, StopReason, acp_prompt_to_text, prompt_has_unsupported_content,
    prompt_stop_reason, turn_end_to_stop_reason,
};
pub use error::{
    AcpError, ERR_INTERNAL, ERR_INVALID_PARAMS, ERR_INVALID_REQUEST, ERR_METHOD_NOT_FOUND,
    ERR_PARSE, internal_error, invalid_params, method_not_found,
};
pub use plugin::{ACP_SERVER_SERVICE, register, register_acp_plugins};
pub use rpc::{
    AcpNdjsonTransport, AcpTransportError, JSONRPC_VERSION, JsonRpcId, NotificationHandler,
    RequestHandler,
};
pub use types::{AGENT_INFO_NAME, AGENT_INFO_VERSION, PROTOCOL_VERSION};

#[cfg(test)]
mod tests {
    #[test]
    fn plugin_name_is_typescript_package_name() {
        assert_eq!(dsh_boot::PLUGIN_ACP, "@deepseek-ai/dsh-acp");
    }
}
