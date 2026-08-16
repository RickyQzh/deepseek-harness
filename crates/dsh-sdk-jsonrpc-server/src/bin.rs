//! `dsh-jsonrpc-agent` binary. Stdout is JSON-RPC frames only; diagnostics go to stderr.

use dsh_agent::{register_execution_plugins, register_spine_plugins};
use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
use dsh_kernel::Context;
use dsh_sdk_jsonrpc_server::{
    HarnessSdkJsonRpcServer, MINIMAL_YAML, SDK_JSONRPC_SERVER_SERVICE, register,
};
use tokio::io::AsyncWriteExt;

fn load_yaml() -> Result<String, String> {
    match std::env::var("DSH_CORDIS_CONFIG") {
        Ok(path) if !path.is_empty() => std::fs::read_to_string(&path)
            .map_err(|error| format!("DSH_CORDIS_CONFIG file not found: {path}: {error}")),
        _ => Ok(MINIMAL_YAML.to_string()),
    }
}

/// Leave persist env as-is when `DSH_SESSION_ROOT` or `DSH_HOME` is set and non-empty.
/// Otherwise set `DSH_HOME` to `$HOME/.dsh` for the process lifetime (not restored).
///
/// # Errors
///
/// When `DSH_SESSION_ROOT`, `DSH_HOME`, and `HOME` are all missing or empty.
fn ensure_persist_env() -> Result<(), String> {
    if let Ok(root) = std::env::var("DSH_SESSION_ROOT") {
        if !root.is_empty() {
            return Ok(());
        }
    }
    if let Ok(home) = std::env::var("DSH_HOME") {
        if !home.is_empty() {
            return Ok(());
        }
    }
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => {
            // SAFETY: persist has not mounted yet, so these keys have no concurrent readers; not restored.
            unsafe {
                std::env::set_var("DSH_HOME", std::path::PathBuf::from(home).join(".dsh"));
            }
            Ok(())
        }
        _ => Err("DSH_SESSION_ROOT, DSH_HOME, or HOME must be set for JSONL persistence".into()),
    }
}

#[tokio::main]
async fn main() {
    let yaml = match load_yaml() {
        Ok(yaml) => yaml,
        Err(message) => {
            eprintln!("dsh: {message}");
            std::process::exit(1);
        }
    };
    let ctx = Context::new();
    let mut registry = PluginRegistry::new();
    register_spine_plugins(&mut registry);
    register_execution_plugins(&mut registry);
    register(&mut registry);
    if let Err(message) = ensure_persist_env() {
        eprintln!("dsh: {message}");
        std::process::exit(1);
    }
    if let Err(error) = boot_yaml(&ctx, &yaml, &[], &registry, &process_interpolate_env()).await {
        eprintln!("dsh: {error}");
        std::process::exit(1);
    }
    let Some(server) = ctx.get::<HarnessSdkJsonRpcServer>(SDK_JSONRPC_SERVER_SERVICE) else {
        eprintln!("dsh: sdk-jsonrpc-server must be mounted");
        std::process::exit(1);
    };
    let _ = server.serve().await;
    let mut out = tokio::io::stdout();
    let _ = out.flush().await;
}
