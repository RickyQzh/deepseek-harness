//! Load-time validation and routed-model policy resolution.

use std::collections::HashSet;

/// Load-time configuration failure for compaction-basic.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{0}")]
pub struct ConfigError(String);

impl ConfigError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// Exact provider/model override for one routed target.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelCompactPolicy {
    /// Provider route to match.
    pub provider: String,
    /// Model id to match.
    pub model: String,
    /// Optional threshold override.
    pub threshold_ratio: Option<f64>,
    /// Optional retain-ratio override.
    pub retain_ratio: Option<f64>,
    /// Optional absolute retain budget.
    pub retain_tokens: Option<u64>,
    /// Optional summarization provider override.
    pub summarization_provider: Option<String>,
    /// Optional summarization model override.
    pub summarization_model: Option<String>,
    /// Optional summarization token cap.
    pub max_tokens: Option<u64>,
    /// Optional extra pressure attempts.
    pub compaction_retries: Option<u32>,
    /// Optional overflow retry cap.
    pub max_overflow_retries: Option<u32>,
}

/// Validated compaction-basic configuration.
#[derive(Clone, Debug, PartialEq)]
pub struct BasicCompactionConfig {
    /// Compact when estimated tokens reach this fraction of the routed context window.
    pub threshold_ratio: f64,
    /// Recent surface fraction kept verbatim when `retain_tokens` is unset.
    pub retain_ratio: f64,
    /// Absolute recent surface budget kept verbatim, when set.
    pub retain_tokens: Option<u64>,
    /// Summarization provider; empty falls back to the routed request then loop options.
    pub summarization_provider: String,
    /// Summarization model; empty falls back with `summarization_provider`.
    pub summarization_model: String,
    /// Provider generation cap for the summarization call.
    pub max_tokens: u64,
    /// Extra pressure attempts after the first.
    pub compaction_retries: u32,
    /// Maximum retries after canonical context-window overflow.
    pub max_overflow_retries: u32,
    /// Register automatic pressure and overflow listeners when true.
    pub auto: bool,
    /// Exact provider/model policy overrides.
    pub model_policies: Vec<ModelCompactPolicy>,
}

impl Default for BasicCompactionConfig {
    fn default() -> Self {
        Self {
            threshold_ratio: 0.8,
            retain_ratio: 0.16,
            retain_tokens: None,
            summarization_provider: String::new(),
            summarization_model: String::new(),
            max_tokens: 8192,
            compaction_retries: 1,
            max_overflow_retries: 1,
            auto: true,
            model_policies: Vec::new(),
        }
    }
}

const BASIC_KEYS: &[&str] = &[
    "thresholdRatio",
    "retainRatio",
    "retainTokens",
    "summarizationProvider",
    "summarizationModel",
    "maxTokens",
    "compactionRetries",
    "maxOverflowRetries",
    "modelPolicies",
    "auto",
];

const MODEL_POLICY_KEYS: &[&str] = &[
    "provider",
    "model",
    "thresholdRatio",
    "retainRatio",
    "retainTokens",
    "summarizationProvider",
    "summarizationModel",
    "maxTokens",
    "compactionRetries",
    "maxOverflowRetries",
];

/// Pressure and retention budgets for one routed model capacity.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedCompactSpec {
    /// Inclusive estimated-token threshold that triggers pressure compaction.
    pub threshold_tokens: u64,
    /// Verbatim tail budget in estimated tokens.
    pub retain_tokens: u64,
    /// Extra attempts after the first pressure compaction.
    pub compaction_retries: u32,
    /// Overflow retry cap for this target.
    pub max_overflow_retries: u32,
    /// Summarization provider; empty inherits the routed request.
    pub summarization_provider: String,
    /// Summarization model; empty inherits the routed request.
    pub summarization_model: String,
    /// Summarization output cap.
    pub max_tokens: u64,
}

/// Resolve plugin JSON into a validated [`BasicCompactionConfig`].
///
/// # Errors
///
/// Unknown keys, invalid types, exclusive retention conflict, or a retain ratio
/// that is not below the threshold ratio.
pub fn resolve_config(value: &serde_json::Value) -> Result<BasicCompactionConfig, ConfigError> {
    match value {
        serde_json::Value::Null => Ok(BasicCompactionConfig::default()),
        serde_json::Value::Object(map) if map.is_empty() => Ok(BasicCompactionConfig::default()),
        serde_json::Value::Object(_) => resolve_object(value, "BasicCompactionConfig"),
        _ => Err(ConfigError::new(
            "BasicCompactionConfig: config must be an object",
        )),
    }
}

fn resolve_object(
    value: &serde_json::Value,
    name: &str,
) -> Result<BasicCompactionConfig, ConfigError> {
    validate_keys(value, BASIC_KEYS, name)?;
    validate_policy_fields(value, name)?;
    if let Some(auto) = value.get("auto") {
        if !auto.is_boolean() {
            return Err(ConfigError::new(format!("{name}: auto must be a boolean")));
        }
    }
    let threshold_ratio = f64_or(value, "thresholdRatio", 0.8);
    let retain_tokens = u64_opt(value, "retainTokens");
    let retain_ratio = if retain_tokens.is_some() && value.get("retainRatio").is_none() {
        0.16
    } else {
        f64_or(value, "retainRatio", 0.16)
    };
    if retain_tokens.is_none() && retain_ratio >= threshold_ratio {
        return Err(ConfigError::new(format!(
            "{name}: retainRatio ({retain_ratio}) must be less than \
             the resolved thresholdRatio ({threshold_ratio})"
        )));
    }
    let model_policies = resolve_model_policies(value.get("modelPolicies"))?;
    for (index, policy) in model_policies.iter().enumerate() {
        let policy_threshold = policy.threshold_ratio.unwrap_or(threshold_ratio);
        let policy_retain = match (policy.retain_tokens, policy.retain_ratio) {
            (Some(_), _) => None,
            (None, Some(ratio)) => Some(ratio),
            (None, None) => {
                if retain_tokens.is_some() {
                    None
                } else {
                    Some(retain_ratio)
                }
            }
        };
        if let Some(ratio) = policy_retain {
            if ratio >= policy_threshold {
                return Err(ConfigError::new(format!(
                    "{name}: modelPolicies[{index}]: retainRatio ({ratio}) must be less than \
                     the resolved thresholdRatio ({policy_threshold})"
                )));
            }
        }
    }
    Ok(BasicCompactionConfig {
        threshold_ratio,
        retain_ratio,
        retain_tokens,
        summarization_provider: string_or(value, "summarizationProvider", ""),
        summarization_model: string_or(value, "summarizationModel", ""),
        max_tokens: u64_or(value, "maxTokens", 8192),
        compaction_retries: u32_or(value, "compactionRetries", 1),
        max_overflow_retries: u32_or(value, "maxOverflowRetries", 1),
        auto: value
            .get("auto")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true),
        model_policies,
    })
}

/// Merge exact-target overrides over service defaults.
#[must_use]
pub fn resolve_target_policy(
    config: &BasicCompactionConfig,
    provider: &str,
    model: &str,
) -> BasicCompactionConfig {
    let override_policy = config
        .model_policies
        .iter()
        .find(|policy| policy.provider == provider && policy.model == model);
    let Some(override_policy) = override_policy else {
        return config.clone();
    };
    let (retain_ratio, retain_tokens) = if override_policy.retain_tokens.is_some() {
        (config.retain_ratio, override_policy.retain_tokens)
    } else if override_policy.retain_ratio.is_some() {
        (
            override_policy.retain_ratio.unwrap_or(config.retain_ratio),
            None,
        )
    } else {
        (config.retain_ratio, config.retain_tokens)
    };
    BasicCompactionConfig {
        threshold_ratio: override_policy
            .threshold_ratio
            .unwrap_or(config.threshold_ratio),
        retain_ratio,
        retain_tokens,
        summarization_provider: override_policy
            .summarization_provider
            .clone()
            .unwrap_or_else(|| config.summarization_provider.clone()),
        summarization_model: override_policy
            .summarization_model
            .clone()
            .unwrap_or_else(|| config.summarization_model.clone()),
        max_tokens: override_policy.max_tokens.unwrap_or(config.max_tokens),
        compaction_retries: override_policy
            .compaction_retries
            .unwrap_or(config.compaction_retries),
        max_overflow_retries: override_policy
            .max_overflow_retries
            .unwrap_or(config.max_overflow_retries),
        auto: config.auto,
        model_policies: Vec::new(),
    }
}

/// Scale one routed policy into token budgets for `context_window`.
///
/// # Errors
///
/// Non-positive capacity, or retain tokens that are not below the threshold.
pub fn resolve_compact_spec(
    policy: &BasicCompactionConfig,
    provider: &str,
    model: &str,
    context_window: u64,
) -> Result<ResolvedCompactSpec, ConfigError> {
    if context_window == 0 {
        return Err(ConfigError::new(format!(
            "BasicCompactionConfig: contextWindow ({context_window}) must be a positive integer"
        )));
    }
    let threshold_tokens = ((context_window as f64) * policy.threshold_ratio).floor() as u64;
    let retain_tokens = match policy.retain_tokens {
        Some(tokens) => tokens,
        None => ((context_window as f64) * policy.retain_ratio).floor() as u64,
    };
    if retain_tokens >= threshold_tokens {
        return Err(ConfigError::new(format!(
            "BasicCompactionConfig: {provider}/{model} retainTokens \
             ({retain_tokens}) must be less than threshold tokens {threshold_tokens}"
        )));
    }
    Ok(ResolvedCompactSpec {
        threshold_tokens,
        retain_tokens,
        compaction_retries: policy.compaction_retries,
        max_overflow_retries: policy.max_overflow_retries,
        summarization_provider: policy.summarization_provider.clone(),
        summarization_model: policy.summarization_model.clone(),
        max_tokens: policy.max_tokens,
    })
}

fn resolve_model_policies(
    configured: Option<&serde_json::Value>,
) -> Result<Vec<ModelCompactPolicy>, ConfigError> {
    let Some(configured) = configured else {
        return Ok(Vec::new());
    };
    let Some(items) = configured.as_array() else {
        return Err(ConfigError::new(
            "BasicCompactionConfig: modelPolicies must be an array",
        ));
    };
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(items.len());
    for (index, source) in items.iter().enumerate() {
        let name = format!("BasicCompactionConfig: modelPolicies[{index}]");
        if !source.is_object() {
            return Err(ConfigError::new(format!("{name} must be an object")));
        }
        validate_keys(source, MODEL_POLICY_KEYS, &name)?;
        let provider = required_non_empty(&name, "provider", source.get("provider"))?;
        let model = required_non_empty(&name, "model", source.get("model"))?;
        validate_policy_fields(source, &name)?;
        let key = format!("{provider}\0{model}");
        if !seen.insert(key) {
            return Err(ConfigError::new(format!(
                "BasicCompactionConfig: duplicate model policy for {provider}/{model}"
            )));
        }
        out.push(ModelCompactPolicy {
            provider,
            model,
            threshold_ratio: f64_opt(source, "thresholdRatio"),
            retain_ratio: f64_opt(source, "retainRatio"),
            retain_tokens: u64_opt(source, "retainTokens"),
            summarization_provider: string_opt(source, "summarizationProvider"),
            summarization_model: string_opt(source, "summarizationModel"),
            max_tokens: u64_opt(source, "maxTokens"),
            compaction_retries: u32_opt(source, "compactionRetries"),
            max_overflow_retries: u32_opt(source, "maxOverflowRetries"),
        });
    }
    Ok(out)
}

fn validate_policy_fields(config: &serde_json::Value, name: &str) -> Result<(), ConfigError> {
    if let Some(value) = config.get("thresholdRatio") {
        assert_ratio(name, "thresholdRatio", value)?;
    }
    if let Some(value) = config.get("retainRatio") {
        assert_ratio(name, "retainRatio", value)?;
    }
    if let Some(value) = config.get("retainTokens") {
        assert_non_negative_int(name, "retainTokens", value)?;
    }
    if config.get("retainRatio").is_some() && config.get("retainTokens").is_some() {
        return Err(ConfigError::new(format!(
            "{name}: retainRatio and retainTokens are mutually exclusive"
        )));
    }
    if let Some(value) = config.get("maxTokens") {
        assert_positive_int(name, "maxTokens", value)?;
    }
    if let Some(value) = config.get("compactionRetries") {
        assert_non_negative_int(name, "compactionRetries", value)?;
    }
    if let Some(value) = config.get("maxOverflowRetries") {
        assert_non_negative_int(name, "maxOverflowRetries", value)?;
    }
    validate_summarization_pair(config, name)?;
    Ok(())
}

fn validate_summarization_pair(config: &serde_json::Value, name: &str) -> Result<(), ConfigError> {
    let provider = config.get("summarizationProvider");
    let model = config.get("summarizationModel");
    if let Some(value) = provider {
        if !value.is_string() {
            return Err(ConfigError::new(format!(
                "{name}.summarizationProvider must be a string"
            )));
        }
    }
    if let Some(value) = model {
        if !value.is_string() {
            return Err(ConfigError::new(format!(
                "{name}.summarizationModel must be a string"
            )));
        }
    }
    if provider.is_none() && model.is_none() {
        return Ok(());
    }
    let provider = provider.and_then(serde_json::Value::as_str);
    let model = model.and_then(serde_json::Value::as_str);
    match (provider, model) {
        (Some(provider), Some(model)) if provider.is_empty() == model.is_empty() => Ok(()),
        _ => Err(ConfigError::new(format!(
            "{name}: summarizationProvider and summarizationModel must be set together \
             as an empty or non-empty pair"
        ))),
    }
}

fn validate_keys(config: &serde_json::Value, keys: &[&str], name: &str) -> Result<(), ConfigError> {
    let Some(map) = config.as_object() else {
        return Ok(());
    };
    for key in map.keys() {
        if !keys.contains(&key.as_str()) {
            return Err(ConfigError::new(format!("{name}: unknown key \"{key}\"")));
        }
    }
    Ok(())
}

fn required_non_empty(
    name: &str,
    field: &str,
    value: Option<&serde_json::Value>,
) -> Result<String, ConfigError> {
    match value.and_then(serde_json::Value::as_str) {
        Some(text) if !text.is_empty() => Ok(text.to_string()),
        _ => Err(ConfigError::new(format!(
            "{name}.{field} must be a non-empty string"
        ))),
    }
}

fn assert_ratio(name: &str, field: &str, value: &serde_json::Value) -> Result<(), ConfigError> {
    let Some(number) = value.as_f64() else {
        return Err(ConfigError::new(format!(
            "{name}.{field} ({value}) must be a number in (0, 1]"
        )));
    };
    if !number.is_finite() || number <= 0.0 || number > 1.0 {
        return Err(ConfigError::new(format!(
            "{name}.{field} ({number}) must be a number in (0, 1]"
        )));
    }
    Ok(())
}

fn assert_positive_int(
    name: &str,
    field: &str,
    value: &serde_json::Value,
) -> Result<(), ConfigError> {
    match as_u64(value) {
        Some(number) if number > 0 => Ok(()),
        _ => Err(ConfigError::new(format!(
            "{name}.{field} ({value}) must be a positive integer"
        ))),
    }
}

fn assert_non_negative_int(
    name: &str,
    field: &str,
    value: &serde_json::Value,
) -> Result<(), ConfigError> {
    match as_u64(value) {
        Some(_) => Ok(()),
        None => Err(ConfigError::new(format!(
            "{name}.{field} ({value}) must be a non-negative integer"
        ))),
    }
}

fn as_u64(value: &serde_json::Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|n| u64::try_from(n).ok()))
}

fn f64_or(value: &serde_json::Value, key: &str, default: f64) -> f64 {
    value
        .get(key)
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(default)
}

fn f64_opt(value: &serde_json::Value, key: &str) -> Option<f64> {
    value.get(key).and_then(serde_json::Value::as_f64)
}

fn u64_or(value: &serde_json::Value, key: &str, default: u64) -> u64 {
    value.get(key).and_then(as_u64).unwrap_or(default)
}

fn u64_opt(value: &serde_json::Value, key: &str) -> Option<u64> {
    value.get(key).and_then(as_u64)
}

fn u32_or(value: &serde_json::Value, key: &str, default: u32) -> u32 {
    u64_or(value, key, u64::from(default)) as u32
}

fn u32_opt(value: &serde_json::Value, key: &str) -> Option<u32> {
    u64_opt(value, key).map(|n| n as u32)
}

fn string_or(value: &serde_json::Value, key: &str, default: &str) -> String {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or(default)
        .to_string()
}

fn string_opt(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::resolve_config;
    use serde_json::json;

    #[test]
    fn unknown_key_fails_load() {
        let err = resolve_config(&json!({"density": 3})).unwrap_err();
        assert!(err.to_string().contains("unknown key"));
    }

    #[test]
    fn defaults_match_typescript() {
        let config = resolve_config(&json!({})).unwrap();
        assert_eq!(config.threshold_ratio, 0.8);
        assert_eq!(config.retain_ratio, 0.16);
        assert_eq!(config.max_tokens, 8192);
        assert_eq!(config.compaction_retries, 1);
        assert_eq!(config.max_overflow_retries, 1);
        assert!(config.auto);
    }
}
