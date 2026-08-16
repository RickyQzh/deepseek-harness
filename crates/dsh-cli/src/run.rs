//! Boot the headless composition and wait for `appExit`.

use dsh_agent::{register_execution_plugins, register_spine_plugins};
use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
use dsh_headless::{AppExit, CmdlineArgs, HeadlessIo, MINIMAL_YAML, register_headless_plugins};
use dsh_kernel::Context;

use crate::parse::{ParsedCli, parse_cli};

fn load_yaml() -> Result<String, String> {
    match std::env::var("DSH_CORDIS_CONFIG") {
        Ok(path) if !path.is_empty() => std::fs::read_to_string(&path)
            .map_err(|error| format!("DSH_CORDIS_CONFIG file not found: {path}: {error}")),
        _ => Ok(MINIMAL_YAML.to_string()),
    }
}

fn load_patches(paths: &[std::path::PathBuf]) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for path in paths {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("patch file not found: {}: {error}", path.display()))?;
        out.push(text);
    }
    Ok(out)
}

/// Parse argv, boot, wait for the runner's exit code.
pub async fn run_cli(args: Vec<String>) -> i32 {
    let parsed = match parse_cli(args) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprint!("{}", error.to_stderr_line());
            return error.exit_code();
        }
    };
    match parsed {
        ParsedCli::Headless(launch) => match run_headless(launch.task, launch.patches).await {
            Ok(code) => code,
            Err(message) => {
                eprintln!("dsh: {message}");
                1
            }
        },
    }
}

async fn run_headless(task: String, patch_paths: Vec<std::path::PathBuf>) -> Result<i32, String> {
    let yaml = load_yaml()?;
    let patches = load_patches(&patch_paths)?;
    let ctx = Context::new();
    let (exit, rx) = AppExit::pair();
    ctx.provide("appExit", exit)
        .map_err(|error| error.to_string())?;
    ctx.provide("cmdlineArgs", CmdlineArgs::new(vec![task]))
        .map_err(|error| error.to_string())?;
    ctx.provide("headlessIo", HeadlessIo::stdio())
        .map_err(|error| error.to_string())?;
    let mut registry = PluginRegistry::new();
    register_spine_plugins(&mut registry);
    register_execution_plugins(&mut registry);
    register_headless_plugins(&mut registry);
    boot_yaml(&ctx, &yaml, &patches, &registry, &process_interpolate_env())
        .await
        .map_err(|error| error.to_string())?;
    rx.await
        .map_err(|_| "headless runner exited without appExit".to_string())
}
