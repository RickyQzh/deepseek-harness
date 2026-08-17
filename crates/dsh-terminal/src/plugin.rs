//! Kernel plugins `@deepseek-ai/dsh-terminal` and `pty-snapshot-backend`.

use std::sync::{Arc, Mutex, PoisonError};

use dsh_boot::{PLUGIN_PTY_SNAPSHOT_BACKEND, PLUGIN_TERMINAL, PluginRegistry, PluginSetup};
use dsh_kernel::KernelError;
use serde_json::Value;

use crate::{SnapshotBackend, TerminalSessionService};

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Register YAML `@deepseek-ai/dsh-terminal`.
///
/// Provides `terminals` as [`Mutex<TerminalSessionService>`]. Unknown config keys fail load.
/// This plugin does not spawn a PTY.
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
            reject_unknown_keys(&config, "TerminalConfig").map_err(setup_err)?;
            let service = TerminalSessionService::new();
            ctx.provide("terminals", Mutex::new(service))
                .map_err(|error| setup_err(error.to_string()))?;
            let terminals = ctx
                .get::<Mutex<TerminalSessionService>>("terminals")
                .ok_or_else(|| setup_err("terminals provide vanished"))?;
            ctx.effect(move || async move {
                terminals
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .dispose_all();
            })
            .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_TERMINAL, setup);
}

/// Register YAML `pty-snapshot-backend`.
///
/// Injects `terminals` and registers backend type `shell`.
///
/// # Parameters
///
/// * `registry` - Closed plugin-name registry.
///
/// # Returns
///
/// Nothing. The YAML name is installed on `registry`.
pub fn register_snapshot_backend(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            reject_unknown_keys(&config, "PtySnapshotBackendConfig").map_err(setup_err)?;
            let terminals = ctx
                .inject::<Mutex<TerminalSessionService>>("terminals")
                .await?;
            terminals
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .register_backend("shell", Arc::new(SnapshotBackend))
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_PTY_SNAPSHOT_BACKEND, setup);
}

/// Register terminal plugins by calling [`register`] then [`register_snapshot_backend`].
///
/// Terminal is registered first so the snapshot backend can inject `terminals`.
///
/// # Parameters
///
/// * `registry` - Closed plugin-name registry.
///
/// # Returns
///
/// Nothing. Both YAML names are installed on `registry`.
pub fn register_terminal_plugins(registry: &mut PluginRegistry) {
    register(registry);
    register_snapshot_backend(registry);
}

fn reject_unknown_keys(config: &Value, prefix: &str) -> Result<(), String> {
    match config {
        Value::Null => Ok(()),
        Value::Object(map) => {
            if let Some(key) = map.keys().next() {
                return Err(format!("{prefix}: unknown key \"{key}\""));
            }
            Ok(())
        }
        _ => Err(format!("{prefix}: config must be an object")),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
    use dsh_kernel::Context;
    use dsh_session::SessionId;

    use super::register_terminal_plugins;
    use crate::{TerminalSessionService, TerminalSpawnRequest};

    #[test]
    fn plugin_name_is_typescript_package_name() {
        assert_eq!(dsh_boot::PLUGIN_TERMINAL, "@deepseek-ai/dsh-terminal");
        assert_eq!(
            dsh_boot::PLUGIN_PTY_SNAPSHOT_BACKEND,
            "pty-snapshot-backend"
        );
        let mut registry = PluginRegistry::new();
        register_terminal_plugins(&mut registry);
        assert!(registry.get(dsh_boot::PLUGIN_TERMINAL).is_some());
        assert!(
            registry
                .get(dsh_boot::PLUGIN_PTY_SNAPSHOT_BACKEND)
                .is_some()
        );
    }

    #[tokio::test]
    async fn snapshot_backend_injects_shell_on_terminals() {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register_terminal_plugins(&mut registry);
        boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-terminal'\n- name: pty-snapshot-backend\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .expect("boot terminal plugins");
        let terminals = ctx
            .inject::<Mutex<TerminalSessionService>>("terminals")
            .await
            .expect("terminals");
        let service = terminals
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let spawned = service
            .spawn(
                SessionId::new("owner-a"),
                TerminalSpawnRequest::new("shell"),
            )
            .expect("spawn through snapshot backend");
        assert_eq!(spawned.session_id().as_str(), "pty-1");
        assert_eq!(spawned.motd(), "dsh> ");
    }

    #[tokio::test]
    async fn unknown_config_key_fails_plugin_load() {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register_terminal_plugins(&mut registry);
        let err = boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-terminal'\n  config:\n    spawn: true\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .map(|_| ())
        .unwrap_err();
        assert!(err.to_string().contains("unknown key"), "{err}");
    }
}
