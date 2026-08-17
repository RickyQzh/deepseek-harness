//! Kernel plugin `@deepseek-ai/dsh-terminal-bash`.

use std::sync::{Arc, Mutex, PoisonError};

use dsh_boot::{PLUGIN_TERMINAL_BASH, PluginRegistry, PluginSetup};
use dsh_kernel::KernelError;
use dsh_sandbox::SandboxPolicyResolver;
use dsh_subprocess::LocalSubprocessRuntime;
use dsh_terminal::{TerminalBackend, TerminalSessionService};
use serde_json::Value;

use crate::backend::BashTerminalBackend;
use crate::config::parse_config;

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Register YAML `@deepseek-ai/dsh-terminal-bash`.
///
/// Parses config (unknown keys fail load), injects `terminals`, `subprocess`, and
/// `sandboxPolicy`, and registers the configured backend type. Spawn argv is the
/// configured shell path and args; this plugin does not wrap argv through sandbox
/// confine.
///
/// # Parameters
///
/// * `registry` - Closed plugin-name registry.
///
/// # Returns
///
/// Nothing. The YAML name is installed on `registry`.
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            let parsed = parse_config(&config).map_err(setup_err)?;
            let terminals = ctx
                .inject::<Mutex<TerminalSessionService>>("terminals")
                .await?;
            let subprocess = ctx.inject::<LocalSubprocessRuntime>("subprocess").await?;
            let sandbox_policy = ctx.inject::<SandboxPolicyResolver>("sandboxPolicy").await?;
            let backend = BashTerminalBackend::new(parsed, subprocess, sandbox_policy);
            let backend_type = backend.backend_type().to_string();
            let backend: Arc<dyn TerminalBackend> = Arc::new(backend);
            terminals
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .register_backend(backend_type, backend)
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_TERMINAL_BASH, setup);
}

#[cfg(test)]
mod tests {
    use super::register;
    use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
    use dsh_kernel::Context;
    use dsh_sandbox::{SandboxMode, SandboxPolicyResolver};
    use dsh_session::SessionId;
    use dsh_subprocess::LocalSubprocessRuntime;
    use dsh_terminal::{TerminalSessionService, TerminalSpawnRequest};
    use std::sync::Mutex;

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

    async fn lock_pty_tests() -> tokio::sync::MutexGuard<'static, ()> {
        static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        LOCK.lock().await
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn open_shell_returns_prompt_motd() {
        let _guard = lock_pty_tests().await;
        if !std::path::Path::new("/bin/bash").is_file() {
            eprintln!("skipping open_shell_returns_prompt_motd: /bin/bash is missing");
            return;
        }
        let root = test_temp_dir("dsh-terminal-bash-motd");
        let ctx = Context::new();
        ctx.provide("terminals", Mutex::new(TerminalSessionService::new()))
            .expect("provide terminals");
        ctx.provide("subprocess", LocalSubprocessRuntime::new())
            .expect("provide subprocess");
        ctx.provide(
            "sandboxPolicy",
            SandboxPolicyResolver::new(
                SandboxMode::DangerFullAccess,
                root.to_string_lossy().into_owned(),
            ),
        )
        .expect("provide sandboxPolicy");
        let mut registry = PluginRegistry::new();
        register(&mut registry);
        boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-terminal-bash'\n  config:\n    pollIntervalMs: 10\n    exactProbeAfterMs: 20\n    idleSilenceMs: 250\n    handoffGraceMs: 50\n    timeoutMs: 5000\n    disposeGraceMs: 500\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .expect("boot terminal-bash");
        let terminals = ctx
            .inject::<Mutex<TerminalSessionService>>("terminals")
            .await
            .expect("terminals");
        let service = terminals
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let owner = SessionId::new("owner-a");
        let spawned = service
            .spawn(
                owner.clone(),
                TerminalSpawnRequest::new("shell").with_cwd(root.to_string_lossy().into_owned()),
            )
            .await
            .expect("spawn shell");
        let motd = spawned.motd().to_string();
        let _ = service
            .kill(&owner, spawned.session_id(), "test close")
            .await;
        let _ = std::fs::remove_dir_all(&root);
        assert!(motd.contains("dsh> "), "motd {motd:?}");
    }
}
