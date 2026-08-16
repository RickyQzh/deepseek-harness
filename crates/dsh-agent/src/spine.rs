//! Register Phase 5 spine and execution plugins by YAML name.

use dsh_boot::PluginRegistry;

/// Credentials, llm (+ mock/replay/deepseek), tools, system prompt, agent, JSONL store.
pub fn register_spine_plugins(registry: &mut PluginRegistry) {
    dsh_credentials::plugin::register(registry);
    dsh_llm::plugin::register_llm(registry);
    dsh_llm::plugin::register_mock(registry);
    dsh_llm::replay::register(registry);
    dsh_llm_deepseek::plugin::register(registry);
    dsh_tools::plugin::register(registry);
    dsh_system_prompt::plugin::register(registry);
    crate::plugin::register(registry);
    dsh_session_persist::plugin::register(registry);
}

/// Subprocess, fs, bash shell, and model-facing fs/bash tools.
pub fn register_execution_plugins(registry: &mut PluginRegistry) {
    dsh_subprocess::plugin::register(registry);
    dsh_fs::plugin::register(registry);
    dsh_shell::plugin::register(registry);
    dsh_tool_fs::plugin::register(registry);
    dsh_tool_bash::plugin::register(registry);
}

#[cfg(test)]
mod tests {
    use crate::AgentRegistry;
    use crate::{register_execution_plugins, register_spine_plugins};
    use dsh_boot::{
        PLUGIN_AGENT, PLUGIN_CREDENTIALS, PLUGIN_LLM, PLUGIN_LLM_MOCK, PLUGIN_SESSION_JSONL,
        PLUGIN_SYSTEM_PROMPT, PLUGIN_TOOLS, PluginRegistry, boot_yaml, process_interpolate_env,
    };
    use dsh_credentials::LayeredCredentials;
    use dsh_kernel::Context;
    use dsh_llm::LlmRuntime;
    use dsh_session_persist::JsonlSessionStore;
    use dsh_system_prompt::SystemPrompt;
    use dsh_tools::ToolRuntime;
    use std::sync::Mutex;
    use tokio::sync::Mutex as AsyncMutex;

    // Serializes tests that mutate process `DSH_SESSION_ROOT`.
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

    #[tokio::test]
    async fn spine_yaml_provides_agents_llm_tools_sessions() {
        let _session_root = SESSION_ROOT_LOCK.lock().await;
        let root = test_temp_dir("dsh-spine");
        let previous = std::env::var("DSH_SESSION_ROOT").ok();
        unsafe {
            std::env::set_var("DSH_SESSION_ROOT", root.as_os_str());
        }
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register_spine_plugins(&mut registry);
        let yaml = r#"
- name: '@deepseek-ai/dsh-credentials'
- name: '@deepseek-ai/dsh-llm'
- name: '@deepseek-ai/dsh-llm-mock'
  config:
    provider: mock
    text: spine-ok
- name: '@deepseek-ai/dsh-tools'
- name: '@deepseek-ai/dsh-system-prompt'
  config:
    persona: You are a test agent.
    includeRuntimeContext: false
- name: '@deepseek-ai/dsh-agent'
- name: '@deepseek-ai/dsh-session-persistence-jsonl'
"#;
        boot_yaml(&ctx, yaml, &[], &registry, &process_interpolate_env())
            .await
            .unwrap();
        assert!(ctx.get::<LayeredCredentials>("credentials").is_some());
        assert!(ctx.get::<Mutex<LlmRuntime>>("llm").is_some());
        assert!(ctx.get::<Mutex<ToolRuntime>>("tools").is_some());
        assert!(ctx.get::<SystemPrompt>("systemPrompt").is_some());
        assert!(ctx.get::<AgentRegistry>("agents").is_some());
        assert!(ctx.get::<JsonlSessionStore>("sessions").is_some());
        let llm = ctx.get::<Mutex<LlmRuntime>>("llm").unwrap();
        assert!(
            llm.lock()
                .expect("llm")
                .list_providers()
                .iter()
                .any(|p| p == "mock")
        );
        // inject of a provided service does not wait for sibling register_adapter;
        // AgentRegistry must lock the kernel mutex, not a map cloned at setup.
        let agents = ctx.get::<AgentRegistry>("agents").unwrap();
        assert!(
            agents.list_providers().iter().any(|p| p == "mock"),
            "{:?}",
            agents.list_providers()
        );
        match previous {
            Some(value) => unsafe { std::env::set_var("DSH_SESSION_ROOT", value) },
            None => unsafe { std::env::remove_var("DSH_SESSION_ROOT") },
        }
        let _ = (
            PLUGIN_CREDENTIALS,
            PLUGIN_LLM,
            PLUGIN_LLM_MOCK,
            PLUGIN_TOOLS,
            PLUGIN_SYSTEM_PROMPT,
            PLUGIN_AGENT,
            PLUGIN_SESSION_JSONL,
        );
    }

    #[tokio::test]
    async fn unknown_spine_name_still_fails_loud() {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register_spine_plugins(&mut registry);
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

    #[tokio::test]
    async fn execution_yaml_registers_bash_and_fs_tools() {
        let _session_root = SESSION_ROOT_LOCK.lock().await;
        let root = test_temp_dir("dsh-exec");
        let previous = std::env::var("DSH_SESSION_ROOT").ok();
        unsafe {
            std::env::set_var("DSH_SESSION_ROOT", root.as_os_str());
        }
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register_spine_plugins(&mut registry);
        register_execution_plugins(&mut registry);
        let yaml = r#"
- name: '@deepseek-ai/dsh-credentials'
- name: '@deepseek-ai/dsh-llm'
- name: '@deepseek-ai/dsh-llm-mock'
- name: '@deepseek-ai/dsh-tools'
- name: '@deepseek-ai/dsh-system-prompt'
- name: '@deepseek-ai/dsh-agent'
- name: '@deepseek-ai/dsh-session-persistence-jsonl'
- name: '@deepseek-ai/dsh-subprocess-local'
- name: '@deepseek-ai/dsh-fs-local'
- name: '@deepseek-ai/dsh-shell-bash-local'
- name: '@deepseek-ai/dsh-tool-fs'
- name: '@deepseek-ai/dsh-tool-bash'
"#;
        boot_yaml(&ctx, yaml, &[], &registry, &process_interpolate_env())
            .await
            .unwrap();
        let tools = ctx.get::<Mutex<ToolRuntime>>("tools").unwrap();
        let names = tools.lock().expect("tools").registered_names();
        assert!(names.iter().any(|n| n == "bash"), "{names:?}");
        assert!(names.iter().any(|n| n == "read"), "{names:?}");
        match previous {
            Some(value) => unsafe { std::env::set_var("DSH_SESSION_ROOT", value) },
            None => unsafe { std::env::remove_var("DSH_SESSION_ROOT") },
        }
    }
}
