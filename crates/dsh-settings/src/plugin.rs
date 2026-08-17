//! Kernel plugin `@deepseek-ai/dsh-settings`.

use std::path::PathBuf;
use std::sync::Arc;

use dsh_boot::PluginSetup;
use dsh_kernel::KernelError;
use serde_json::Value;

use crate::service::{SettingsService, ensure_settings_dir};

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Register the settings plugin on `registry`.
pub fn register(registry: &mut dsh_boot::PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            let dir = persist_dir_from_config(&config).map_err(setup_err)?;
            ensure_settings_dir(&dir).map_err(|error| setup_err(error.to_string()))?;
            ctx.provide("settings", SettingsService::with_dir(dir))
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(dsh_boot::PLUGIN_SETTINGS, setup);
}

const CONFIG_KEYS: &[&str] = &["dir"];

fn persist_dir_from_config(config: &Value) -> Result<PathBuf, String> {
    match config {
        Value::Null => persist_dir_from_env(),
        Value::Object(map) => {
            for key in map.keys() {
                if !CONFIG_KEYS.contains(&key.as_str()) {
                    return Err(format!("SettingsConfig: unknown key \"{key}\""));
                }
            }
            match map.get("dir") {
                None | Some(Value::Null) => persist_dir_from_env(),
                Some(Value::String(dir)) => {
                    if dir.is_empty() {
                        return Err("SettingsConfig.dir must be a non-empty string".into());
                    }
                    Ok(PathBuf::from(dir))
                }
                Some(_) => Err("SettingsConfig.dir must be a string".into()),
            }
        }
        _ => Err("SettingsConfig: config must be an object".into()),
    }
}

fn persist_dir_from_env() -> Result<PathBuf, String> {
    match std::env::var("DSH_HOME") {
        Ok(home) if !home.is_empty() => Ok(PathBuf::from(home).join("settings")),
        _ => Err(
            "DSH_HOME must be set for settings, or config.dir must be a non-empty string".into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::register;
    use crate::SettingsService;
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
    async fn provides_settings_from_config_dir() {
        let dir = test_temp_dir("settings-plugin");
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register(&mut registry);
        let yaml = format!(
            "- name: '@deepseek-ai/dsh-settings'\n  config:\n    dir: \"{}\"\n",
            dir.display()
        );
        boot_yaml(&ctx, &yaml, &[], &registry, &process_interpolate_env())
            .await
            .expect("boot settings");
        let service = ctx
            .inject::<SettingsService>("settings")
            .await
            .expect("settings");
        let view = service.describe("ui-onboarding").unwrap();
        assert_eq!(view.revision(), 0);
        assert_eq!(view.ns(), "ui-onboarding");
    }

    #[test]
    fn persist_dir_from_config_rejects_unknown_key() {
        let err = super::persist_dir_from_config(&serde_json::json!({"path": "/x"})).unwrap_err();
        assert!(err.contains("unknown key"));
    }

    #[test]
    fn persist_dir_from_config_uses_dir_string() {
        let dir =
            super::persist_dir_from_config(&serde_json::json!({"dir": "/tmp/settings"})).unwrap();
        assert_eq!(dir, std::path::PathBuf::from("/tmp/settings"));
    }

    #[tokio::test]
    async fn dir_that_cannot_be_created_fails_load() {
        let dir = test_temp_dir("settings-blocked");
        let blocked = dir.join("blocked");
        std::fs::write(&blocked, b"not-a-dir").unwrap();
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register(&mut registry);
        let yaml = format!(
            "- name: '@deepseek-ai/dsh-settings'\n  config:\n    dir: \"{}\"\n",
            blocked.display()
        );
        let err = boot_yaml(&ctx, &yaml, &[], &registry, &process_interpolate_env())
            .await
            .map(|_| ())
            .unwrap_err();
        assert!(err.to_string().contains("plugin setup failed"), "{err}");
    }
}
