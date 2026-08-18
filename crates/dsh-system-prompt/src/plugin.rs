//! Kernel plugin `@deepseek-ai/dsh-system-prompt`.

use std::sync::Arc;

use dsh_boot::{PLUGIN_SYSTEM_PROMPT, PluginRegistry, PluginSetup};
use dsh_kernel::KernelError;
use serde_json::Value;

use crate::{SystemPrompt, SystemPromptConfig};

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Provide `systemPrompt` from YAML config.
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            let persona = config
                .get("persona")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let include_harness_identity = config
                .get("includeHarnessIdentity")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let include_runtime_context = config
                .get("includeRuntimeContext")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let prompt = SystemPrompt::new(SystemPromptConfig {
                include_harness_identity,
                include_runtime_context,
                persona,
                tool_order: None,
            })
            .map_err(|error| setup_err(error.to_string()))?;
            ctx.provide("systemPrompt", prompt)
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_SYSTEM_PROMPT, setup);
}
