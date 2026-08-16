//! Kernel plugin `@deepseek-ai/dsh-web`.

use std::sync::Arc;

use dsh_boot::{PLUGIN_WEB, PluginRegistry, PluginSetup};
use dsh_kernel::KernelError;
use serde_json::Value;

use crate::WebRuntime;

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Provide `web` as [`WebRuntime`]. Fetch registration is not part of this plugin.
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            let search_provider_id =
                resolve_config(&config).map_err(|error| setup_err(error.to_string()))?;
            ctx.provide("web", WebRuntime::new(search_provider_id))
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_WEB, setup);
}

const CONFIG_KEYS: &[&str] = &["searchProvider"];

fn resolve_config(value: &Value) -> Result<Option<String>, String> {
    match value {
        Value::Null => Ok(None),
        Value::Object(map) => {
            for key in map.keys() {
                if !CONFIG_KEYS.contains(&key.as_str()) {
                    return Err(format!("WebConfig: unknown key \"{key}\""));
                }
            }
            match map.get("searchProvider") {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(id)) => Ok(Some(id.clone())),
                Some(_) => Err("WebConfig.searchProvider must be a string".into()),
            }
        }
        _ => Err("WebConfig: config must be an object".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::register;
    use crate::WebRuntime;
    use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
    use dsh_kernel::Context;

    #[tokio::test]
    async fn provides_web_runtime() {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register(&mut registry);
        boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-web'\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .expect("boot web");
        ctx.inject::<WebRuntime>("web").await.expect("web");
    }

    #[tokio::test]
    async fn unknown_config_key_fails_plugin_load() {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register(&mut registry);
        let err = boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-web'\n  config:\n    fetchProvider: http\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .map(|_| ())
        .unwrap_err();
        assert!(err.to_string().contains("unknown key"));
    }
}
