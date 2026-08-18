//! Kernel plugin `@deepseek-ai/dsh-session-persistence-jsonl`.

use std::sync::Arc;

use dsh_boot::{PLUGIN_SESSION_JSONL, PluginRegistry, PluginSetup};
use dsh_kernel::KernelError;
use serde_json::Value;

use crate::JsonlSessionStore;

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Provide `sessions`. Config `root` overrides env when it is a string.
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            let store = if let Some(root) = config.get("root").and_then(Value::as_str) {
                JsonlSessionStore::with_root(root)
            } else {
                JsonlSessionStore::from_env().map_err(|error| setup_err(error.to_string()))?
            };
            ctx.provide("sessions", store)
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_SESSION_JSONL, setup);
}
