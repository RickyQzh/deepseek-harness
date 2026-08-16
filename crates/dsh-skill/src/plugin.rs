//! Kernel plugins `@deepseek-ai/dsh-skill`, `@deepseek-ai/dsh-skill-filesystem`, and `@deepseek-ai/dsh-tool-skill`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use dsh_boot::{
    PLUGIN_SKILL, PLUGIN_SKILL_FILESYSTEM, PLUGIN_TOOL_SKILL, PluginRegistry, PluginSetup,
};
use dsh_fs::LocalFileSystem;
use dsh_kernel::KernelError;
use dsh_tools::ToolRuntime;
use serde_json::Value;

use crate::filesystem::FilesystemSkillConfig;
use crate::tool::{
    CatalogConfig, DEFAULT_CATALOG_DESCRIPTION_MAX_LENGTH, register_catalog_listener,
    register_skill_tool,
};
use crate::{FilesystemSkillProvider, SkillRegistry};

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Provide `skills` as an empty [`SkillRegistry`].
pub fn register_skill(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            reject_unknown_keys(&config, &[], "SkillConfig")?;
            ctx.provide("skills", SkillRegistry::new())
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_SKILL, setup);
}

/// Register the local filesystem provider on `skills`.
pub fn register_skill_filesystem(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            let resolved =
                resolve_filesystem_config(&config).map_err(|error| setup_err(error.to_string()))?;
            let skills = ctx.inject::<SkillRegistry>("skills").await?;
            let fs = match ctx.get::<LocalFileSystem>("fs") {
                Some(existing) => LocalFileSystem::new(existing.cwd.clone()),
                None => LocalFileSystem::new(
                    std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
                ),
            };
            skills
                .register_provider(Arc::new(FilesystemSkillProvider::new(fs, resolved)))
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_SKILL_FILESYSTEM, setup);
}

/// Register the `skill` tool and the first-pre-step catalog listener.
pub fn register_tool_skill(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            let catalog =
                resolve_catalog_config(&config).map_err(|error| setup_err(error.to_string()))?;
            let skills = ctx.inject::<SkillRegistry>("skills").await?;
            let tools = ctx.inject::<Mutex<ToolRuntime>>("tools").await?;
            register_skill_tool(
                &mut tools
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                Arc::clone(&skills),
            );
            register_catalog_listener(&ctx, skills, tools, catalog)
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_TOOL_SKILL, setup);
}

const FILESYSTEM_KEYS: &[&str] = &[
    "providerName",
    "includeDefaultRoots",
    "dshHome",
    "agentsHome",
    "customSkillDirs",
    "bundledSkillDir",
];

fn resolve_filesystem_config(value: &Value) -> Result<FilesystemSkillConfig, String> {
    match value {
        Value::Null => Ok(FilesystemSkillConfig::default()),
        Value::Object(map) => {
            reject_unknown_object_keys(map, FILESYSTEM_KEYS, "FilesystemSkillConfig")?;
            let mut config = FilesystemSkillConfig::default();
            if let Some(flag) =
                optional_bool(map.get("includeDefaultRoots"), "includeDefaultRoots")?
            {
                config.include_default_roots = flag;
            }
            if let Some(name) = optional_string(map.get("providerName"), "providerName")? {
                if name.is_empty() {
                    return Err("FilesystemSkillConfig.providerName must be non-empty".into());
                }
                config.provider_name = name;
            }
            config.dsh_home = optional_string(map.get("dshHome"), "dshHome")?.map(PathBuf::from);
            config.agents_home =
                optional_string(map.get("agentsHome"), "agentsHome")?.map(PathBuf::from);
            config.custom_skill_dirs =
                optional_string_list(map.get("customSkillDirs"), "customSkillDirs")?
                    .unwrap_or_default()
                    .into_iter()
                    .map(PathBuf::from)
                    .collect();
            config.bundled_skill_dir =
                optional_string(map.get("bundledSkillDir"), "bundledSkillDir")?.map(PathBuf::from);
            Ok(config)
        }
        _ => Err("FilesystemSkillConfig: config must be an object".into()),
    }
}

fn resolve_catalog_config(value: &Value) -> Result<CatalogConfig, String> {
    match value {
        Value::Null => Ok(CatalogConfig::default()),
        Value::Object(map) => {
            reject_unknown_object_keys(map, &["catalogDescriptionMaxLength"], "ToolSkillConfig")?;
            let mut config = CatalogConfig::default();
            if let Some(length) = optional_usize(
                map.get("catalogDescriptionMaxLength"),
                "catalogDescriptionMaxLength",
            )? {
                if length < 3 {
                    return Err(
                        "tool-skill: catalogDescriptionMaxLength must be an integer greater than or equal to 3"
                            .into(),
                    );
                }
                config.description_max_length = length;
            } else {
                config.description_max_length = DEFAULT_CATALOG_DESCRIPTION_MAX_LENGTH;
            }
            Ok(config)
        }
        _ => Err("ToolSkillConfig: config must be an object".into()),
    }
}

fn reject_unknown_keys(value: &Value, allowed: &[&str], subject: &str) -> Result<(), KernelError> {
    match value {
        Value::Null => Ok(()),
        Value::Object(map) => reject_unknown_object_keys(map, allowed, subject).map_err(setup_err),
        _ => Err(setup_err(format!("{subject}: config must be an object"))),
    }
}

fn reject_unknown_object_keys(
    map: &serde_json::Map<String, Value>,
    allowed: &[&str],
    subject: &str,
) -> Result<(), String> {
    for key in map.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(format!("{subject}: unknown key \"{key}\""));
        }
    }
    Ok(())
}

fn optional_bool(value: Option<&Value>, key: &str) -> Result<Option<bool>, String> {
    match value {
        None => Ok(None),
        Some(Value::Bool(flag)) => Ok(Some(*flag)),
        Some(_) => Err(format!("FilesystemSkillConfig.{key} must be a boolean")),
    }
}

fn optional_string(value: Option<&Value>, key: &str) -> Result<Option<String>, String> {
    match value {
        None => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(format!("FilesystemSkillConfig.{key} must be a string")),
    }
}

fn optional_string_list(value: Option<&Value>, key: &str) -> Result<Option<Vec<String>>, String> {
    match value {
        None => Ok(None),
        Some(Value::Array(items)) => {
            let mut out = Vec::new();
            for item in items {
                match item {
                    Value::String(text) => out.push(text.clone()),
                    _ => {
                        return Err(format!(
                            "FilesystemSkillConfig.{key} must be an array of strings"
                        ));
                    }
                }
            }
            Ok(Some(out))
        }
        Some(_) => Err(format!(
            "FilesystemSkillConfig.{key} must be an array of strings"
        )),
    }
}

fn optional_usize(value: Option<&Value>, key: &str) -> Result<Option<usize>, String> {
    match value {
        None => Ok(None),
        Some(item) => {
            let invalid =
                || format!("tool-skill: {key} must be an integer greater than or equal to 3");
            if let Some(number) = item.as_u64() {
                return usize::try_from(number).map(Some).map_err(|_| invalid());
            }
            Err(invalid())
        }
    }
}
