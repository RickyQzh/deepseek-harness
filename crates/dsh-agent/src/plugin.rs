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

/// Provide `agents` from the kernel `llm`, `tools`, and `systemPrompt` services.
///
/// Stores the injected mutex Arcs. Sibling plugins may still `register_adapter`
/// / `register` after `inject` returns, because `inject` waits only for the
/// service slot, not for those mutations.
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, _config| {
        Box::pin(async move {
            let llm = ctx.inject::<Mutex<LlmRuntime>>("llm").await?;
            let tools = ctx.inject::<Mutex<ToolRuntime>>("tools").await?;
            let prompt = ctx.inject::<SystemPrompt>("systemPrompt").await?;
            ctx.provide(
                "agents",
                AgentRegistry::from_shared(Arc::clone(&llm), Arc::clone(&tools), (*prompt).clone()),
            )
            .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_AGENT, setup);
}
