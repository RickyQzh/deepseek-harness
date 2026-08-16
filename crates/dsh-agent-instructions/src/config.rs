//! Load-time validation for workspace instruction discovery and rendering.

use std::path::PathBuf;

/// Load-time configuration failure for agent-instructions.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{0}")]
pub struct ConfigError(String);

impl ConfigError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

const CONFIG_KEYS: &[&str] = &[
    "dshHome",
    "projectRootMarkers",
    "maxBytes",
    "maxSourceBytes",
    "instructionFileCandidates",
    "localInstructionFileCandidates",
];

const DEFAULT_PROJECT_ROOT_MARKERS: &[&str] = &[".git"];
const DEFAULT_INSTRUCTION_FILE_CANDIDATES: &[&str] = &["AGENTS.md", "CLAUDE.md"];
const DEFAULT_LOCAL_INSTRUCTION_FILE_CANDIDATES: &[&str] = &["AGENTS.local.md", "CLAUDE.local.md"];
const DEFAULT_MAX_SOURCE_BYTES: u64 = 1_048_576;

/// Validated plugin configuration used by discovery and the pre-step listener.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentInstructionsConfig {
    /// Harness home containing the user-global `AGENTS.md`; `None` resolves from the environment.
    pub dsh_home: Option<PathBuf>,
    /// Directory entries that identify the project root while walking upward from the session cwd.
    pub project_root_markers: Vec<String>,
    /// UTF-8 byte cap for one rendered baseline. Zero disables loading.
    pub max_bytes: u64,
    /// Maximum UTF-8 bytes read from one instruction file; larger files are ignored.
    pub max_source_bytes: u64,
    /// Ordered same-directory project candidates.
    pub instruction_file_candidates: Vec<String>,
    /// Ordered same-directory local-overlay candidates loaded after the base files.
    pub local_instruction_file_candidates: Vec<String>,
}

/// Resolve plugin JSON into a validated [`AgentInstructionsConfig`].
///
/// # Errors
///
/// Unknown keys, a missing `maxBytes`, or a value that is not an object.
pub fn resolve_config(value: &serde_json::Value) -> Result<AgentInstructionsConfig, ConfigError> {
    match value {
        serde_json::Value::Null => Err(ConfigError::new(
            "AgentInstructionsConfig: maxBytes is required",
        )),
        serde_json::Value::Object(map) => {
            validate_keys(map)?;
            let max_bytes = required_u64(value, "maxBytes")?;
            let max_source_bytes = match value.get("maxSourceBytes") {
                None => DEFAULT_MAX_SOURCE_BYTES,
                Some(item) => as_u64(item).ok_or_else(|| {
                    ConfigError::new(
                        "AgentInstructionsConfig.maxSourceBytes must be a non-negative integer",
                    )
                })?,
            };
            if max_source_bytes == 0 {
                return Err(ConfigError::new(
                    "AgentInstructionsConfig.maxSourceBytes must be a positive integer",
                ));
            }
            Ok(AgentInstructionsConfig {
                dsh_home: optional_path(value, "dshHome")?,
                project_root_markers: string_list_or(
                    value,
                    "projectRootMarkers",
                    DEFAULT_PROJECT_ROOT_MARKERS,
                )?,
                max_bytes,
                max_source_bytes,
                instruction_file_candidates: resolve_candidates(
                    value,
                    "instructionFileCandidates",
                    DEFAULT_INSTRUCTION_FILE_CANDIDATES,
                )?,
                local_instruction_file_candidates: resolve_candidates(
                    value,
                    "localInstructionFileCandidates",
                    DEFAULT_LOCAL_INSTRUCTION_FILE_CANDIDATES,
                )?,
            })
        }
        _ => Err(ConfigError::new(
            "AgentInstructionsConfig: config must be an object",
        )),
    }
}

fn validate_keys(map: &serde_json::Map<String, serde_json::Value>) -> Result<(), ConfigError> {
    for key in map.keys() {
        if !CONFIG_KEYS.contains(&key.as_str()) {
            return Err(ConfigError::new(format!(
                "AgentInstructionsConfig: unknown key \"{key}\""
            )));
        }
    }
    Ok(())
}

fn required_u64(value: &serde_json::Value, key: &str) -> Result<u64, ConfigError> {
    match value.get(key) {
        None => Err(ConfigError::new(
            "AgentInstructionsConfig: maxBytes is required",
        )),
        Some(item) => as_u64(item).ok_or_else(|| {
            ConfigError::new(format!(
                "AgentInstructionsConfig.{key} must be a non-negative integer"
            ))
        }),
    }
}

fn as_u64(value: &serde_json::Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|n| u64::try_from(n).ok()))
        .or_else(|| {
            let number = value.as_f64()?;
            if number.is_finite() && number >= 0.0 && number.fract() == 0.0 {
                Some(number as u64)
            } else {
                None
            }
        })
}

fn optional_path(value: &serde_json::Value, key: &str) -> Result<Option<PathBuf>, ConfigError> {
    match value.get(key) {
        None => Ok(None),
        Some(serde_json::Value::String(text)) => Ok(Some(PathBuf::from(text))),
        Some(_) => Err(ConfigError::new(format!(
            "AgentInstructionsConfig.{key} must be a string"
        ))),
    }
}

fn string_list_or(
    value: &serde_json::Value,
    key: &str,
    fallback: &[&str],
) -> Result<Vec<String>, ConfigError> {
    match value.get(key) {
        None => Ok(fallback.iter().map(|item| (*item).to_string()).collect()),
        Some(serde_json::Value::Array(items)) => {
            let mut out = Vec::new();
            for item in items {
                let Some(text) = item.as_str() else {
                    return Err(ConfigError::new(format!(
                        "AgentInstructionsConfig.{key} must be an array of strings"
                    )));
                };
                out.push(text.to_string());
            }
            Ok(out)
        }
        Some(_) => Err(ConfigError::new(format!(
            "AgentInstructionsConfig.{key} must be an array of strings"
        ))),
    }
}

fn resolve_candidates(
    value: &serde_json::Value,
    key: &str,
    fallback: &[&str],
) -> Result<Vec<String>, ConfigError> {
    Ok(string_list_or(value, key, fallback)?
        .into_iter()
        .filter(|candidate| {
            !candidate.is_empty()
                && candidate != "."
                && candidate != ".."
                && !candidate.contains('/')
                && !candidate.contains('\\')
        })
        .collect())
}

/// Resolve `$DSH_HOME` or `~/.dsh` when `dshHome` is omitted.
#[must_use]
pub fn resolve_dsh_home(configured: Option<&std::path::Path>) -> PathBuf {
    if let Some(path) = configured {
        return expand_home(&path.to_string_lossy());
    }
    if let Ok(home) = std::env::var("DSH_HOME") {
        if !home.is_empty() {
            return PathBuf::from(home);
        }
    }
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => PathBuf::from(home).join(".dsh"),
        _ => PathBuf::from(".dsh"),
    }
}

fn expand_home(path: &str) -> PathBuf {
    if path == "~" {
        return std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(path));
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::resolve_config;
    use serde_json::json;

    #[test]
    fn unknown_key_fails_load() {
        let err = resolve_config(&json!({"maxBytes": 65536, "density": 3})).unwrap_err();
        assert!(err.to_string().contains("unknown key"));
    }

    #[test]
    fn missing_max_bytes_fails_load() {
        let err = resolve_config(&json!({})).unwrap_err();
        assert!(err.to_string().contains("maxBytes"));
    }

    #[test]
    fn defaults_match_typescript() {
        let config = resolve_config(&json!({"maxBytes": 65536})).unwrap();
        assert_eq!(config.max_bytes, 65536);
        assert_eq!(config.max_source_bytes, 1_048_576);
        assert_eq!(config.project_root_markers, vec![".git".to_string()]);
        assert_eq!(
            config.instruction_file_candidates,
            vec!["AGENTS.md".to_string(), "CLAUDE.md".to_string()]
        );
        assert_eq!(
            config.local_instruction_file_candidates,
            vec!["AGENTS.local.md".to_string(), "CLAUDE.local.md".to_string()]
        );
    }
}
