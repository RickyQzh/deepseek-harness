//! Kernel plugin `@deepseek-ai/dsh-workspace`.

use std::path::PathBuf;
use std::sync::Arc;

use dsh_boot::PluginSetup;
use dsh_kernel::KernelError;
use serde_json::Value;

use crate::registry::{WorkspaceRegistry, default_persist_path, ensure_persist_parent};

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Register the workspace plugin on `registry`.
pub fn register(registry: &mut dsh_boot::PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            let path = persist_path_from_config(&config).map_err(setup_err)?;
            ensure_persist_parent(&path).map_err(|error| setup_err(error.to_string()))?;
            ctx.provide("workspaces", WorkspaceRegistry::with_path(path))
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(dsh_boot::PLUGIN_WORKSPACE, setup);
}

const CONFIG_KEYS: &[&str] = &["path"];

fn persist_path_from_config(config: &Value) -> Result<PathBuf, String> {
    match config {
        Value::Null => persist_path_from_env(),
        Value::Object(map) => {
            for key in map.keys() {
                if !CONFIG_KEYS.contains(&key.as_str()) {
                    return Err(format!("WorkspaceConfig: unknown key \"{key}\""));
                }
            }
            match map.get("path") {
                None | Some(Value::Null) => persist_path_from_env(),
                Some(Value::String(path)) => {
                    if path.is_empty() {
                        return Err("WorkspaceConfig.path must be a non-empty string".into());
                    }
                    Ok(PathBuf::from(path))
                }
                Some(_) => Err("WorkspaceConfig.path must be a string".into()),
            }
        }
        _ => Err("WorkspaceConfig: config must be an object".into()),
    }
}

fn persist_path_from_env() -> Result<PathBuf, String> {
    let home = std::env::var("DSH_HOME").ok();
    let session_root = std::env::var("DSH_SESSION_ROOT").ok();
    default_persist_path(home.as_deref(), session_root.as_deref())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::register;
    use crate::WorkspaceRegistry;
    use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
    use dsh_kernel::Context;

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
    async fn provides_workspaces_from_config_path() {
        let dir = test_temp_dir("ws-plugin");
        let persist = dir.join("workspaces.json");
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register(&mut registry);
        let yaml = format!(
            "- name: '@deepseek-ai/dsh-workspace'\n  config:\n    path: \"{}\"\n",
            persist.display()
        );
        boot_yaml(&ctx, &yaml, &[], &registry, &process_interpolate_env())
            .await
            .expect("boot workspace");
        let service = ctx
            .inject::<WorkspaceRegistry>("workspaces")
            .await
            .expect("workspaces");
        let cwd = dir.join("proj");
        std::fs::create_dir(&cwd).unwrap();
        let (ws, created) = service.create(&cwd).unwrap();
        assert!(created);
        assert_eq!(ws.title(), "proj");
    }

    #[test]
    fn persist_path_from_config_rejects_unknown_key() {
        let err = super::persist_path_from_config(&serde_json::json!({"root": "/x"})).unwrap_err();
        assert!(err.contains("unknown key"));
    }

    #[test]
    fn persist_path_from_config_uses_path_string() {
        let path =
            super::persist_path_from_config(&serde_json::json!({"path": "/tmp/w.json"})).unwrap();
        assert_eq!(path, std::path::PathBuf::from("/tmp/w.json"));
    }

    #[tokio::test]
    async fn parent_that_cannot_be_created_fails_load() {
        let dir = test_temp_dir("ws-blocked");
        let blocked = dir.join("blocked");
        std::fs::write(&blocked, b"not-a-dir").unwrap();
        let persist = blocked.join("workspaces.json");
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register(&mut registry);
        let yaml = format!(
            "- name: '@deepseek-ai/dsh-workspace'\n  config:\n    path: \"{}\"\n",
            persist.display()
        );
        let err = boot_yaml(&ctx, &yaml, &[], &registry, &process_interpolate_env())
            .await
            .map(|_| ())
            .unwrap_err();
        assert!(err.to_string().contains("plugin setup failed"), "{err}");
    }
}
