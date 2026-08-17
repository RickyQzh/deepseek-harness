//! Kernel plugin `@deepseek-ai/dsh-acp`.

use std::sync::Arc;

use dsh_agent::AgentRegistry;
use dsh_boot::PluginSetup;
use dsh_kernel::KernelError;
use dsh_session_persist::JsonlSessionStore;
use serde_json::Value;
use tokio::io::BufReader;

use crate::bridge::AcpBridge;
use crate::rpc::AcpNdjsonTransport;

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Kernel service name the CLI reads after `boot_yaml`.
pub const ACP_SERVER_SERVICE: &str = "acpServer";

/// Inject `agents` and `sessions`, bind stdin/stdout, install the permission listener, and provide [`ACP_SERVER_SERVICE`]. The CLI serves after boot so sibling adapters are visible to `session/new`.
pub fn register(registry: &mut dsh_boot::PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            reject_unknown_config(&config).map_err(setup_err)?;
            let provider = required_config_string(&config, "provider").map_err(setup_err)?;
            let model = required_config_string(&config, "model").map_err(setup_err)?;
            let agents = ctx.inject::<AgentRegistry>("agents").await?;
            let sessions = ctx.inject::<JsonlSessionStore>("sessions").await?;
            let transport =
                AcpNdjsonTransport::new(BufReader::new(tokio::io::stdin()), tokio::io::stdout());
            let bridge = AcpBridge::new(transport, agents, provider, model).with_sessions(sessions);
            bridge.bind();
            bridge.install_permission_listener(&ctx)?;
            ctx.provide(ACP_SERVER_SERVICE, bridge)
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(dsh_boot::PLUGIN_ACP, setup);
}

/// Register ACP plugins by calling [`register`].
pub fn register_acp_plugins(registry: &mut dsh_boot::PluginRegistry) {
    register(registry);
}

const CONFIG_KEYS: &[&str] = &["provider", "model"];

fn reject_unknown_config(config: &Value) -> Result<(), String> {
    match config {
        Value::Object(map) => {
            for key in map.keys() {
                if !CONFIG_KEYS.contains(&key.as_str()) {
                    return Err(format!("AcpConfig: unknown key \"{key}\""));
                }
            }
            Ok(())
        }
        _ => Err("AcpConfig: config must be an object".into()),
    }
}

fn required_config_string(config: &Value, key: &str) -> Result<String, String> {
    match config.get(key) {
        Some(Value::String(value)) => Ok(value.clone()),
        _ => Err(format!("AcpConfig.{key} must be a string")),
    }
}

#[cfg(test)]
mod tests {
    use super::{ACP_SERVER_SERVICE, register_acp_plugins};
    use crate::AcpBridge;
    use dsh_agent::{AgentRegistry, register_execution_plugins, register_spine_plugins};
    use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
    use dsh_kernel::Context;
    use dsh_llm::LlmRuntime;
    use dsh_session_persist::JsonlSessionStore;
    use dsh_system_prompt::{SystemPrompt, SystemPromptConfig};
    use dsh_tools::{ToolPresentationMode, ToolRuntime};
    use tokio::sync::Mutex as AsyncMutex;

    static SESSION_ROOT_LOCK: AsyncMutex<()> = AsyncMutex::const_new(());

    fn test_temp_dir(prefix: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    fn restore_session_root(previous: Option<String>) {
        match previous {
            Some(value) => unsafe { std::env::set_var("DSH_SESSION_ROOT", value) },
            None => unsafe { std::env::remove_var("DSH_SESSION_ROOT") },
        }
    }

    fn provide_agents_and_sessions(ctx: &Context, root: &std::path::Path) {
        ctx.provide(
            "agents",
            AgentRegistry::new(
                LlmRuntime::new(),
                ToolRuntime::new(ToolPresentationMode::Native),
                SystemPrompt::new(SystemPromptConfig::default()).unwrap(),
            ),
        )
        .expect("provide agents");
        ctx.provide("sessions", JsonlSessionStore::with_root(root))
            .expect("provide sessions");
    }

    #[test]
    fn acp_yaml_rejects_js_tag_substring() {
        assert!(!crate::ACP_YAML.contains("!!js"));
        assert!(!crate::ACP_YAML.contains("headless-auto-approve"));
    }

    #[tokio::test]
    async fn plugin_requires_provider_and_model() {
        let _session_root = SESSION_ROOT_LOCK.lock().await;
        let root = test_temp_dir("dsh-acp-plugin-cfg");
        let previous = std::env::var("DSH_SESSION_ROOT").ok();
        unsafe {
            std::env::set_var("DSH_SESSION_ROOT", root.as_os_str());
        }

        let ctx = Context::new();
        provide_agents_and_sessions(&ctx, &root);
        let mut registry = PluginRegistry::new();
        register_acp_plugins(&mut registry);
        let empty = boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-acp'\n  config: {}\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .map(|_| ())
        .expect_err("empty config");
        assert!(
            empty.to_string().contains("provider"),
            "missing provider: {empty}"
        );

        let ctx = Context::new();
        provide_agents_and_sessions(&ctx, &root);
        let mut registry = PluginRegistry::new();
        register_acp_plugins(&mut registry);
        let no_model = boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-acp'\n  config:\n    provider: mock\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .map(|_| ())
        .expect_err("provider only");
        assert!(
            no_model.to_string().contains("model"),
            "missing model: {no_model}"
        );

        restore_session_root(previous);
    }

    #[tokio::test]
    async fn boot_yaml_provides_acp_server() {
        let _session_root = SESSION_ROOT_LOCK.lock().await;
        let root = test_temp_dir("dsh-acp-plugin-boot");
        let previous = std::env::var("DSH_SESSION_ROOT").ok();
        unsafe {
            std::env::set_var("DSH_SESSION_ROOT", root.as_os_str());
        }

        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register_spine_plugins(&mut registry);
        register_execution_plugins(&mut registry);
        dsh_user_approval::plugin::register(&mut registry);
        register_acp_plugins(&mut registry);
        boot_yaml(
            &ctx,
            crate::ACP_YAML,
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .expect("boot acp");
        assert!(ctx.get::<AcpBridge>(ACP_SERVER_SERVICE).is_some());

        restore_session_root(previous);
    }
}
