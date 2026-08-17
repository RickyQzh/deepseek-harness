//! Kernel plugin `@deepseek-ai/dsh-mcp-client`.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};

use dsh_boot::{PluginRegistry, PluginSetup};
use dsh_kernel::KernelError;
use dsh_subprocess::EnvEntry;
use dsh_tools::ToolRuntime;
use serde_json::{Map, Value};
use tokio::process::Child;
use tokio::sync::Mutex as AsyncMutex;

use crate::spawn_stdio;
use crate::sync::swap_generation;

const CONFIG_KEYS: &[&str] = &[
    "transport",
    "serverName",
    "command",
    "args",
    "env",
    "cwd",
    "toolCallTimeoutMs",
    "failOnStartupError",
    "reconnect",
];
const DEFAULT_TOOL_CALL_TIMEOUT_MS: u64 = 60_000;
const HTTP_UNSUPPORTED: &str = "mcp-client: streamable-http is not supported in Phase 8 item 2";

static RESERVATIONS: LazyLock<Mutex<HashMap<usize, HashSet<String>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

struct StdioPluginConfig {
    server_name: String,
    command: String,
    args: Vec<String>,
    env: Vec<EnvEntry>,
    cwd: String,
    tool_call_timeout_ms: u64,
    fail_on_startup_error: bool,
}

/// Register YAML `@deepseek-ai/dsh-mcp-client`.
///
/// Parses stdio config, reserves `serverName` on the injected `tools` runtime,
/// spawns the child, initializes, and registers tools under `mcp__` public names.
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
            let parsed = parse_stdio_config(&config).map_err(setup_err)?;
            let tools = ctx.inject::<Mutex<ToolRuntime>>("tools").await?;
            let key = Arc::as_ptr(&tools) as usize;
            reserve_server_name(key, &parsed.server_name).map_err(setup_err)?;
            let owned_names = Arc::new(Mutex::new(Vec::<String>::new()));
            let child_slot: Arc<AsyncMutex<Option<Child>>> = Arc::new(AsyncMutex::new(None));
            let release_name = parsed.server_name.clone();
            let release_tools = Arc::clone(&tools);
            let release_owned = Arc::clone(&owned_names);
            let release_child = Arc::clone(&child_slot);
            ctx.effect(move || async move {
                release_server_name(key, &release_name);
                let names = release_owned
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone();
                {
                    let mut runtime = release_tools
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    for name in names {
                        runtime.unregister(&name);
                    }
                }
                let mut slot = release_child.lock().await;
                if let Some(mut child) = slot.take() {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                }
            })?;
            connect_and_sync(parsed, tools, owned_names, child_slot).await
        })
    });
    registry.register(dsh_boot::PLUGIN_MCP_CLIENT, setup);
}

/// Register MCP client plugins by calling [`register`].
///
/// # Parameters
///
/// * `registry` - Closed plugin-name registry.
///
/// # Returns
///
/// Nothing. Same as [`register`].
pub fn register_mcp_plugins(registry: &mut PluginRegistry) {
    register(registry);
}

fn parse_stdio_config(config: &Value) -> Result<StdioPluginConfig, String> {
    let map = match config {
        Value::Object(map) => map,
        _ => return Err("mcp-client: config must be an object".into()),
    };
    for key in map.keys() {
        if !CONFIG_KEYS.contains(&key.as_str()) {
            return Err(format!("mcp-client: unknown config key \"{key}\""));
        }
    }
    match map.get("transport").and_then(Value::as_str) {
        Some("streamable-http") => return Err(HTTP_UNSUPPORTED.into()),
        Some("stdio") => {}
        Some(other) => {
            return Err(format!(
                "mcp-client: transport \"{other}\" is not supported"
            ));
        }
        None => return Err("mcp-client: transport is required".into()),
    }
    match map.get("reconnect") {
        None => {}
        Some(Value::Object(_)) => {}
        Some(_) => return Err("mcp-client: reconnect must be an object".into()),
    }
    let server_name = required_string(map, "serverName")?;
    if !valid_server_name(&server_name) {
        return Err("mcp-client: serverName must match ^[A-Za-z0-9_-]{1,32}$".into());
    }
    let command = required_string(map, "command")?;
    let args = optional_string_array(map, "args")?;
    let env = optional_env(map)?;
    let cwd = match map.get("cwd") {
        None => String::new(),
        Some(Value::String(value)) => value.clone(),
        Some(_) => return Err("mcp-client: cwd must be a string".into()),
    };
    let tool_call_timeout_ms = match map.get("toolCallTimeoutMs") {
        None => DEFAULT_TOOL_CALL_TIMEOUT_MS,
        Some(value) => parse_timeout_ms(value)?,
    };
    let fail_on_startup_error = match map.get("failOnStartupError") {
        None => false,
        Some(Value::Bool(value)) => *value,
        Some(_) => return Err("mcp-client: failOnStartupError must be a boolean".into()),
    };
    Ok(StdioPluginConfig {
        server_name,
        command,
        args,
        env,
        cwd,
        tool_call_timeout_ms,
        fail_on_startup_error,
    })
}

fn required_string(map: &Map<String, Value>, key: &str) -> Result<String, String> {
    match map.get(key) {
        Some(Value::String(value)) if !value.is_empty() => Ok(value.clone()),
        Some(Value::String(_)) => Err(format!("mcp-client: {key} must be a non-empty string")),
        _ => Err(format!("mcp-client: {key} must be a string")),
    }
}

fn optional_string_array(map: &Map<String, Value>, key: &str) -> Result<Vec<String>, String> {
    match map.get(key) {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => {
            let mut args = Vec::with_capacity(items.len());
            for item in items {
                match item.as_str() {
                    Some(value) => args.push(value.to_string()),
                    None => return Err(format!("mcp-client: {key} must be an array of strings")),
                }
            }
            Ok(args)
        }
        Some(_) => Err(format!("mcp-client: {key} must be an array of strings")),
    }
}

fn optional_env(map: &Map<String, Value>) -> Result<Vec<EnvEntry>, String> {
    match map.get("env") {
        None => Ok(Vec::new()),
        Some(Value::Object(env)) => {
            let mut entries = Vec::with_capacity(env.len());
            for (key, value) in env {
                match value.as_str() {
                    Some(text) => entries.push(EnvEntry {
                        key: key.clone(),
                        value: Some(text.to_string()),
                    }),
                    None => return Err("mcp-client: env values must be strings".into()),
                }
            }
            Ok(entries)
        }
        Some(_) => Err("mcp-client: env must be an object".into()),
    }
}

fn parse_timeout_ms(value: &Value) -> Result<u64, String> {
    let Some(number) = value.as_number() else {
        return Err("mcp-client: toolCallTimeoutMs must be a number".into());
    };
    if let Some(ms) = number.as_u64() {
        return Ok(ms);
    }
    match number.as_i64() {
        Some(ms) if ms >= 0 => Ok(ms as u64),
        _ => Err("mcp-client: toolCallTimeoutMs must be a non-negative integer".into()),
    }
}

fn valid_server_name(name: &str) -> bool {
    let len = name.len();
    if !(1..=32).contains(&len) {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn reserve_server_name(key: usize, name: &str) -> Result<(), String> {
    let mut table = RESERVATIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let names = table.entry(key).or_default();
    if names.contains(name) {
        return Err(format!(
            "mcp-client: serverName \"{name}\" is already in use by another mcp-client instance — pick a unique serverName in cordis.yml"
        ));
    }
    names.insert(name.to_string());
    Ok(())
}

fn release_server_name(key: usize, name: &str) {
    let mut table = RESERVATIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let empty = match table.get_mut(&key) {
        Some(names) => {
            names.remove(name);
            names.is_empty()
        }
        None => false,
    };
    if empty {
        table.remove(&key);
    }
}

fn startup_failed(
    fail_on_startup_error: bool,
    server_name: &str,
    err: impl std::fmt::Display,
) -> Result<(), KernelError> {
    if fail_on_startup_error {
        return Err(setup_err(format!(
            "mcp-client({server_name}): initial connection or tool synchronization failed: {err}"
        )));
    }
    eprintln!("mcp-client({server_name}): {err}");
    Ok(())
}

async fn connect_and_sync(
    parsed: StdioPluginConfig,
    tools: Arc<Mutex<ToolRuntime>>,
    owned_names: Arc<Mutex<Vec<String>>>,
    child_slot: Arc<AsyncMutex<Option<Child>>>,
) -> Result<(), KernelError> {
    let cwd = if parsed.cwd.is_empty() {
        None
    } else {
        Some(parsed.cwd.as_str())
    };
    let (mut session, child) = match spawn_stdio(&parsed.command, &parsed.args, &parsed.env, cwd) {
        Ok(pair) => pair,
        Err(err) => {
            return startup_failed(parsed.fail_on_startup_error, &parsed.server_name, err);
        }
    };
    *child_slot.lock().await = Some(child);
    session.set_tool_call_timeout_ms(parsed.tool_call_timeout_ms);
    if let Err(err) = session.initialize().await {
        return startup_failed(parsed.fail_on_startup_error, &parsed.server_name, err);
    }
    let drafts = match session.list_tools().await {
        Ok(drafts) => drafts,
        Err(err) => {
            return startup_failed(parsed.fail_on_startup_error, &parsed.server_name, err);
        }
    };
    let names = {
        let mut runtime = tools
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        swap_generation(
            &session,
            &mut runtime,
            &parsed.server_name,
            Vec::new(),
            drafts,
        )
    };
    match names {
        Ok(names) => {
            *owned_names
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = names;
            Ok(())
        }
        Err(err) => startup_failed(parsed.fail_on_startup_error, &parsed.server_name, err),
    }
}
