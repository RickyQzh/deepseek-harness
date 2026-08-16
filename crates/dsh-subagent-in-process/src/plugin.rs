//! Kernel plugins `@deepseek-ai/dsh-subagent-spawn-in-process` and `@deepseek-ai/dsh-subagent-fork-in-process`.

use std::sync::Arc;

use dsh_boot::{PLUGIN_SUBAGENT_FORK, PLUGIN_SUBAGENT_SPAWN, PluginRegistry, PluginSetup};
use dsh_kernel::KernelError;
use dsh_subagent::SubagentRuntime;
use serde_json::Value;

use crate::fork::ForkInProcessProvider;
use crate::spawn::SpawnInProcessProvider;

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Register the spawn provider on `subagents` (default name `spawn`).
pub fn register_spawn(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            let name = resolve_provider_name(&config, "spawn", "SpawnInProcessConfig")
                .map_err(setup_err)?;
            let subagents = ctx.inject::<SubagentRuntime>("subagents").await?;
            subagents
                .register_provider(Arc::new(SpawnInProcessProvider::new(name, ctx.clone())))
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_SUBAGENT_SPAWN, setup);
}

/// Register the fork provider on `subagents` (default name `fork`).
pub fn register_fork(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            let name =
                resolve_provider_name(&config, "fork", "ForkInProcessConfig").map_err(setup_err)?;
            let subagents = ctx.inject::<SubagentRuntime>("subagents").await?;
            subagents
                .register_provider(Arc::new(ForkInProcessProvider::new(name, ctx.clone())))
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_SUBAGENT_FORK, setup);
}

const CONFIG_KEYS: &[&str] = &["providerName"];

fn resolve_provider_name(value: &Value, default: &str, subject: &str) -> Result<String, String> {
    match value {
        Value::Null => Ok(default.to_string()),
        Value::Object(map) => {
            for key in map.keys() {
                if !CONFIG_KEYS.contains(&key.as_str()) {
                    return Err(format!("{subject}: unknown key \"{key}\""));
                }
            }
            match map.get("providerName") {
                None | Some(Value::Null) => Ok(default.to_string()),
                Some(Value::String(name)) => {
                    if name.is_empty() {
                        Err(format!("{subject}.providerName must be non-empty"))
                    } else {
                        Ok(name.clone())
                    }
                }
                Some(_) => Err(format!("{subject}.providerName must be a string")),
            }
        }
        _ => Err(format!("{subject}: config must be an object")),
    }
}

#[cfg(test)]
mod tests {
    use super::{register_fork, register_spawn};
    use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
    use dsh_kernel::Context;
    use dsh_subagent::{SubagentError, SubagentRuntime};

    #[tokio::test]
    async fn spawn_and_fork_register_default_names() {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        dsh_subagent::register(&mut registry);
        register_spawn(&mut registry);
        register_fork(&mut registry);
        boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-subagent'\n- name: '@deepseek-ai/dsh-subagent-spawn-in-process'\n- name: '@deepseek-ai/dsh-subagent-fork-in-process'\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .expect("boot");
        let rt = ctx
            .inject::<SubagentRuntime>("subagents")
            .await
            .expect("subagents");
        let err = rt
            .register_provider(std::sync::Arc::new(Duplicate("spawn")))
            .unwrap_err();
        assert!(matches!(err, SubagentError::DuplicateProvider(name) if name == "spawn"));
        let err = rt
            .register_provider(std::sync::Arc::new(Duplicate("fork")))
            .unwrap_err();
        assert!(matches!(err, SubagentError::DuplicateProvider(name) if name == "fork"));
    }

    #[tokio::test]
    async fn unknown_config_key_fails_plugin_load() {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        dsh_subagent::register(&mut registry);
        register_spawn(&mut registry);
        let err = boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-subagent'\n- name: '@deepseek-ai/dsh-subagent-spawn-in-process'\n  config:\n    extra: 1\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .map(|_| ())
        .unwrap_err();
        assert!(err.to_string().contains("unknown key"));
    }

    struct Duplicate(&'static str);

    impl dsh_subagent::SubagentProvider for Duplicate {
        fn name(&self) -> &str {
            self.0
        }

        fn capabilities(&self) -> dsh_subagent::SubagentCapabilities {
            dsh_subagent::SubagentCapabilities::all()
        }

        fn inherits_parent_context(&self) -> bool {
            false
        }

        fn start(
            &self,
            _request: dsh_subagent::SubagentStartRequest,
            _parent: dsh_agent::AgentHandle,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<dsh_subagent::SubagentResult, dsh_subagent::SubagentError>,
                    > + Send
                    + '_,
            >,
        > {
            Box::pin(async { Err(dsh_subagent::SubagentError::other("unused")) })
        }

        fn prepare_continuable(
            &self,
            _parent: &dsh_agent::AgentHandle,
        ) -> dsh_subagent::ContinuableCreateSpec {
            dsh_subagent::ContinuableCreateSpec { seed: None }
        }
    }
}
