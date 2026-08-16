//! Kernel plugin `@deepseek-ai/dsh-agent`.

use std::sync::{Arc, Mutex};

use dsh_boot::{PLUGIN_AGENT, PluginRegistry, PluginSetup};
use dsh_kernel::KernelError;
use dsh_llm::LlmRuntime;
use dsh_system_prompt::SystemPrompt;
use dsh_tools::ToolRuntime;

use crate::AgentRegistry;

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Provide `agents` from the shared llm, tools, and systemPrompt services.
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, _config| {
        Box::pin(async move {
            let llm = ctx.inject::<Mutex<LlmRuntime>>("llm").await?;
            let tools = ctx.inject::<Mutex<ToolRuntime>>("tools").await?;
            let prompt = ctx.inject::<SystemPrompt>("systemPrompt").await?;
            let llm = llm.lock().expect("llm").clone();
            let tools = tools.lock().expect("tools").clone();
            let prompt = (*prompt).clone();
            ctx.provide("agents", AgentRegistry::new(llm, tools, prompt))
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_AGENT, setup);
}
