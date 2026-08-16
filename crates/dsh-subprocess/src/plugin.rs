//! Kernel plugin `@deepseek-ai/dsh-subprocess-local`.

use std::sync::Arc;

use dsh_boot::{PLUGIN_SUBPROCESS, PluginRegistry, PluginSetup};
use dsh_kernel::KernelError;

use crate::LocalSubprocessRuntime;

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Provide `subprocess`.
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, _config| {
        Box::pin(async move {
            ctx.provide("subprocess", LocalSubprocessRuntime::new())
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_SUBPROCESS, setup);
}
