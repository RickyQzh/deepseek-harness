//! YAML plugin `@deepseek-ai/dsh-permission-presets`.

use std::sync::Arc;

use dsh_agent::AgentRegistry;
use dsh_boot::{PLUGIN_PERMISSION_PRESETS, PluginRegistry, PluginSetup};
use dsh_kernel::KernelError;
use dsh_sandbox::SandboxMode;
use dsh_shell::LocalBashExecutor;
use dsh_user_approval::ApprovalService;
use serde_json::Value;

use crate::{PermissionPresetConfig, PermissionPresetService};

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Provide `permissionPresets` and pin knobs from `agents.on_session_create`.
///
/// Injects `agents`, `approval`, and `shell`. Composed sandbox is
/// [`LocalBashExecutor::sandbox_mode`] or [`SandboxMode::WorkspaceWrite`] when that is [`None`].
/// Composed approval is [`ApprovalService::effective_policy`] on an empty log.
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            let agents = ctx.inject::<AgentRegistry>("agents").await?;
            let approval = ctx.inject::<ApprovalService>("approval").await?;
            let shell = ctx.inject::<LocalBashExecutor>("shell").await?;
            let composed_sandbox = match shell.sandbox_mode() {
                Some(mode) => mode,
                None => SandboxMode::WorkspaceWrite,
            };
            let composed_approval = approval.effective_policy(&[]);
            let parsed = PermissionPresetConfig::from_value(&config)
                .map_err(|error| setup_err(error.to_string()))?;
            let service =
                PermissionPresetService::from_config(parsed, composed_sandbox, composed_approval)
                    .map_err(|error| setup_err(error.to_string()))?;
            let hook_service = service.clone();
            ctx.provide("permissionPresets", service)
                .map_err(|error| setup_err(error.to_string()))?;
            agents.on_session_create(Arc::new(move |session| {
                hook_service
                    .pin_initial(session)
                    .expect("pin permission presets");
            }));
            Ok(())
        })
    });
    registry.register(PLUGIN_PERMISSION_PRESETS, setup);
}

#[cfg(test)]
mod tests {
    use super::register;
    use crate::{PermissionPresetService, effective_permission_preset, effective_sandbox_mode};
    use dsh_agent::{AgentRegistry, CreateAgentOptions};
    use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
    use dsh_kernel::Context;
    use dsh_llm::LlmRuntime;
    use dsh_sandbox::SandboxMode;
    use dsh_session::SessionId;
    use dsh_shell::{BashConfig, LocalBashExecutor};
    use dsh_subprocess::LocalSubprocessRuntime;
    use dsh_system_prompt::{SystemPrompt, SystemPromptConfig};
    use dsh_tools::{ToolPresentationMode, ToolRuntime};
    use dsh_user_approval::{ApprovalPolicy, ApprovalService, effective_approval_policy};
    use std::sync::Arc;

    fn provide_deps(ctx: &Context) {
        ctx.provide(
            "agents",
            AgentRegistry::new(
                LlmRuntime::new(),
                ToolRuntime::new(ToolPresentationMode::Native),
                SystemPrompt::new(SystemPromptConfig::default()).unwrap(),
            ),
        )
        .unwrap();
        ctx.provide(
            "approval",
            ApprovalService::new(ctx.clone(), ApprovalPolicy::Ask),
        )
        .unwrap();
        ctx.provide(
            "shell",
            LocalBashExecutor::new(
                Arc::new(LocalSubprocessRuntime::new()),
                BashConfig::default(),
            )
            .unwrap(),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn plugin_pins_workspace_write_on_session_create() {
        let ctx = Context::new();
        provide_deps(&ctx);
        let mut registry = PluginRegistry::new();
        register(&mut registry);
        boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-permission-presets'\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .unwrap();
        let names = ctx
            .get::<PermissionPresetService>("permissionPresets")
            .unwrap()
            .names();
        assert_eq!(
            names,
            vec![
                "workspace-write".to_string(),
                "danger-full-access".to_string()
            ]
        );
        let agents = ctx.get::<AgentRegistry>("agents").unwrap();
        let handle = agents
            .create(CreateAgentOptions {
                session_id: SessionId::new("perm-plugin"),
                cwd: None,
                provider: "mock".into(),
                model: "mock".into(),
                max_tokens: None,
            })
            .unwrap();
        let guard = handle.lock();
        assert_eq!(
            effective_permission_preset(guard.session.events()).as_deref(),
            Some("workspace-write")
        );
        assert_eq!(
            effective_sandbox_mode(guard.session.events()),
            Some(SandboxMode::WorkspaceWrite)
        );
        assert_eq!(
            effective_approval_policy(guard.session.events()),
            Some(ApprovalPolicy::Ask)
        );
    }

    #[tokio::test]
    async fn plugin_unknown_default_preset_fails_load() {
        let ctx = Context::new();
        provide_deps(&ctx);
        let mut registry = PluginRegistry::new();
        register(&mut registry);
        let err = boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-permission-presets'\n  config:\n    defaultPreset: not-a-preset\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .map(|_| ())
        .expect_err("unknown defaultPreset");
        assert!(err.to_string().contains("unknown preset"));
    }

    #[tokio::test]
    async fn plugin_dsh_base_yaml_includes_read_only() {
        let ctx = Context::new();
        provide_deps(&ctx);
        let mut registry = PluginRegistry::new();
        register(&mut registry);
        let yaml = "\
- name: '@deepseek-ai/dsh-permission-presets'
  config:
    presets:
      read-only: { sandbox: read-only, approval: ask }
      workspace-write: { sandbox: workspace-write, approval: ask }
      danger-full-access: { sandbox: danger-full-access, approval: never }
";
        boot_yaml(&ctx, yaml, &[], &registry, &process_interpolate_env())
            .await
            .unwrap();
        let names = ctx
            .get::<PermissionPresetService>("permissionPresets")
            .unwrap()
            .names();
        assert!(names.iter().any(|name| name == "read-only"), "{names:?}");
    }
}
