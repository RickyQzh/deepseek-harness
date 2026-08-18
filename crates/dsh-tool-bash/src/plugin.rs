//! Kernel plugin `@deepseek-ai/dsh-tool-bash`.

use std::sync::{Arc, Mutex};

use dsh_boot::{PLUGIN_TOOL_BASH, PluginRegistry, PluginSetup};
use dsh_shell::LocalBashExecutor;
use dsh_tools::ToolRuntime;

use crate::register_bash_tool;

/// Register unfenced `bash` on `tools`.
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, _config| {
        Box::pin(async move {
            let tools = ctx.inject::<Mutex<ToolRuntime>>("tools").await?;
            let shell = ctx.inject::<LocalBashExecutor>("shell").await?;
            register_bash_tool(&mut tools.lock().expect("tools"), shell, None);
            Ok(())
        })
    });
    registry.register(PLUGIN_TOOL_BASH, setup);
}
