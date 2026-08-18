//! Kernel plugin `@deepseek-ai/dsh-subagent`.

use std::sync::Arc;

use dsh_boot::{PLUGIN_SUBAGENT, PluginRegistry, PluginSetup};
use dsh_kernel::KernelError;
use serde_json::Value;

use crate::SubagentRuntime;

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Provide `subagents` as [`SubagentRuntime`].
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            reject_unknown_keys(&config, &[], "SubagentConfig")?;
            let runtime = SubagentRuntime::new(ctx.clone());
            ctx.provide("subagents", runtime)
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_SUBAGENT, setup);
}

fn reject_unknown_keys(value: &Value, allowed: &[&str], subject: &str) -> Result<(), KernelError> {
    match value {
        Value::Null => Ok(()),
        Value::Object(map) => {
            for key in map.keys() {
                if !allowed.contains(&key.as_str()) {
                    return Err(setup_err(format!("{subject}: unknown key \"{key}\"")));
                }
            }
            Ok(())
        }
        _ => Err(setup_err(format!("{subject}: config must be an object"))),
    }
}

#[cfg(test)]
mod tests {
    use super::register;
    use crate::SubagentRuntime;
    use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
    use dsh_kernel::Context;

    #[tokio::test]
    async fn provides_subagent_runtime() {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register(&mut registry);
        boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-subagent'\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .expect("boot subagent");
        ctx.inject::<SubagentRuntime>("subagents")
            .await
            .expect("subagents");
    }

    #[tokio::test]
    async fn unknown_config_key_fails_plugin_load() {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register(&mut registry);
        let err = boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-subagent'\n  config:\n    extra: true\n",
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
