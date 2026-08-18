//! `window.__DSH_BOOT__` graph types and index.html injection.

use serde::Serialize;
use sha2::{Digest, Sha256};

/// One composed client entry the host injects into the SPA (a graph row).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WebBootEntry {
    id: String,
    url: String,
    rev: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    inject: Option<Vec<String>>,
    #[serde(skip_serializing_if = "is_false")]
    immediately: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl WebBootEntry {
    /// Build one graph row. `url` is `/plugins/<id>/client.js?rev=<hex>`.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        url: impl Into<String>,
        rev: impl Into<String>,
        inject: Option<Vec<String>>,
        immediately: bool,
    ) -> Self {
        Self {
            id: id.into(),
            url: url.into(),
            rev: rev.into(),
            inject,
            immediately,
        }
    }

    /// Entry name (package name or scanned directory name).
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Bundle URL, `/plugins/<id>/client.js?rev=<hex>`.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Lowercase hex SHA-256 of the bundle bytes (or the test-supplied stand-in).
    #[must_use]
    pub fn rev(&self) -> &str {
        &self.rev
    }

    /// Optional package-name dependency edges from `dsh.client.inject`.
    #[must_use]
    pub fn inject(&self) -> Option<&[String]> {
        self.inject.as_deref()
    }

    /// Stage-one prefetch mark; `dsh.client.immediately` defaults to false.
    #[must_use]
    pub fn immediately(&self) -> bool {
        self.immediately
    }
}

/// Composed client entry graph injected as `window.__DSH_BOOT__`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WebBootGraph {
    rev: String,
    entries: Vec<WebBootEntry>,
}

impl WebBootGraph {
    /// Empty graph. `rev` is SHA-256 of the empty concatenation.
    #[must_use]
    pub fn empty() -> Self {
        Self::from_entries(Vec::new())
    }

    /// Graph whose `rev` is SHA-256 of UTF-8 `id` then `rev` for entries sorted by `id`.
    #[must_use]
    pub fn from_entries(entries: Vec<WebBootEntry>) -> Self {
        let rev = graph_rev(&entries);
        Self { rev, entries }
    }

    /// Lowercase hex SHA-256 over the sorted `id`/`rev` concatenation.
    #[must_use]
    pub fn rev(&self) -> &str {
        &self.rev
    }

    /// Composed rows. Order is the order passed to [`WebBootGraph::from_entries`].
    #[must_use]
    pub fn entries(&self) -> &[WebBootEntry] {
        &self.entries
    }
}

/// Lowercase hex SHA-256 of `bytes`.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    for &byte in digest.as_slice() {
        hex.push(DIGITS[(byte >> 4) as usize] as char);
        hex.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    hex
}

fn graph_rev(entries: &[WebBootEntry]) -> String {
    let mut sorted: Vec<&WebBootEntry> = entries.iter().collect();
    sorted.sort_by(|left, right| left.id.cmp(&right.id));
    let mut concat = String::new();
    for entry in sorted {
        concat.push_str(&entry.id);
        concat.push_str(&entry.rev);
    }
    sha256_hex(concat.as_bytes())
}

/// Insert `window.__DSH_BOOT__ = {…};` immediately before `</head>` (any case), or prepend.
///
/// After JSON serialize, every `<` is replaced with `\u003c`.
#[must_use]
pub fn inject_boot_manifest(html: &str, graph: &WebBootGraph) -> String {
    let json = serde_json::to_string(graph).expect("WebBootGraph is always serializable");
    let json = json.replace('<', r"\u003c");
    let script = format!("<script>window.__DSH_BOOT__ = {json}</script>");
    match find_ignore_ascii_case(html, "</head>") {
        Some(at) => {
            let mut out = String::with_capacity(html.len() + script.len());
            out.push_str(&html[..at]);
            out.push_str(&script);
            out.push_str(&html[at..]);
            out
        }
        None => {
            let mut out = String::with_capacity(html.len() + script.len());
            out.push_str(&script);
            out.push_str(html);
            out
        }
    }
}

fn find_ignore_ascii_case(haystack: &str, needle: &str) -> Option<usize> {
    let haystack_bytes = haystack.as_bytes();
    let needle_bytes = needle.as_bytes();
    if needle_bytes.is_empty() || haystack_bytes.len() < needle_bytes.len() {
        return None;
    }
    let last = haystack_bytes.len() - needle_bytes.len();
    let mut index = 0;
    while index <= last {
        if haystack_bytes[index..index + needle_bytes.len()].eq_ignore_ascii_case(needle_bytes) {
            return Some(index);
        }
        index += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{WebBootEntry, WebBootGraph, inject_boot_manifest};

    #[test]
    fn boot_escapes_left_angle_in_json() {
        let graph = WebBootGraph::from_entries(vec![WebBootEntry::new(
            "evil<id",
            "/plugins/evil/client.js?rev=1",
            "1",
            None,
            false,
        )]);
        let out = inject_boot_manifest("<html><head></head></html>", &graph);
        assert!(out.contains("\\u003c"), "{out}");
        assert!(!out.contains("evil<id"), "{out}");
    }
}
