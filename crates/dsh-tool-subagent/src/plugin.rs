//! Kernel plugins for subagent, control, list, and report tools.

use std::sync::{Arc, Mutex, PoisonError};

use dsh_agent::AgentRegistry;
use dsh_boot::{
    PLUGIN_TOOL_SUBAGENT, PLUGIN_TOOL_SUBAGENT_CONTROL, PLUGIN_TOOL_SUBAGENT_LIST,
    PLUGIN_TOOL_SUBAGENT_REPORT, PluginRegistry, PluginSetup,
};
use dsh_subagent::SubagentRuntime;
use dsh_tools::ToolRuntime;
use serde_json::Value;

use crate::control::{register_send_message, resolve_config as resolve_control};
use crate::delegate::{register_delegate_tool, resolve_config as resolve_delegate};
use crate::list::{register_list_agents, resolve_config as resolve_list};
use crate::report::{register_report_setup, resolve_config as resolve_report};
use crate::util::setup_err;

/// Register the four YAML names from this crate.
pub fn register(registry: &mut PluginRegistry) {
    register_delegate(registry);
    register_control(registry);
    register_list(registry);
    register_report(registry);
}

fn register_delegate(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            let resolved = resolve_delegate(&config).map_err(setup_err)?;
            let tools = ctx.inject::<Mutex<ToolRuntime>>("tools").await?;
            let subagents = ctx.inject::<SubagentRuntime>("subagents").await?;
            let agents = ctx.inject::<AgentRegistry>("agents").await?;
            register_delegate_tool(
                &mut tools.lock().unwrap_or_else(PoisonError::into_inner),
                subagents,
                agents,
                resolved,
            )?;
            Ok(())
        })
    });
    registry.register(PLUGIN_TOOL_SUBAGENT, setup);
}

fn register_control(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            resolve_control(&config).map_err(setup_err)?;
            let tools = ctx.inject::<Mutex<ToolRuntime>>("tools").await?;
            let subagents = ctx.inject::<SubagentRuntime>("subagents").await?;
            let agents = ctx.inject::<AgentRegistry>("agents").await?;
            register_send_message(
                &mut tools.lock().unwrap_or_else(PoisonError::into_inner),
                subagents,
                agents,
            );
            Ok(())
        })
    });
    registry.register(PLUGIN_TOOL_SUBAGENT_CONTROL, setup);
}

fn register_list(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            resolve_list(&config).map_err(setup_err)?;
            let tools = ctx.inject::<Mutex<ToolRuntime>>("tools").await?;
            let subagents = ctx.inject::<SubagentRuntime>("subagents").await?;
            let agents = ctx.inject::<AgentRegistry>("agents").await?;
            register_list_agents(
                &mut tools.lock().unwrap_or_else(PoisonError::into_inner),
                subagents,
                agents,
            );
            Ok(())
        })
    });
    registry.register(PLUGIN_TOOL_SUBAGENT_LIST, setup);
}

fn register_report(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            let resolved = resolve_report(&config).map_err(setup_err)?;
            let subagents = ctx.inject::<SubagentRuntime>("subagents").await?;
            let agents = ctx.inject::<AgentRegistry>("agents").await?;
            register_report_setup(&subagents, agents, resolved);
            Ok(())
        })
    });
    registry.register(PLUGIN_TOOL_SUBAGENT_REPORT, setup);
}

#[cfg(test)]
mod tests {
    use super::register;
    use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
    use dsh_kernel::Context;

    #[tokio::test]
    async fn unknown_delegate_config_key_fails_plugin_load() {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        dsh_subagent::register(&mut registry);
        register(&mut registry);
        let err = boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-subagent'\n- name: '@deepseek-ai/dsh-tool-subagent'\n  config:\n    extra: true\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .map(|_| ())
        .unwrap_err();
        assert!(err.to_string().contains("unknown key"));
    }
}
