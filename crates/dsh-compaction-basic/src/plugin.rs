//! Kernel plugin `@deepseek-ai/dsh-compaction-basic`.

use std::sync::{Arc, Mutex};

use dsh_boot::{PLUGIN_COMPACTION_BASIC, PluginRegistry, PluginSetup};
use dsh_kernel::KernelError;
use dsh_llm::LlmRuntime;
use dsh_token_meter::TokenMeter;
use serde_json::Value;

use crate::config::resolve_config;
use crate::engine::{BasicCompactionEngine, register_automatic_listeners};

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Inject `llm` and `tokenMeter`, provide `compaction`, and register auto listeners when `auto`.
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            let resolved = resolve_config(&config).map_err(|error| setup_err(error.to_string()))?;
            let llm = ctx.inject::<Mutex<LlmRuntime>>("llm").await?;
            let meter = ctx.inject::<TokenMeter>("tokenMeter").await?;
            let auto = resolved.auto;
            let engine = BasicCompactionEngine::new(llm, meter, resolved).with_kernel(ctx.clone());
            ctx.provide("compaction", engine)
                .map_err(|error| setup_err(error.to_string()))?;
            if auto {
                register_automatic_listeners(&ctx);
            }
            Ok(())
        })
    });
    registry.register(PLUGIN_COMPACTION_BASIC, setup);
}

#[cfg(test)]
mod tests {
    use super::register;
    use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
    use dsh_kernel::Context;

    #[tokio::test]
    async fn unknown_config_key_fails_plugin_load() {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register(&mut registry);
        let err = boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-compaction-basic'\n  config:\n    density: 3\n",
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
