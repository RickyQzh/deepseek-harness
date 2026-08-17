//! ACP stdio adapter for the DeepSeek Harness Rust host.

mod plugin;

pub use plugin::{ACP_SERVER_SERVICE, register, register_acp_plugins};

#[cfg(test)]
mod tests {
    #[test]
    fn plugin_name_is_typescript_package_name() {
        assert_eq!(dsh_boot::PLUGIN_ACP, "@deepseek-ai/dsh-acp");
    }
}
