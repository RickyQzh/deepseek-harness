//! Compose YAML and mount each row through the closed registry.

use dsh_compose::{
    Entry, InterpolateEnv, apply_entry_patches, interpolate_value, is_disabled, parse_yaml_entries,
    parse_yaml_patches,
};
use dsh_kernel::{Context, FiberHandle};

use crate::error::BootError;
use crate::registry::PluginRegistry;

/// Interpolators from this process: env map, cwd, `DSH_HOME` or `$HOME/.dsh`, platform.
#[must_use]
pub fn process_interpolate_env() -> InterpolateEnv {
    let mut env = std::collections::BTreeMap::new();
    for (key, value) in std::env::vars() {
        env.insert(key, value);
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let dsh_home = std::env::var("DSH_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(|home| std::path::PathBuf::from(home).join(".dsh"))
                .unwrap_or_else(|_| std::path::PathBuf::from("/tmp/dsh"))
        });
    let platform = if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "windows"
    };
    InterpolateEnv {
        env,
        cwd,
        dsh_home,
        platform: platform.into(),
    }
}

fn mount_one(
    ctx: &Context,
    entry: &Entry,
    registry: &PluginRegistry,
    env: &InterpolateEnv,
    handles: &mut Vec<FiberHandle>,
) -> Result<(), BootError> {
    if is_disabled(&entry.disabled, env) {
        return Ok(());
    }
    if entry.group {
        for child in &entry.children {
            mount_one(ctx, child, registry, env, handles)?;
        }
        return Ok(());
    }
    let setup = registry
        .get(&entry.name)
        .cloned()
        .ok_or_else(|| BootError::UnknownPlugin {
            name: entry.name.clone(),
        })?;
    let config = interpolate_value(&entry.config, env)?;
    let handle = ctx.plugin(move |plugin_ctx| async move { setup(plugin_ctx, config).await });
    handles.push(handle);
    Ok(())
}

/// Mount already-parsed entries. Unknown names fail before spawn of that row.
///
/// # Errors
///
/// `UnknownPlugin`, compose interpolation failure, or kernel setup failure after await.
pub async fn mount_entries(
    ctx: &Context,
    entries: &[Entry],
    registry: &PluginRegistry,
    env: &InterpolateEnv,
) -> Result<Vec<FiberHandle>, BootError> {
    let mut handles = Vec::new();
    for entry in entries {
        mount_one(ctx, entry, registry, env, &mut handles)?;
    }
    for handle in &handles {
        handle.await_ready().await?;
    }
    Ok(handles)
}

/// Parse `yaml` as an entry list, apply `--patch` documents, interpolate, mount.
///
/// # Errors
///
/// `!!js`, unknown names, missing patch ids, interpolation, or kernel setup.
pub async fn boot_yaml(
    ctx: &Context,
    yaml: &str,
    patches: &[String],
    registry: &PluginRegistry,
    env: &InterpolateEnv,
) -> Result<Vec<FiberHandle>, BootError> {
    let mut entries = parse_yaml_entries(yaml)?;
    for patch_src in patches {
        let patch_list = parse_yaml_patches(patch_src)?;
        entries = apply_entry_patches(&entries, &patch_list)?;
    }
    mount_entries(ctx, &entries, registry, env).await
}

#[cfg(test)]
mod tests {
    use super::boot_yaml;
    use crate::{BootError, PluginRegistry, PluginSetup};
    use dsh_compose::{InterpolateEnv, parse_yaml_entries};
    use dsh_kernel::Context;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn env() -> InterpolateEnv {
        InterpolateEnv {
            env: BTreeMap::new(),
            cwd: PathBuf::from("/work"),
            dsh_home: PathBuf::from("/tmp/dsh"),
            platform: "linux".into(),
        }
    }

    fn probe_registry() -> PluginRegistry {
        let mut registry = PluginRegistry::new();
        let setup: PluginSetup = Arc::new(|ctx, config| {
            Box::pin(async move {
                let marker = config
                    .get("marker")
                    .and_then(|v| v.as_str())
                    .unwrap_or("probe")
                    .to_string();
                ctx.provide("probe", marker).map(|_| ())?;
                Ok(())
            })
        });
        registry.register("probe", setup);
        registry
    }

    #[tokio::test]
    async fn mounts_named_plugin_and_provides_service() {
        let ctx = Context::new();
        let yaml = "- name: probe\n  config:\n    marker: hello\n";
        let handles = boot_yaml(&ctx, yaml, &[], &probe_registry(), &env())
            .await
            .unwrap();
        for handle in &handles {
            handle.await_ready().await.unwrap();
        }
        assert_eq!(
            ctx.get::<String>("probe").as_deref().map(String::as_str),
            Some("hello")
        );
    }

    #[tokio::test]
    async fn unknown_name_fails_loud() {
        let ctx = Context::new();
        let yaml = "- name: definitely-not-registered\n";
        let err = boot_yaml(&ctx, yaml, &[], &probe_registry(), &env())
            .await
            .map(|_| ())
            .expect_err("unknown");
        match err {
            BootError::UnknownPlugin { name } => {
                assert_eq!(name, "definitely-not-registered");
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn js_tag_is_still_rejected() {
        let ctx = Context::new();
        let yaml = "- name: probe\n  config:\n    task: !!js 'nope'\n";
        let err = boot_yaml(&ctx, yaml, &[], &probe_registry(), &env())
            .await
            .map(|_| ())
            .expect_err("js");
        let text = err.to_string();
        assert!(text.contains("!!js"), "{text}");
    }

    #[tokio::test]
    async fn parse_yaml_entries_rejects_js_before_mount() {
        let err = parse_yaml_entries("- name: probe\n  config: !!js 1\n").unwrap_err();
        assert!(err.to_string().contains("!!js"));
    }
}
