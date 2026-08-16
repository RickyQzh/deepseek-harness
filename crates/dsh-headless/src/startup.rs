//! Kernel plugin `headless-startup`.

use std::sync::Arc;

use dsh_boot::{PLUGIN_HEADLESS_STARTUP, PluginRegistry, PluginSetup};
use dsh_kernel::KernelError;

use crate::error::HeadlessError;
use crate::io::{CmdlineArgs, HeadlessStartup};

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Parse the inner argv into `headlessStartup`.
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, _config| {
        Box::pin(async move {
            let args = ctx.inject::<CmdlineArgs>("cmdlineArgs").await?;
            let task = args.get().join(" ");
            if task.trim().is_empty() {
                return Err(setup_err(HeadlessError::MissingTask.to_string()));
            }
            ctx.provide("headlessStartup", HeadlessStartup::new(task))
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_HEADLESS_STARTUP, setup);
}
