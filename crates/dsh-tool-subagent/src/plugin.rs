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
            wait_until_named_provider(&subagents, &resolved.provider).await?;
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

/// Poll until `ready`, then true; false after 64 yield-plus-1ms attempts.
async fn wait_until(mut ready: impl FnMut() -> bool) -> bool {
    const ATTEMPTS: u32 = 64;
    for attempt in 0..ATTEMPTS {
        if ready() {
            return true;
        }
        if attempt + 1 == ATTEMPTS {
            break;
        }
        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    false
}

/// `boot_yaml` runs plugin fibers concurrently, so spawn/fork `register_provider` may land after this row starts.
async fn wait_until_named_provider(
    subagents: &SubagentRuntime,
    name: &str,
) -> Result<(), dsh_kernel::KernelError> {
    if wait_until(|| subagents.get_provider(name).is_some()).await {
        Ok(())
    } else {
        Err(setup_err(format!(
            "tool-subagent: unknown provider \"{name}\""
        )))
    }
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

    #[tokio::test]
    async fn wait_until_sees_ready_after_empty_polls() {
        let polls = std::cell::Cell::new(0u32);
        assert!(
            super::wait_until(|| {
                let n = polls.get() + 1;
                polls.set(n);
                n >= 5
            })
            .await
        );
        assert!(polls.get() >= 5, "polls={}", polls.get());
    }

    #[tokio::test]
    async fn wait_until_named_provider_fails_when_missing_after_bound() {
        let rt = dsh_subagent::SubagentRuntime::new(dsh_kernel::Context::new());
        let err = super::wait_until_named_provider(&rt, "spawn")
            .await
            .expect_err("missing");
        assert!(
            err.to_string().contains("unknown provider \"spawn\""),
            "{err}"
        );
    }
}
