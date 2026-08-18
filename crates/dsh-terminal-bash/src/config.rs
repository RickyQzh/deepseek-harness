//! Validated YAML configuration for `@deepseek-ai/dsh-terminal-bash`.

use serde_json::{Map, Value};

const CONFIG_KEYS: &[&str] = &[
    "backendType",
    "shellPath",
    "shellArgs",
    "rows",
    "cols",
    "scrollbackLines",
    "scrollbackMaxBytes",
    "maxReadBytes",
    "pollIntervalMs",
    "exactProbeAfterMs",
    "idleSilenceMs",
    "handoffGraceMs",
    "timeoutMs",
    "disposeGraceMs",
];

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// Plugin configuration after defaults and validation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedConfig {
    pub(crate) backend_type: String,
    pub(crate) shell_path: String,
    pub(crate) shell_args: Vec<String>,
    pub(crate) rows: u16,
    pub(crate) cols: u16,
    pub(crate) scrollback_lines: usize,
    pub(crate) scrollback_max_bytes: usize,
    pub(crate) max_read_bytes: usize,
    pub(crate) poll_interval_ms: u64,
    /// Delay after send start before `input_waiting` may settle `stdin_read`.
    pub(crate) exact_probe_after_ms: u64,
    pub(crate) idle_silence_ms: u64,
    pub(crate) handoff_grace_ms: u64,
    pub(crate) timeout_ms: u64,
    pub(crate) dispose_grace_ms: u64,
}

/// Parse YAML/JSON plugin config, apply defaults, and validate.
///
/// Unknown keys fail. Numeric fields must be positive safe integers.
pub(crate) fn parse_config(value: &Value) -> Result<ResolvedConfig, String> {
    let map = match value {
        Value::Null => Map::new(),
        Value::Object(map) => {
            for key in map.keys() {
                if !CONFIG_KEYS.contains(&key.as_str()) {
                    return Err(format!("TerminalBashConfig: unknown key \"{key}\""));
                }
            }
            map.clone()
        }
        _ => return Err("TerminalBashConfig: config must be an object".into()),
    };
    let backend_type = optional_string(&map, "backendType")?.unwrap_or_else(|| "shell".into());
    let shell_path = optional_string(&map, "shellPath")?.unwrap_or_else(|| "/bin/bash".into());
    let shell_args = match map.get("shellArgs") {
        None => vec![
            "--noprofile".to_string(),
            "--norc".to_string(),
            "-i".to_string(),
        ],
        Some(Value::Array(items)) => {
            let mut args = Vec::with_capacity(items.len());
            for item in items {
                match item.as_str() {
                    Some(text) => args.push(text.to_string()),
                    None => {
                        return Err("terminal-bash: shellArgs must be an array of strings".into());
                    }
                }
            }
            args
        }
        Some(_) => return Err("terminal-bash: shellArgs must be an array of strings".into()),
    };
    let rows = optional_positive(&map, "rows")?.unwrap_or(40);
    let cols = optional_positive(&map, "cols")?.unwrap_or(160);
    let scrollback_lines = optional_positive(&map, "scrollbackLines")?.unwrap_or(10_000);
    let scrollback_max_bytes = optional_positive(&map, "scrollbackMaxBytes")?.unwrap_or(4_194_304);
    let max_read_bytes = optional_positive(&map, "maxReadBytes")?.unwrap_or(262_144);
    let poll_interval_ms = optional_positive(&map, "pollIntervalMs")?.unwrap_or(50);
    let exact_probe_after_ms = optional_positive(&map, "exactProbeAfterMs")?.unwrap_or(150);
    let idle_silence_ms = optional_positive(&map, "idleSilenceMs")?.unwrap_or(3_000);
    let handoff_grace_ms = optional_positive(&map, "handoffGraceMs")?.unwrap_or(500);
    let timeout_ms = optional_positive(&map, "timeoutMs")?.unwrap_or(30_000);
    let dispose_grace_ms = optional_positive(&map, "disposeGraceMs")?.unwrap_or(3_000);
    let config = ResolvedConfig {
        backend_type,
        shell_path,
        shell_args,
        rows: u16_field(rows, "rows")?,
        cols: u16_field(cols, "cols")?,
        scrollback_lines: usize_field(scrollback_lines, "scrollbackLines")?,
        scrollback_max_bytes: usize_field(scrollback_max_bytes, "scrollbackMaxBytes")?,
        max_read_bytes: usize_field(max_read_bytes, "maxReadBytes")?,
        poll_interval_ms,
        exact_probe_after_ms,
        idle_silence_ms,
        handoff_grace_ms,
        timeout_ms,
        dispose_grace_ms,
    };
    validate_config(&config)?;
    Ok(config)
}

fn validate_config(config: &ResolvedConfig) -> Result<(), String> {
    if config.backend_type.is_empty() {
        return Err("terminal-bash: backendType must be non-empty".into());
    }
    if config.shell_path.is_empty() {
        return Err("terminal-bash: shellPath must be non-empty".into());
    }
    if config.max_read_bytes as u64 > config.scrollback_max_bytes as u64 {
        return Err("terminal-bash: maxReadBytes must not exceed scrollbackMaxBytes".into());
    }
    if config.handoff_grace_ms < config.poll_interval_ms {
        return Err(
            "terminal-bash: handoffGraceMs must be at least pollIntervalMs so one readiness poll runs inside the grace window"
                .into(),
        );
    }
    Ok(())
}

fn optional_string(map: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!("terminal-bash: {key} must be a string")),
    }
}

fn optional_positive(map: &Map<String, Value>, key: &str) -> Result<Option<u64>, String> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => Ok(Some(positive_safe_int(value, key)?)),
    }
}

fn positive_safe_int(value: &Value, name: &str) -> Result<u64, String> {
    let invalid = || format!("terminal-bash: {name} must be a positive safe integer");
    let number = match value.as_u64() {
        Some(n) => n,
        None => match value.as_i64() {
            Some(n) if n > 0 => n as u64,
            _ => return Err(invalid()),
        },
    };
    if number == 0 || number > MAX_SAFE_INTEGER {
        return Err(invalid());
    }
    Ok(number)
}

fn u16_field(value: u64, name: &str) -> Result<u16, String> {
    u16::try_from(value)
        .map_err(|_| format!("terminal-bash: {name} must be a positive safe integer"))
}

fn usize_field(value: u64, name: &str) -> Result<usize, String> {
    usize::try_from(value)
        .map_err(|_| format!("terminal-bash: {name} must be a positive safe integer"))
}

#[cfg(test)]
mod tests {
    use super::parse_config;
    use serde_json::json;

    #[test]
    fn config_rejects_handoff_shorter_than_poll() {
        let err = parse_config(&json!({
            "handoffGraceMs": 9,
            "pollIntervalMs": 10,
        }))
        .expect_err("handoff shorter than poll");
        assert_eq!(
            err,
            "terminal-bash: handoffGraceMs must be at least pollIntervalMs so one readiness poll runs inside the grace window"
        );

        let err = parse_config(&json!({
            "maxReadBytes": 100,
            "scrollbackMaxBytes": 50,
        }))
        .expect_err("read cap above retention");
        assert_eq!(
            err,
            "terminal-bash: maxReadBytes must not exceed scrollbackMaxBytes"
        );
    }
}
