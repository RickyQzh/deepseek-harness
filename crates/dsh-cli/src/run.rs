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
    ensure_persist_env()?;
    boot_yaml(&ctx, &yaml, &patches, &registry, &process_interpolate_env())
        .await
        .map_err(|error| error.to_string())?;
    rx.await
        .map_err(|_| "headless runner exited without appExit".to_string())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::ensure_persist_env;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

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

    #[test]
    fn ensure_persist_env_defaults_dsh_home_from_home() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let home = test_temp_dir("dsh-cli-home");
        let previous_root = std::env::var("DSH_SESSION_ROOT").ok();
        let previous_dsh_home = std::env::var("DSH_HOME").ok();
        let previous_home = std::env::var("HOME").ok();
        unsafe {
            std::env::remove_var("DSH_SESSION_ROOT");
            std::env::remove_var("DSH_HOME");
            std::env::set_var("HOME", home.as_os_str());
        }
        let result = ensure_persist_env();
        let observed = std::env::var("DSH_HOME").ok();
        match previous_root {
            Some(value) => unsafe { std::env::set_var("DSH_SESSION_ROOT", value) },
            None => unsafe { std::env::remove_var("DSH_SESSION_ROOT") },
        }
        match previous_dsh_home {
            Some(value) => unsafe { std::env::set_var("DSH_HOME", value) },
            None => unsafe { std::env::remove_var("DSH_HOME") },
        }
        match previous_home {
            Some(value) => unsafe { std::env::set_var("HOME", value) },
            None => unsafe { std::env::remove_var("HOME") },
        }
        result.unwrap();
        assert_eq!(
            std::path::PathBuf::from(observed.expect("DSH_HOME")),
            home.join(".dsh")
        );
    }

    #[test]
    fn ensure_persist_env_leaves_dsh_home_when_session_root_set() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let home = test_temp_dir("dsh-cli-root");
        let previous_root = std::env::var("DSH_SESSION_ROOT").ok();
        let previous_dsh_home = std::env::var("DSH_HOME").ok();
        let previous_home = std::env::var("HOME").ok();
        unsafe {
            std::env::set_var("DSH_SESSION_ROOT", home.join("sessions").as_os_str());
            std::env::remove_var("DSH_HOME");
            std::env::set_var("HOME", home.as_os_str());
        }
        let result = ensure_persist_env();
        let observed = std::env::var("DSH_HOME").ok();
        match previous_root {
            Some(value) => unsafe { std::env::set_var("DSH_SESSION_ROOT", value) },
            None => unsafe { std::env::remove_var("DSH_SESSION_ROOT") },
        }
        match previous_dsh_home {
            Some(value) => unsafe { std::env::set_var("DSH_HOME", value) },
            None => unsafe { std::env::remove_var("DSH_HOME") },
        }
        match previous_home {
            Some(value) => unsafe { std::env::set_var("HOME", value) },
            None => unsafe { std::env::remove_var("HOME") },
        }
        result.unwrap();
        assert_eq!(observed, None);
    }

    #[test]
    fn ensure_persist_env_leaves_existing_dsh_home() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let home = test_temp_dir("dsh-cli-keep-home");
        let existing = home.join("existing-dsh");
        let previous_root = std::env::var("DSH_SESSION_ROOT").ok();
        let previous_dsh_home = std::env::var("DSH_HOME").ok();
        let previous_home = std::env::var("HOME").ok();
        unsafe {
            std::env::remove_var("DSH_SESSION_ROOT");
            std::env::set_var("DSH_HOME", existing.as_os_str());
            std::env::set_var("HOME", home.as_os_str());
        }
        let result = ensure_persist_env();
        let observed = std::env::var("DSH_HOME").ok();
        match previous_root {
            Some(value) => unsafe { std::env::set_var("DSH_SESSION_ROOT", value) },
            None => unsafe { std::env::remove_var("DSH_SESSION_ROOT") },
        }
        match previous_dsh_home {
            Some(value) => unsafe { std::env::set_var("DSH_HOME", value) },
            None => unsafe { std::env::remove_var("DSH_HOME") },
        }
        match previous_home {
            Some(value) => unsafe { std::env::set_var("HOME", value) },
            None => unsafe { std::env::remove_var("HOME") },
        }
        result.unwrap();
        assert_eq!(
            std::path::PathBuf::from(observed.expect("DSH_HOME")),
            existing
        );
    }

    #[test]
    fn ensure_persist_env_errors_when_persist_vars_and_home_missing() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let previous_root = std::env::var("DSH_SESSION_ROOT").ok();
        let previous_dsh_home = std::env::var("DSH_HOME").ok();
        let previous_home = std::env::var("HOME").ok();
        unsafe {
            std::env::remove_var("DSH_SESSION_ROOT");
            std::env::remove_var("DSH_HOME");
            std::env::remove_var("HOME");
        }
        let result = ensure_persist_env();
        match previous_root {
            Some(value) => unsafe { std::env::set_var("DSH_SESSION_ROOT", value) },
            None => unsafe { std::env::remove_var("DSH_SESSION_ROOT") },
        }
        match previous_dsh_home {
            Some(value) => unsafe { std::env::set_var("DSH_HOME", value) },
            None => unsafe { std::env::remove_var("DSH_HOME") },
        }
        match previous_home {
            Some(value) => unsafe { std::env::set_var("HOME", value) },
            None => unsafe { std::env::remove_var("HOME") },
        }
        let err = result.expect_err("missing persist env");
        assert!(err.contains("DSH_SESSION_ROOT, DSH_HOME, or HOME"), "{err}");
    }
}
