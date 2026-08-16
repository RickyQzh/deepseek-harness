//! Kernel plugin `@deepseek-ai/dsh-token-meter`.

use std::sync::Arc;

use dsh_boot::{PLUGIN_TOKEN_METER, PluginRegistry, PluginSetup};
use dsh_kernel::KernelError;
use serde_json::Value;

use crate::TokenMeter;

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

fn reject_unknown_config_keys(config: &Value) -> Result<(), KernelError> {
    match config {
        Value::Null => Ok(()),
        Value::Object(map) => match map.keys().next() {
            Some(key) => Err(setup_err(format!(
                "TokenMeterConfig: unknown key \"{key}\" (no settings are supported)"
            ))),
            None => Ok(()),
        },
        Value::Array(_) | Value::Bool(_) | Value::Number(_) | Value::String(_) => Err(setup_err(
            "TokenMeterConfig: unknown key (config must be an empty object)".to_string(),
        )),
    }
}

/// Provide `tokenMeter` from YAML config. Config must be omitted or empty.
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            reject_unknown_config_keys(&config)?;
            ctx.provide("tokenMeter", TokenMeter::new())
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_TOKEN_METER, setup);
}

#[cfg(test)]
mod tests {
    use super::register;
    use crate::TokenMeter;
    use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
    use dsh_kernel::Context;

    #[tokio::test]
    async fn unknown_config_key_fails_plugin_load() {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register(&mut registry);
        let err = boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-token-meter'\n  config:\n    density: 3\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .map(|_| ())
        .unwrap_err();
        assert!(err.to_string().contains("unknown key"));
    }

    #[tokio::test]
    async fn empty_config_provides_token_meter() {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register(&mut registry);
        boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-token-meter'\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .unwrap();
        assert!(ctx.get::<TokenMeter>("tokenMeter").is_some());
    }
}
