//! SPA dist helper: path traversal is 403; a miss falls back to injected `index.html`.

use std::path::{Component, Path, PathBuf};

use crate::boot::{WebBootGraph, inject_boot_manifest};

/// One static-file answer. The HTTP listener maps methods; this helper is GET/HEAD only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaticResponse {
    status: u16,
    content_type: &'static str,
    cache_control: Option<&'static str>,
    body: Vec<u8>,
}

impl StaticResponse {
    fn new(
        status: u16,
        content_type: &'static str,
        cache_control: Option<&'static str>,
        body: Vec<u8>,
    ) -> Self {
        Self {
            status,
            content_type,
            cache_control,
            body,
        }
    }

    pub(crate) fn forbidden() -> Self {
        Self::new(403, "application/octet-stream", None, Vec::new())
    }

    pub(crate) fn not_found() -> Self {
        Self::new(404, "application/octet-stream", None, Vec::new())
    }

    pub(crate) fn bytes(
        content_type: &'static str,
        cache_control: Option<&'static str>,
        body: Vec<u8>,
    ) -> Self {
        Self::new(200, content_type, cache_control, body)
    }

    /// HTTP status code.
    #[must_use]
    pub fn status(&self) -> u16 {
        self.status
    }

    /// MIME type, including `charset` when the SPA table specifies one.
    #[must_use]
    pub fn content_type(&self) -> &'static str {
        self.content_type
    }

    /// `Cache-Control` value when the helper sets one (plugin bundles use `no-cache`).
    #[must_use]
    pub fn cache_control(&self) -> Option<&'static str> {
        self.cache_control
    }

    /// Response body bytes. Empty for 403/404.
    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }
}

/// Serve one SPA path from `dist`. Path traversal (`..`, NUL) is 403.
///
/// A missing file or a directory is `index.html` 200 after `__DSH_BOOT__` injection.
#[must_use]
pub fn serve_spa(dist: &Path, url_path: &str, graph: &WebBootGraph) -> StaticResponse {
    let target = match resolve_under_root(dist, url_path) {
        Some(path) => path,
        None => return StaticResponse::forbidden(),
    };
    let index_path = dist.join("index.html");
    if target == dist || target == index_path {
        return serve_index(dist, graph);
    }
    match std::fs::read(&target) {
        Ok(body) => StaticResponse::bytes(mime_for(&target), None, body),
        Err(_) => serve_index(dist, graph),
    }
}

fn serve_index(dist: &Path, graph: &WebBootGraph) -> StaticResponse {
    let index_path = dist.join("index.html");
    match std::fs::read_to_string(&index_path) {
        Ok(html) => {
            let body = inject_boot_manifest(&html, graph).into_bytes();
            StaticResponse::bytes("text/html; charset=utf-8", None, body)
        }
        Err(_) => StaticResponse::not_found(),
    }
}

/// Join `url_path` under `root`. `None` means traversal (NUL, `..`, prefix, or absolute).
pub(crate) fn resolve_under_root(root: &Path, url_path: &str) -> Option<PathBuf> {
    if url_path.as_bytes().contains(&0) {
        return None;
    }
    let without_hash = match url_path.split_once('#') {
        Some((path, _)) => path,
        None => url_path,
    };
    let without_query = match without_hash.split_once('?') {
        Some((path, _)) => path,
        None => without_hash,
    };
    let trimmed = without_query.trim_start_matches('/');
    let mut relative = PathBuf::new();
    for component in Path::new(trimmed).components() {
        match component {
            Component::Normal(name) => relative.push(name),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(root.join(relative))
}

fn mime_for(path: &Path) -> &'static str {
    let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
    match ext {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript",
        "css" => "text/css",
        "svg" => "image/svg+xml",
        "json" | "webmanifest" | "map" => "application/json",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::serve_spa;
    use crate::boot::{WebBootEntry, WebBootGraph};

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
    fn traversal_is_forbidden() {
        let dist = test_temp_dir("spa");
        std::fs::write(
            dist.join("index.html"),
            "<html><head></head><body></body></html>",
        )
        .unwrap();
        let graph = WebBootGraph::empty();
        let response = serve_spa(&dist, "/../etc/passwd", &graph);
        assert_eq!(response.status(), 403);
    }

    #[test]
    fn missing_path_falls_back_to_injected_index() {
        let dist = test_temp_dir("spa-fb");
        std::fs::write(
            dist.join("index.html"),
            "<html><head></head><body>app</body></html>",
        )
        .unwrap();
        let graph = WebBootGraph::from_entries(vec![WebBootEntry::new(
            "dsh-client-runtime",
            "/plugins/dsh-client-runtime/client.js?rev=abc",
            "abc",
            None,
            true,
        )]);
        let response = serve_spa(&dist, "/session/xyz", &graph);
        assert_eq!(response.status(), 200);
        let body = String::from_utf8(response.body().to_vec()).unwrap();
        assert!(body.contains("window.__DSH_BOOT__"));
        assert!(body.contains("dsh-client-runtime"));
        let boot_at = body.find("window.__DSH_BOOT__").expect("boot script");
        let head_end = body.find("</head>").expect("head close");
        assert!(boot_at < head_end, "{body}");
    }
}
