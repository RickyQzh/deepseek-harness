//! Host YAML plugins: webserver, frontend-static, client-modules, and web-app.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use dsh_agent::AgentRegistry;
use dsh_boot::{
    PLUGIN_CLIENT_MODULES, PLUGIN_FRONTEND_STATIC, PLUGIN_HOST_WEBSERVER, PLUGIN_WEB_APP,
    PluginRegistry, PluginSetup,
};
use dsh_commands::CommandRegistry;
use dsh_credentials::LayeredCredentials;
use dsh_kernel::KernelError;
use dsh_session_persist::JsonlSessionStore;
use dsh_settings::SettingsService;
use dsh_skill::SkillRegistry;
use dsh_workspace::WorkspaceRegistry;
use serde_json::Value;

use crate::lookup::AgentLookup;
use crate::{GuiHandler, GuiServices, HostBind, HostPaths, HostState, serve};

/// Bundled Phase 7 web composition. Mock LLM, approval `ask`, no `!!js`.
pub const WEB_YAML: &str = include_str!("../web.cordis.yml");

#[derive(Clone)]
enum IoTarget {
    Stdout,
    Buffer(Arc<Mutex<String>>),
}

/// stdout used by `web-app`. Tests install a capture via `webIo`.
#[derive(Clone)]
pub struct WebIo {
    stdout: IoTarget,
}

impl WebIo {
    /// Real process stdout.
    #[must_use]
    pub fn stdio() -> Self {
        Self {
            stdout: IoTarget::Stdout,
        }
    }

    /// In-memory capture.
    #[must_use]
    pub fn capture() -> Self {
        Self {
            stdout: IoTarget::Buffer(Arc::new(Mutex::new(String::new()))),
        }
    }

    /// Write to the configured stdout.
    pub fn write_stdout(&self, chunk: &str) {
        match &self.stdout {
            IoTarget::Stdout => {
                let mut out = std::io::stdout();
                let _ = out.write_all(chunk.as_bytes());
                let _ = out.flush();
            }
            IoTarget::Buffer(buf) => buf.lock().expect("webIo").push_str(chunk),
        }
    }

    /// Captured stdout, or empty for stdio.
    #[must_use]
    pub fn stdout(&self) -> String {
        match &self.stdout {
            IoTarget::Stdout => String::new(),
            IoTarget::Buffer(buf) => buf.lock().expect("webIo").clone(),
        }
    }

    /// Take captured stdout, or empty for stdio.
    #[must_use]
    pub fn take_stdout(&self) -> String {
        match &self.stdout {
            IoTarget::Stdout => String::new(),
            IoTarget::Buffer(buf) => std::mem::take(&mut *buf.lock().expect("webIo")),
        }
    }
}

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

fn reject_unknown_keys(config: &Value, keys: &[&str], name: &str) -> Result<(), KernelError> {
    match config {
        Value::Null => Ok(()),
        Value::Object(map) => {
            for key in map.keys() {
                if !keys.contains(&key.as_str()) {
                    return Err(setup_err(format!("{name}: unknown key \"{key}\"")));
                }
            }
            Ok(())
        }
        _ => Err(setup_err(format!("{name}: config must be an object"))),
    }
}

fn required_path(config: &Value, key: &str, name: &str) -> Result<PathBuf, KernelError> {
    match config.get(key) {
        Some(Value::String(path)) if !path.is_empty() => Ok(PathBuf::from(path)),
        Some(_) => Err(setup_err(format!("{name}.{key} must be a string"))),
        None => Err(setup_err(format!("{name}.{key} is required"))),
    }
}

fn host_bind_from_config(config: &Value) -> Result<HostBind, KernelError> {
    reject_unknown_keys(config, &["host", "port"], "WebserverConfig")?;
    let host = match config.get("host") {
        Some(Value::String(host)) if !host.is_empty() => host.clone(),
        Some(_) => return Err(setup_err("WebserverConfig.host must be a string")),
        None => return Err(setup_err("WebserverConfig.host is required")),
    };
    let port = match config.get("port") {
        Some(value) => {
            let Some(number) = value.as_u64() else {
                return Err(setup_err("WebserverConfig.port must be an integer"));
            };
            u16::try_from(number).map_err(|_| setup_err("WebserverConfig.port must fit in u16"))?
        }
        None => return Err(setup_err("WebserverConfig.port is required")),
    };
    HostBind::new(host, port).map_err(|error| setup_err(error.to_string()))
}

fn existing_dir(config: &Value, key: &str, name: &str) -> Result<PathBuf, KernelError> {
    let path = required_path(config, key, name)?;
    if !path.is_dir() {
        return Err(setup_err(format!(
            "{name}.{key} is not a directory: {}",
            path.display()
        )));
    }
    Ok(path)
}

fn print_url_from_config(config: &Value) -> Result<bool, KernelError> {
    reject_unknown_keys(config, &["printUrl"], "WebAppConfig")?;
    match config.get("printUrl") {
        None | Some(Value::Null) => Ok(true),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(setup_err("WebAppConfig.printUrl must be a boolean")),
    }
}

fn register_webserver(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            let bind = host_bind_from_config(&config)?;
            ctx.provide("hostBind", bind)
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_HOST_WEBSERVER, setup);
}

fn register_frontend_static(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            reject_unknown_keys(&config, &["dist"], "FrontendStaticConfig")?;
            let dist = existing_dir(&config, "dist", "FrontendStaticConfig")?;
            ctx.provide("webDist", dist)
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_FRONTEND_STATIC, setup);
}

fn register_client_modules(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            reject_unknown_keys(&config, &["dir"], "ClientModulesConfig")?;
            let dir = existing_dir(&config, "dir", "ClientModulesConfig")?;
            ctx.provide("clientPackages", dir)
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_CLIENT_MODULES, setup);
}

fn register_web_app(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            let print_url = print_url_from_config(&config)?;
            let bind = ctx.inject::<HostBind>("hostBind").await?;
            let dist = ctx.inject::<PathBuf>("webDist").await?;
            let client_packages = ctx.inject::<PathBuf>("clientPackages").await?;
            let agents = ctx.inject::<AgentRegistry>("agents").await?;
            let sessions = ctx.inject::<JsonlSessionStore>("sessions").await?;
            let mut services = GuiServices::new();
            if let Some(value) = ctx.get::<WorkspaceRegistry>("workspaces") {
                services = services.workspaces(value);
            }
            if let Some(value) = ctx.get::<SettingsService>("settings") {
                services = services.settings(value);
            }
            if let Some(value) = ctx.get::<CommandRegistry>("commands") {
                services = services.commands(value);
            }
            if let Some(value) = ctx.get::<LayeredCredentials>("credentials") {
                services = services.credentials(value);
            }
            if let Some(value) = ctx.get::<SkillRegistry>("skills") {
                services = services.skills(value);
            }
            let lookup = AgentLookup::new(agents, sessions);
            let handler = GuiHandler::new(lookup, services);
            let paths = HostPaths::new((*dist).clone(), (*client_packages).clone());
            let state = HostState::new((*bind).clone(), paths, Vec::new(), handler)
                .map_err(|error| setup_err(error.to_string()))?;
            let host = serve(state)
                .await
                .map_err(|error| setup_err(error.to_string()))?;
            if print_url {
                let io = ctx
                    .get::<WebIo>("webIo")
                    .map(|arc| (*arc).clone())
                    .unwrap_or_else(WebIo::stdio);
                io.write_stdout(&format!(
                    "dsh web: http://127.0.0.1:{}\n",
                    host.local_addr().port()
                ));
            }
            ctx.provide("listeningHost", host)
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_WEB_APP, setup);
}

/// Register workspace, settings, commands, and the four host web plugins.
///
/// Does not register spine, execution, or base plugins.
pub fn register_host_plugins(registry: &mut PluginRegistry) {
    dsh_workspace::plugin::register(registry);
    dsh_settings::plugin::register(registry);
    dsh_commands::plugin::register(registry);
    register_webserver(registry);
    register_frontend_static(registry);
    register_client_modules(registry);
    register_web_app(registry);
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use dsh_agent::{register_execution_plugins, register_spine_plugins};
    use dsh_base::register_base_plugins;
    use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
    use dsh_kernel::Context;

    use super::register_host_plugins;
    use crate::{ListeningHost, WebIo};

    fn test_temp_dir(prefix: &str) -> PathBuf {
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

    fn yaml_quoted(path: &Path) -> String {
        serde_json::to_string(&path.to_string_lossy()).expect("path json")
    }

    fn test_web_yaml(
        dist: &Path,
        client_packages: &Path,
        sessions: &Path,
        workspaces: &Path,
        settings: &Path,
    ) -> String {
        let mut yaml = crate::WEB_YAML.to_string();
        yaml = yaml.replace("port: 3080", "port: 0");
        yaml = yaml.replace(
            "- name: '@deepseek-ai/dsh-session-persistence-jsonl'",
            &format!(
                "- name: '@deepseek-ai/dsh-session-persistence-jsonl'\n  config:\n    root: {}",
                yaml_quoted(sessions)
            ),
        );
        yaml = yaml.replace(
            "- name: '@deepseek-ai/dsh-workspace'",
            &format!(
                "- name: '@deepseek-ai/dsh-workspace'\n  config:\n    path: {}",
                yaml_quoted(workspaces)
            ),
        );
        yaml = yaml.replace(
            "- name: '@deepseek-ai/dsh-settings'",
            &format!(
                "- name: '@deepseek-ai/dsh-settings'\n  config:\n    dir: {}",
                yaml_quoted(settings)
            ),
        );
        yaml = yaml.replace(
            "- name: '@deepseek-ai/dsh-host-frontend-static'",
            &format!(
                "- name: '@deepseek-ai/dsh-host-frontend-static'\n  config:\n    dist: {}",
                yaml_quoted(dist)
            ),
        );
        yaml = yaml.replace(
            "- name: '@deepseek-ai/dsh-client-modules'",
            &format!(
                "- name: '@deepseek-ai/dsh-client-modules'\n  config:\n    dir: {}",
                yaml_quoted(client_packages)
            ),
        );
        yaml
    }

    #[test]
    fn web_yaml_rejects_js_tag_substring() {
        assert!(!crate::WEB_YAML.contains("!!js"));
        assert!(!crate::WEB_YAML.contains("headless-auto-approve"));
        assert!(!crate::WEB_YAML.contains("headless-runner"));
        assert!(!crate::WEB_YAML.contains("headless-startup"));
        assert!(!crate::WEB_YAML.contains("sdk-jsonrpc-server"));
    }

    #[tokio::test]
    async fn web_app_prints_ready_url_and_serves_index() {
        let root = test_temp_dir("dsh-web-app");
        let dist = root.join("dist");
        std::fs::create_dir_all(&dist).unwrap();
        std::fs::write(
            dist.join("index.html"),
            "<html><head></head><body>ok</body></html>",
        )
        .unwrap();
        let client_packages = root.join("client");
        std::fs::create_dir_all(&client_packages).unwrap();
        let sessions = root.join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        let settings = root.join("settings");
        std::fs::create_dir_all(&settings).unwrap();
        let workspaces = root.join("workspaces.json");

        let yaml = test_web_yaml(&dist, &client_packages, &sessions, &workspaces, &settings);
        let ctx = Context::new();
        let io = WebIo::capture();
        ctx.provide("webIo", io.clone()).expect("webIo");
        let mut registry = PluginRegistry::new();
        register_spine_plugins(&mut registry);
        register_execution_plugins(&mut registry);
        register_base_plugins(&mut registry);
        register_host_plugins(&mut registry);
        boot_yaml(&ctx, &yaml, &[], &registry, &process_interpolate_env())
            .await
            .expect("boot web");
        let stdout = io.take_stdout();
        assert!(stdout.contains("dsh web: http://127.0.0.1:"), "{stdout:?}");
        let url = stdout
            .lines()
            .find(|line| line.contains("dsh web: http://127.0.0.1:"))
            .expect("ready line")
            .trim()
            .strip_prefix("dsh web: ")
            .expect("ready prefix");
        let response = reqwest::Client::new().get(url).send().await.unwrap();
        assert_eq!(response.status(), 200);
        let host = ctx
            .get::<ListeningHost>("listeningHost")
            .expect("listeningHost");
        host.shutdown().await;
    }
}
