//! Kernel plugin `@deepseek-ai/dsh-jobs-local`.

use std::sync::Arc;

use dsh_boot::{PLUGIN_JOBS_LOCAL, PluginRegistry, PluginSetup};
use dsh_kernel::KernelError;
use serde_json::Value;

use crate::{DEFAULT_MAX_CONCURRENT_PER_OWNER, LocalJobRegistry};

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Provide `jobs` as [`LocalJobRegistry`].
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            let max = resolve_config(&config).map_err(setup_err)?;
            ctx.provide("jobs", LocalJobRegistry::new(max))
                .map_err(|error| setup_err(error.to_string()))?;
            let jobs = ctx
                .get::<LocalJobRegistry>("jobs")
                .ok_or_else(|| setup_err("jobs provide vanished"))?;
            ctx.effect(move || async move {
                jobs.cancel_live("jobs service disposed");
            })
            .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_JOBS_LOCAL, setup);
}

const CONFIG_KEYS: &[&str] = &["maxConcurrentJobsPerOwner"];

fn resolve_config(value: &Value) -> Result<u32, String> {
    match value {
        Value::Null => Ok(DEFAULT_MAX_CONCURRENT_PER_OWNER),
        Value::Object(map) => {
            for key in map.keys() {
                if !CONFIG_KEYS.contains(&key.as_str()) {
                    return Err(format!("JobsConfig: unknown key \"{key}\""));
                }
            }
            match map.get("maxConcurrentJobsPerOwner") {
                None | Some(Value::Null) => Ok(DEFAULT_MAX_CONCURRENT_PER_OWNER),
                Some(item) => {
                    let invalid =
                        || "JobsConfig.maxConcurrentJobsPerOwner must be a positive integer".into();
                    let Some(number) = item.as_u64() else {
                        return Err(invalid());
                    };
                    if number == 0 {
                        return Err(invalid());
                    }
                    u32::try_from(number).map_err(|_| invalid())
                }
            }
        }
        _ => Err("JobsConfig: config must be an object".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::register;
    use crate::LocalJobRegistry;
    use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
    use dsh_kernel::Context;

    #[tokio::test]
    async fn provides_jobs_runtime() {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register(&mut registry);
        boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-jobs-local'\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .expect("boot jobs-local");
        ctx.inject::<LocalJobRegistry>("jobs").await.expect("jobs");
    }

    #[tokio::test]
    async fn abstract_jobs_yaml_name_is_unknown() {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register(&mut registry);
        let err = boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-jobs'\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .map(|_| ())
        .unwrap_err();
        assert!(err.to_string().contains("unknown plugin"), "{}", err);
    }

    #[tokio::test]
    async fn unknown_config_key_fails_plugin_load() {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register(&mut registry);
        let err = boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-jobs-local'\n  config:\n    maxJobs: 3\n",
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
    async fn zero_limit_fails_plugin_load() {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register(&mut registry);
        let err = boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-jobs-local'\n  config:\n    maxConcurrentJobsPerOwner: 0\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .map(|_| ())
        .unwrap_err();
        assert!(err.to_string().contains("positive integer"));
    }
}
