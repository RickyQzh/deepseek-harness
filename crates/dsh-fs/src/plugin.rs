//! Kernel plugin `@deepseek-ai/dsh-fs-local`.

use std::sync::Arc;

use dsh_boot::{PLUGIN_FS, PluginRegistry, PluginSetup};
use dsh_kernel::KernelError;
use serde_json::Value;

use crate::LocalFileSystem;

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Provide `fs`. Cwd: config `cwd`, else `DSH_CWD`, else process cwd.
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            let cwd = config
                .get("cwd")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| std::env::var("DSH_CWD").ok())
                .unwrap_or_else(|| {
                    std::env::current_dir()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_else(|_| ".".into())
                });
            ctx.provide("fs", LocalFileSystem::new(cwd))
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_FS, setup);
}
