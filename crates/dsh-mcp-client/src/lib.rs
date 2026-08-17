//! MCP client for the DeepSeek Harness Rust host.

mod client;
mod name;
mod plugin;
mod result;
mod rpc;
mod stdio;
mod sync;

#[cfg(test)]
mod test_server;

pub use client::{McpSession, McpToolDraft};
pub use name::public_tool_name;
pub use plugin::{register, register_mcp_plugins};
pub use result::extract_text;
pub use rpc::{McpRpcError, encode_frame, read_frame};
pub use stdio::{StdioSpawnError, spawn_stdio, stdio_child_env, stdio_command};
pub use sync::{SyncError, sync_tools};

#[cfg(test)]
mod tests {
    use super::register_mcp_plugins;
    use dsh_boot::PluginRegistry;

    #[test]
    fn plugin_name_is_typescript_package_name() {
        assert_eq!(dsh_boot::PLUGIN_MCP_CLIENT, "@deepseek-ai/dsh-mcp-client");
    }

    #[test]
    fn register_mcp_plugins_installs_yaml_name() {
        let mut registry = PluginRegistry::new();
        register_mcp_plugins(&mut registry);
        assert!(registry.get(dsh_boot::PLUGIN_MCP_CLIENT).is_some());
    }
}
