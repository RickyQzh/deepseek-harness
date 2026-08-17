//! Phase 6 product plugin aggregator for the DeepSeek Harness Rust host.

use dsh_boot::PluginRegistry;

/// Register Phase 6 product plugins by YAML name.
///
/// Does not register spine, execution, headless, or `sdk-jsonrpc-server` plugins.
/// Time-context, the tool-result pruner, `@deepseek-ai/dsh-mcp-client`, and the
/// PTY plugin names (`@deepseek-ai/dsh-terminal`, `pty-snapshot-backend`,
/// `@deepseek-ai/dsh-terminal-bash`, `@deepseek-ai/dsh-tool-terminal`) are
/// registered so a later `--patch` can mount them; default base YAML omits those
/// rows and does not allocate a PTY.
pub fn register_base_plugins(registry: &mut PluginRegistry) {
    dsh_user_approval::plugin::register(registry);
    dsh_user_approval::plugin::register_auto_approve(registry);
    dsh_permission_presets::plugin::register(registry);
    dsh_llm::plugin::register_retry(registry);
    dsh_llm::retry_snapshot::register_retry_snapshot_backend(registry);
    dsh_token_meter::plugin::register(registry);
    dsh_compaction_basic::register(registry);
    dsh_compaction_basic::register_pruner(registry);
    dsh_agent_instructions::register(registry);
    dsh_time_context::register(registry);
    dsh_skill::plugin::register_skill(registry);
    dsh_skill::plugin::register_skill_filesystem(registry);
    dsh_skill::plugin::register_tool_skill(registry);
    dsh_web::plugin::register(registry);
    dsh_web_search_deepseek::plugin::register(registry);
    dsh_tool_web::plugin::register(registry);
    dsh_jobs_local::plugin::register(registry);
    dsh_tool_jobs::plugin::register(registry);
    dsh_subagent::plugin::register(registry);
    dsh_subagent_in_process::plugin::register_spawn(registry);
    dsh_subagent_in_process::plugin::register_fork(registry);
    dsh_tool_subagent::plugin::register(registry);
    dsh_mcp_client::register_mcp_plugins(registry);
    dsh_terminal::register_terminal_plugins(registry);
    dsh_terminal_bash::register(registry);
    dsh_tool_terminal::register(registry);
}

#[cfg(test)]
mod phase6_exit;

#[cfg(test)]
mod phase8_mcp_exit;

#[cfg(test)]
mod phase8_pty_exit;

#[cfg(test)]
mod tests {
    use super::register_base_plugins;
    use dsh_agent::{register_execution_plugins, register_spine_plugins};
    use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
    use dsh_compaction_basic::BasicCompactionEngine;
    use dsh_jobs_local::LocalJobRegistry;
    use dsh_kernel::Context;
    use dsh_skill::SkillRegistry;
    use dsh_subagent::SubagentRuntime;
    use dsh_token_meter::TokenMeter;
    use dsh_tools::ToolRuntime;
    use dsh_user_approval::ApprovalService;
    use dsh_web::WebRuntime;
    use std::sync::Mutex;
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

    fn yaml_without_headless_rows(yaml: &str) -> String {
        yaml.lines()
            .filter(|line| {
                let trimmed = line.trim();
                trimmed != "- name: headless-startup" && trimmed != "- name: headless-runner"
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[tokio::test]
    async fn register_base_plugins_provides_approval_compaction_skills_web_jobs_subagents() {
        let _session_root = SESSION_ROOT_LOCK.lock().await;
        let root = test_temp_dir("dsh-base");
        let previous = std::env::var("DSH_SESSION_ROOT").ok();
        unsafe {
            std::env::set_var("DSH_SESSION_ROOT", root.as_os_str());
        }
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register_spine_plugins(&mut registry);
        register_execution_plugins(&mut registry);
        register_base_plugins(&mut registry);
        let yaml = yaml_without_headless_rows(include_str!("../../dsh-headless/base.cordis.yml"));
        let result = boot_yaml(&ctx, &yaml, &[], &registry, &process_interpolate_env()).await;
        restore_session_root(previous);
        result.unwrap();
        assert!(ctx.get::<ApprovalService>("approval").is_some());
        assert!(ctx.get::<TokenMeter>("tokenMeter").is_some());
        assert!(ctx.get::<BasicCompactionEngine>("compaction").is_some());
        assert!(ctx.get::<SkillRegistry>("skills").is_some());
        assert!(ctx.get::<WebRuntime>("web").is_some());
        assert!(ctx.get::<LocalJobRegistry>("jobs").is_some());
        assert!(ctx.get::<SubagentRuntime>("subagents").is_some());
        let tools = ctx.get::<Mutex<ToolRuntime>>("tools").unwrap();
        let names = tools.lock().unwrap().registered_names();
        assert!(names.iter().any(|n| n == "skill"));
        assert!(names.iter().any(|n| n == "web_search"));
        assert!(names.iter().any(|n| n == "subagent"));
        assert!(!names.iter().any(|n| n == "web_fetch"));
    }

    #[test]
    fn base_yaml_rejects_js_tag_substring() {
        let yaml = include_str!("../../dsh-headless/base.cordis.yml");
        assert!(!yaml.contains("!!js"));
        let jsonrpc = include_str!("../../dsh-sdk-jsonrpc-server/base.cordis.yml");
        assert!(!jsonrpc.contains("!!js"));
    }

    #[tokio::test]
    async fn unknown_yaml_name_still_fails_loud() {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register_spine_plugins(&mut registry);
        register_execution_plugins(&mut registry);
        register_base_plugins(&mut registry);
        let err = boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-not-a-plugin'\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .map(|_| ())
        .expect_err("unknown");
        assert!(err.to_string().contains("@deepseek-ai/dsh-not-a-plugin"));
    }

    fn yaml_omits_pty_plugin_names(yaml: &str) {
        assert!(!yaml.contains("@deepseek-ai/dsh-terminal"));
        assert!(!yaml.contains("dsh-terminal-bash"));
        assert!(!yaml.contains("dsh-tool-terminal"));
        assert!(!yaml.contains("pty-snapshot-backend"));
    }

    #[tokio::test]
    async fn register_base_plugins_resolves_mcp_client_yaml_name() {
        let yaml = "\
- name: '@deepseek-ai/dsh-tools'
- name: '@deepseek-ai/dsh-mcp-client'
  config:
    transport: stdio
    serverName: baseMcpTrue
    command: /bin/true
    reconnect:
      enabled: false
    failOnStartupError: false
";
        assert!(!yaml.contains("!!js"));
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register_spine_plugins(&mut registry);
        register_base_plugins(&mut registry);
        let result = boot_yaml(&ctx, yaml, &[], &registry, &process_interpolate_env()).await;
        ctx.dispose().await;
        result.expect("mcp-client yaml name must resolve");
    }

    #[tokio::test]
    async fn register_base_plugins_installs_terminal_yaml_names() {
        let yaml = "\
- name: '@deepseek-ai/dsh-tools'
- name: '@deepseek-ai/dsh-system-prompt'
- name: '@deepseek-ai/dsh-subprocess-local'
- name: '@deepseek-ai/dsh-terminal'
- name: pty-snapshot-backend
- name: '@deepseek-ai/dsh-terminal-bash'
  config:
    backendType: bash
- name: '@deepseek-ai/dsh-tool-terminal'
";
        assert!(!yaml.contains("!!js"));
        yaml_omits_pty_plugin_names(include_str!("../../dsh-headless/minimal.cordis.yml"));
        yaml_omits_pty_plugin_names(include_str!("../../dsh-headless/base.cordis.yml"));
        yaml_omits_pty_plugin_names(include_str!("../../dsh-acp/acp.cordis.yml"));
        yaml_omits_pty_plugin_names(include_str!("../../dsh-host/web.cordis.yml"));
        yaml_omits_pty_plugin_names(include_str!("../../dsh-sdk-jsonrpc-server/base.cordis.yml"));
        yaml_omits_pty_plugin_names(include_str!(
            "../../../examples/jsonrpc-agent/rust.snapshot.cordis.yml"
        ));
        yaml_omits_pty_plugin_names(include_str!(
            "../../../examples/acp-agent/rust.snapshot.cordis.yml"
        ));
        let ctx = Context::new();
        ctx.provide(
            "sandboxPolicy",
            dsh_sandbox::SandboxPolicyResolver::new(
                dsh_sandbox::SandboxMode::DangerFullAccess,
                std::env::temp_dir().to_string_lossy().into_owned(),
            ),
        )
        .expect("provide sandboxPolicy");
        let mut registry = PluginRegistry::new();
        register_spine_plugins(&mut registry);
        register_execution_plugins(&mut registry);
        register_base_plugins(&mut registry);
        let result = boot_yaml(&ctx, yaml, &[], &registry, &process_interpolate_env()).await;
        ctx.dispose().await;
        result.expect("terminal yaml names must resolve");
    }

    #[test]
    fn minimal_yaml_does_not_mention_mcp_client() {
        assert!(!include_str!("../../dsh-headless/minimal.cordis.yml").contains("dsh-mcp-client"));
        assert!(!include_str!("../../dsh-headless/base.cordis.yml").contains("dsh-mcp-client"));
        assert!(!include_str!("../../dsh-acp/acp.cordis.yml").contains("dsh-mcp-client"));
        assert!(!include_str!("../../dsh-host/web.cordis.yml").contains("dsh-mcp-client"));
        assert!(
            !include_str!("../../dsh-sdk-jsonrpc-server/base.cordis.yml")
                .contains("dsh-mcp-client")
        );
        assert!(
            !include_str!("../../../examples/jsonrpc-agent/rust.snapshot.cordis.yml")
                .contains("dsh-mcp-client")
        );
        assert!(
            !include_str!("../../../examples/acp-agent/rust.snapshot.cordis.yml")
                .contains("dsh-mcp-client")
        );
    }
}
