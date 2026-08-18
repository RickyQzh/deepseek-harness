//! Kernel plugin `@deepseek-ai/dsh-tools`.

use std::sync::{Arc, Mutex};

use dsh_boot::{PLUGIN_TOOLS, PluginRegistry, PluginSetup};
use dsh_kernel::KernelError;

use crate::{ToolPresentationMode, ToolRuntime};

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Provide an empty native tool runtime.
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, _config| {
        Box::pin(async move {
            ctx.provide(
                "tools",
                Mutex::new(ToolRuntime::new(ToolPresentationMode::Native)),
            )
            .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_TOOLS, setup);
}
