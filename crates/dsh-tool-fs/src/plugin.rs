//! Kernel plugin `@deepseek-ai/dsh-tool-fs`.

use std::sync::{Arc, Mutex};

use dsh_boot::{PLUGIN_TOOL_FS, PluginRegistry, PluginSetup};
use dsh_fs::{LocalFileSystem, ObservationGate, ObservationOwner};
use dsh_subprocess::LocalSubprocessRuntime;
use dsh_tools::ToolRuntime;

use crate::{FsToolContext, register_fs_tools};

/// Register read/write/edit/glob/grep on `tools`.
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, _config| {
        Box::pin(async move {
            let tools = ctx.inject::<Mutex<ToolRuntime>>("tools").await?;
            let fs = ctx.inject::<LocalFileSystem>("fs").await?;
            let subprocess = ctx.inject::<LocalSubprocessRuntime>("subprocess").await?;
            register_fs_tools(
                &mut tools.lock().expect("tools"),
                FsToolContext {
                    fs,
                    gate: Arc::new(ObservationGate::new()),
                    owner: ObservationOwner(1),
                    sandbox: None,
                    subprocess,
                    rg_binary: "rg".into(),
                },
            );
            Ok(())
        })
    });
    registry.register(PLUGIN_TOOL_FS, setup);
}
