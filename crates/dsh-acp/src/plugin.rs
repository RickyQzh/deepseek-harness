//! Kernel plugin `@deepseek-ai/dsh-acp`.

use std::sync::Arc;

use dsh_boot::PluginSetup;
use dsh_kernel::KernelError;
use serde_json::Value;

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Kernel service name the CLI reads after `boot_yaml`.
pub const ACP_SERVER_SERVICE: &str = "acpServer";

/// Register the ACP plugin on `registry`.
pub fn register(registry: &mut dsh_boot::PluginRegistry) {
    let setup: PluginSetup = Arc::new(|_ctx, config: Value| {
        Box::pin(async move {
            reject_unknown_config(&config).map_err(setup_err)?;
            Ok(())
        })
    });
    registry.register(dsh_boot::PLUGIN_ACP, setup);
}

/// Register ACP plugins by calling [`register`].
pub fn register_acp_plugins(registry: &mut dsh_boot::PluginRegistry) {
    register(registry);
}

const CONFIG_KEYS: &[&str] = &[];

fn reject_unknown_config(config: &Value) -> Result<(), String> {
    match config {
        Value::Null => Ok(()),
        Value::Object(map) => {
            for key in map.keys() {
                if !CONFIG_KEYS.contains(&key.as_str()) {
                    return Err(format!("AcpConfig: unknown key \"{key}\""));
                }
            }
            Ok(())
        }
        _ => Err("AcpConfig: config must be an object".into()),
    }
}
