//! MCP client for the DeepSeek Harness Rust host.

mod name;
mod result;

use std::sync::Arc;

use dsh_boot::PluginSetup;
use dsh_kernel::KernelError;
use serde_json::Value;

pub use name::public_tool_name;
pub use result::extract_text;

/// Register YAML `@deepseek-ai/dsh-mcp-client`. Setup returns `Ok(())` without connecting.
pub fn register(registry: &mut dsh_boot::PluginRegistry) {
    let setup: PluginSetup =
        Arc::new(|_ctx, _config: Value| Box::pin(std::future::ready(Ok::<(), KernelError>(()))));
    registry.register(dsh_boot::PLUGIN_MCP_CLIENT, setup);
}

/// Register MCP client plugins by calling [`register`].
pub fn register_mcp_plugins(registry: &mut dsh_boot::PluginRegistry) {
    register(registry);
}

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
