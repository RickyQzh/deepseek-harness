//! Boot headless, web, or ACP compositions.

use std::path::{Path, PathBuf};

use dsh_agent::{register_execution_plugins, register_spine_plugins};
use dsh_base::register_base_plugins;
use dsh_boot::{PluginRegistry, boot_yaml, mount_entries, process_interpolate_env};
use dsh_compose::{Entry, apply_entry_patches, parse_yaml_entries, parse_yaml_patches};
use dsh_headless::{AppExit, CmdlineArgs, HeadlessIo, MINIMAL_YAML, register_headless_plugins};
use dsh_host::{WebIo, register_host_plugins};
use dsh_kernel::Context;

use crate::parse::{AcpLaunch, ParsedCli, WebLaunch, parse_cli};

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
        ParsedCli::Web(launch) => match run_web(launch).await {
            Ok(code) => code,
            Err(message) => {
                eprintln!("dsh: {message}");
                1
            }
        },
        ParsedCli::Acp(launch) => match run_acp(launch).await {
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
    register_base_plugins(&mut registry);
    register_headless_plugins(&mut registry);
    ensure_persist_env()?;
    boot_yaml(&ctx, &yaml, &patches, &registry, &process_interpolate_env())
        .await
        .map_err(|error| error.to_string())?;
    rx.await
        .map_err(|_| "headless runner exited without appExit".to_string())
}

fn load_web_yaml() -> Result<String, String> {
    match std::env::var("DSH_CORDIS_CONFIG") {
        Ok(path) if !path.is_empty() => std::fs::read_to_string(&path)
            .map_err(|error| format!("DSH_CORDIS_CONFIG file not found: {path}: {error}")),
        _ => Ok(dsh_host::WEB_YAML.to_string()),
    }
}

fn load_acp_yaml() -> Result<String, String> {
    match std::env::var("DSH_CORDIS_CONFIG") {
        Ok(path) if !path.is_empty() => std::fs::read_to_string(&path)
            .map_err(|error| format!("DSH_CORDIS_CONFIG file not found: {path}: {error}")),
        _ => Ok(dsh_acp::ACP_YAML.to_string()),
    }
}

fn yaml_quoted(value: &str) -> Result<String, String> {
    serde_json::to_string(value).map_err(|error| error.to_string())
}

fn resolve_web_dist(flag: Option<PathBuf>) -> Result<PathBuf, String> {
    let path = match flag {
        Some(path) => path,
        None => match std::env::var("DSH_WEB_DIST") {
            Ok(path) if !path.is_empty() => PathBuf::from(path),
            _ => {
                let cwd = std::env::current_dir().map_err(|error| error.to_string())?;
                cwd.join("apps/web/dist")
            }
        },
    };
    if path.is_dir() {
        Ok(path)
    } else {
        Err(format!("dist is not a directory: {}", path.display()))
    }
}

fn resolve_client_packages() -> Result<PathBuf, String> {
    match std::env::var("DSH_CLIENT_PACKAGES") {
        Ok(path) if !path.is_empty() => {
            let path = PathBuf::from(path);
            if path.is_dir() {
                Ok(path)
            } else {
                Err(format!(
                    "DSH_CLIENT_PACKAGES is not a directory: {}",
                    path.display()
                ))
            }
        }
        _ => {
            let dir = std::env::temp_dir().join(format!(
                "dsh-client-packages-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|error| error.to_string())?
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
            Ok(dir)
        }
    }
}

fn web_overlay_yaml(
    port: u16,
    dist: &Path,
    client_packages: &Path,
    trusted_hosts: &[String],
) -> Result<String, String> {
    let dist = yaml_quoted(&dist.to_string_lossy())?;
    let dir = yaml_quoted(&client_packages.to_string_lossy())?;
    let mut trusted = String::new();
    if !trusted_hosts.is_empty() {
        trusted.push_str("\n    trustedHosts:");
        for host in trusted_hosts {
            let quoted = yaml_quoted(host)?;
            trusted.push_str("\n      - ");
            trusted.push_str(&quoted);
        }
    }
    Ok(format!(
        "- name: '@deepseek-ai/dsh-host-webserver'\n  config:\n    host: \"127.0.0.1\"\n    port: {port}{trusted}\n- name: '@deepseek-ai/dsh-host-frontend-static'\n  config:\n    dist: {dist}\n- name: '@deepseek-ai/dsh-client-modules'\n  config:\n    dir: {dir}\n"
    ))
}

fn find_entry_mut<'a>(entries: &'a mut [Entry], name: &str) -> Option<&'a mut Entry> {
    for entry in entries.iter_mut() {
        if entry.name == name {
            return Some(entry);
        }
        if let Some(found) = find_entry_mut(&mut entry.children, name) {
            return Some(found);
        }
    }
    None
}

fn overlay_by_name(entries: &mut [Entry], overlay: &[Entry]) -> Result<(), String> {
    for over in overlay {
        match find_entry_mut(entries, &over.name) {
            Some(target) => target.config = over.config.clone(),
            None => {
                return Err(format!(
                    "overlay plugin {} is not in the composition",
                    over.name
                ));
            }
        }
    }
    Ok(())
}

async fn boot_web_yaml(
    ctx: &Context,
    yaml: &str,
    user_patches: &[String],
    overlay: &str,
    registry: &PluginRegistry,
) -> Result<(), String> {
    let mut entries = parse_yaml_entries(yaml).map_err(|error| error.to_string())?;
    for patch_src in user_patches {
        let patch_list = parse_yaml_patches(patch_src).map_err(|error| error.to_string())?;
        entries = apply_entry_patches(&entries, &patch_list).map_err(|error| error.to_string())?;
    }
    let overlay_entries = parse_yaml_entries(overlay).map_err(|error| error.to_string())?;
    overlay_by_name(&mut entries, &overlay_entries)?;
    mount_entries(ctx, &entries, registry, &process_interpolate_env())
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

async fn run_web(launch: WebLaunch) -> Result<i32, String> {
    ensure_persist_env()?;
    let yaml = load_web_yaml()?;
    let dist = resolve_web_dist(launch.dist)?;
    let client_packages = resolve_client_packages()?;
    let user_patches = load_patches(&launch.patches)?;
    let overlay = web_overlay_yaml(launch.port, &dist, &client_packages, &launch.trusted_hosts)?;
    let ctx = Context::new();
    let (exit, rx) = AppExit::pair();
    ctx.provide("appExit", exit)
        .map_err(|error| error.to_string())?;
    ctx.provide("cmdlineArgs", CmdlineArgs::new(Vec::new()))
        .map_err(|error| error.to_string())?;
    ctx.provide("webIo", WebIo::stdio())
        .map_err(|error| error.to_string())?;
    let mut registry = PluginRegistry::new();
    register_spine_plugins(&mut registry);
    register_execution_plugins(&mut registry);
    register_base_plugins(&mut registry);
    register_host_plugins(&mut registry);
    boot_web_yaml(&ctx, &yaml, &user_patches, &overlay, &registry).await?;
    rx.await
        .map_err(|_| "web host exited without appExit".to_string())
}

async fn run_acp(launch: AcpLaunch) -> Result<i32, String> {
    ensure_persist_env()?;
    let yaml = load_acp_yaml()?;
    let patches = load_patches(&launch.patches)?;
    let ctx = Context::new();
    let mut registry = PluginRegistry::new();
    register_spine_plugins(&mut registry);
    register_execution_plugins(&mut registry);
    register_base_plugins(&mut registry);
    dsh_acp::register_acp_plugins(&mut registry);
    boot_yaml(&ctx, &yaml, &patches, &registry, &process_interpolate_env())
        .await
        .map_err(|error| error.to_string())?;
    let bridge = ctx
        .inject::<dsh_acp::AcpBridge>(dsh_acp::ACP_SERVER_SERVICE)
        .await
        .map_err(|error| error.to_string())?;
    bridge.serve().await.map_err(|error| error.to_string())?;
    Ok(0)
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
