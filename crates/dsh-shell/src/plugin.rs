//! Kernel plugin `@deepseek-ai/dsh-shell-bash-local`.

use std::sync::Arc;

use dsh_boot::{PLUGIN_SHELL, PluginRegistry, PluginSetup};
use dsh_kernel::KernelError;
use dsh_subprocess::LocalSubprocessRuntime;

use crate::{BashConfig, LocalBashExecutor};

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Provide unfenced `shell`.
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, _config| {
        Box::pin(async move {
            let subprocess = ctx.inject::<LocalSubprocessRuntime>("subprocess").await?;
            let shell = LocalBashExecutor::new(subprocess, BashConfig::default())
                .map_err(|error| setup_err(error.to_string()))?;
            ctx.provide("shell", shell)
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_SHELL, setup);
}
