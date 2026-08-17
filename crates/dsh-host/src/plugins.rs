//! `/plugins` bundle helper and `packages/client/*/package.json` `dsh.client` scan.

use std::path::{Path, PathBuf};

use crate::boot::{WebBootEntry, WebBootGraph, sha256_hex};
use crate::static_files::{StaticResponse, resolve_under_root};

/// Scan or bundle failure. Malformed `dsh` / `dsh.client` and missing web bundles fail loud.
#[derive(Debug, thiserror::Error)]
pub enum HostError {
    /// Built `lib/client.js` is missing for a `dsh.client.platform == "web"` package.
    #[error(
        "client bundle not found; run pnpm run build before launch: package {package_name} path {path}"
    )]
    ClientBundleNotFound {
        /// Package id from `package.json` `name`, or the directory name when `name` is absent.
        package_name: String,
        /// Path that was read (`{package_dir}/lib/client.js`).
        path: String,
    },
    /// `package.json` or nested `dsh` / `dsh.client` is not the declared object/fields.
    #[error("{0}")]
    Malformed(String),
    /// Filesystem failure while reading a client package tree.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl HostError {
    fn malformed(message: impl Into<String>) -> Self {
        Self::Malformed(message.into())
    }

    fn client_bundle_not_found(package_name: impl Into<String>, path: impl Into<String>) -> Self {
        Self::ClientBundleNotFound {
            package_name: package_name.into(),
            path: path.into(),
        }
    }
}

/// Serve `{root}/{package_name}/lib/client.js`. Missing is 404. Successful bodies are `Cache-Control: no-cache`.
#[must_use]
pub fn serve_plugin_js(root: &Path, package_name: &str) -> StaticResponse {
    serve_plugin_file(root, package_name, false)
}

/// Serve `{root}/{package_name}/lib/client.js.map` with the same 404 / `no-cache` rules as the bundle.
#[must_use]
pub fn serve_plugin_source_map(root: &Path, package_name: &str) -> StaticResponse {
    serve_plugin_file(root, package_name, true)
}

fn serve_plugin_file(root: &Path, package_name: &str, source_map: bool) -> StaticResponse {
    let Some(js_path) = plugin_js_path(root, package_name) else {
        return StaticResponse::forbidden();
    };
    let path = if source_map {
        let mut map_path = js_path.into_os_string();
        map_path.push(".map");
        PathBuf::from(map_path)
    } else {
        js_path
    };
    match std::fs::read(&path) {
        Ok(body) => {
            let content_type = if source_map {
                "application/json"
            } else {
                "text/javascript"
            };
            StaticResponse::bytes(content_type, Some("no-cache"), body)
        }
        Err(_) => StaticResponse::not_found(),
    }
}

fn plugin_js_path(root: &Path, package_name: &str) -> Option<PathBuf> {
    let pkg_dir = resolve_under_root(root, package_name)?;
    if pkg_dir == root {
        return None;
    }
    Some(pkg_dir.join("lib").join("client.js"))
}

/// Read each `{packages_client_dir}/*/package.json` and compose the web boot graph.
///
/// The declaration is the nested object `dsh.client`, not a top-level `"dsh.client"` key. Packages with no `dsh` object are skipped. `platform == "web"` requires `{package}/lib/client.js` or fails with a message containing `pnpm run build` / `client bundle not found`.
pub fn scan_client_packages(packages_client_dir: &Path) -> Result<WebBootGraph, HostError> {
    let mut entries = Vec::new();
    let mut dirs = Vec::new();
    for child in std::fs::read_dir(packages_client_dir)? {
        let child = child?;
        let path = child.path();
        if path.is_dir() {
            dirs.push(path);
        }
    }
    dirs.sort();
    for dir in dirs {
        if let Some(entry) = scan_one_package(&dir)? {
            entries.push(entry);
        }
    }
    Ok(WebBootGraph::from_entries(entries))
}

fn scan_one_package(dir: &Path) -> Result<Option<WebBootEntry>, HostError> {
    let pkg_path = dir.join("package.json");
    let text = match std::fs::read_to_string(&pkg_path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let value: serde_json::Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(error) => {
            return Err(HostError::malformed(format!(
                "{}: {error}",
                pkg_path.display()
            )));
        }
    };
    let Some(dsh) = value.get("dsh") else {
        return Ok(None);
    };
    if !dsh.is_object() {
        return Err(HostError::malformed(format!(
            "{}: dsh must be an object",
            pkg_path.display()
        )));
    }
    let Some(client) = dsh.get("client") else {
        return Ok(None);
    };
    if !client.is_object() {
        return Err(HostError::malformed(format!(
            "{}: dsh.client must be an object",
            pkg_path.display()
        )));
    }
    let platform = match client.get("platform") {
        Some(serde_json::Value::String(platform)) => platform.as_str(),
        Some(_) | None => {
            return Err(HostError::malformed(format!(
                "{}: dsh.client.platform must be a string",
                pkg_path.display()
            )));
        }
    };
    if platform != "web" {
        return Ok(None);
    }
    let inject = match client.get("inject") {
        None => None,
        Some(serde_json::Value::Array(items)) => {
            let mut names = Vec::with_capacity(items.len());
            for item in items {
                match item.as_str() {
                    Some(name) => names.push(name.to_string()),
                    None => {
                        return Err(HostError::malformed(format!(
                            "{}: dsh.client.inject must be a string array",
                            pkg_path.display()
                        )));
                    }
                }
            }
            Some(names)
        }
        Some(_) => {
            return Err(HostError::malformed(format!(
                "{}: dsh.client.inject must be a string array",
                pkg_path.display()
            )));
        }
    };
    let immediately = match client.get("immediately") {
        None => false,
        Some(serde_json::Value::Bool(value)) => *value,
        Some(_) => {
            return Err(HostError::malformed(format!(
                "{}: dsh.client.immediately must be a boolean",
                pkg_path.display()
            )));
        }
    };
    let id = match value.get("name") {
        Some(serde_json::Value::String(name)) if !name.is_empty() => name.clone(),
        Some(_) => {
            return Err(HostError::malformed(format!(
                "{}: name must be a string",
                pkg_path.display()
            )));
        }
        None => match dir.file_name() {
            Some(name) => name.to_string_lossy().into_owned(),
            None => {
                return Err(HostError::malformed(format!(
                    "{}: package directory has no name",
                    dir.display()
                )));
            }
        },
    };
    let js_path = dir.join("lib").join("client.js");
    let bytes = match std::fs::read(&js_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(HostError::client_bundle_not_found(
                id,
                js_path.display().to_string(),
            ));
        }
        Err(error) => return Err(error.into()),
    };
    let rev = sha256_hex(&bytes);
    let url = format!("/plugins/{id}/client.js?rev={rev}");
    Ok(Some(WebBootEntry::new(id, url, rev, inject, immediately)))
}

#[cfg(test)]
mod tests {
    use super::{scan_client_packages, serve_plugin_js};

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
    fn plugin_js_is_no_cache() {
        let root = test_temp_dir("plug");
        let pkg = root.join("dsh-client-runtime");
        std::fs::create_dir_all(pkg.join("lib")).unwrap();
        std::fs::write(pkg.join("lib/client.js"), "module.exports=1").unwrap();
        let response = serve_plugin_js(&root, "dsh-client-runtime");
        assert_eq!(response.status(), 200);
        assert_eq!(response.content_type(), "text/javascript");
        assert_eq!(response.cache_control(), Some("no-cache"));
    }

    #[test]
    fn scan_fails_loud_when_client_js_missing() {
        let root = test_temp_dir("scan");
        let pkg = root.join("ui-layout");
        std::fs::create_dir_all(&pkg).unwrap();
        // Real packages use nested package.json key dsh.client (see packages/client/ui-layout/package.json).
        std::fs::write(
            pkg.join("package.json"),
            r#"{"name":"@deepseek-ai/dsh-client-ui-layout","dsh":{"client":{"platform":"web"}}}"#,
        )
        .unwrap();
        let err = scan_client_packages(&root).unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains("pnpm run build") || text.contains("client bundle not found"),
            "{text}"
        );
    }
}
