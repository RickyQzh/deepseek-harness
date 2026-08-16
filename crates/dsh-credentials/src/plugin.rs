//! Kernel plugin `@deepseek-ai/dsh-credentials`.

use std::sync::Arc;

use dsh_boot::PluginSetup;
use dsh_kernel::KernelError;
use serde_json::Value;

use crate::LayeredCredentials;

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Register the credentials plugin on `registry`.
pub fn register(registry: &mut dsh_boot::PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, _config: Value| {
        Box::pin(async move {
            ctx.provide("credentials", LayeredCredentials::new())
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(dsh_boot::PLUGIN_CREDENTIALS, setup);
}
