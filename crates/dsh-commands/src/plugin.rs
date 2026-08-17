//! Kernel plugin `@deepseek-ai/dsh-commands`.

use std::sync::Arc;

use dsh_boot::PluginSetup;
use dsh_kernel::KernelError;
use serde_json::Value;

use crate::CommandRegistry;

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Register the commands plugin on `registry`.
pub fn register(registry: &mut dsh_boot::PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            reject_unknown_config(&config).map_err(setup_err)?;
            ctx.provide("commands", CommandRegistry::new())
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(dsh_boot::PLUGIN_COMMANDS, setup);
}

const CONFIG_KEYS: &[&str] = &[];

fn reject_unknown_config(config: &Value) -> Result<(), String> {
    match config {
        Value::Null => Ok(()),
        Value::Object(map) => {
            for key in map.keys() {
                if !CONFIG_KEYS.contains(&key.as_str()) {
                    return Err(format!("CommandsConfig: unknown key \"{key}\""));
                }
            }
            Ok(())
        }
        _ => Err("CommandsConfig: config must be an object".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::register;
    use crate::CommandRegistry;
    use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
    use dsh_kernel::Context;

    #[tokio::test]
    async fn provides_empty_commands_registry() {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register(&mut registry);
        boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-commands'\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .expect("boot commands");
        let service = ctx
            .inject::<CommandRegistry>("commands")
            .await
            .expect("commands");
        assert!(service.list().is_empty());
    }

    #[test]
    fn persist_config_rejects_unknown_key() {
        let err = super::reject_unknown_config(&serde_json::json!({"path": "/x"})).unwrap_err();
        assert!(err.contains("unknown key"));
    }

    #[test]
    fn empty_object_config_is_ok() {
        super::reject_unknown_config(&serde_json::json!({})).unwrap();
    }

    #[tokio::test]
    async fn unknown_config_key_fails_load() {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register(&mut registry);
        let err = boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-commands'\n  config:\n    path: /x\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .map(|_| ())
        .unwrap_err();
        assert!(err.to_string().contains("unknown key"), "{err}");
    }
}
