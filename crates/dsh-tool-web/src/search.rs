//! Model-facing `web_search`: schema, argument checks, source cap, and markdown formatting.
//!
//! Presentation intent for this tool is `generic` / kind `search`. [`dsh_tools::ToolDefinition`]
//! has no card field; that intent is documented here rather than stored on the definition.

use std::sync::Arc;

use dsh_session::ContentBlock;
use dsh_tools::{AbortFlag, ToolDefinition, ToolError, ToolRuntime};
use dsh_web::{WebRuntime, WebSearchRequest, WebSearchResult, WebSearchSource};
use serde_json::{Map, Value, json};

/// Default upper bound on returned sources (`searchMaxResults`).
pub const WEB_SEARCH_MAX_RESULTS: u32 = 8;

/// Structured `web_search` presentation meta matching the TypeScript `WebSearchMeta`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WebSearchMeta {
    /// Faithful structured sources, in result order.
    pub sources: Vec<WebSearchSource>,
    /// True when the search runtime cut the source list to honor the result cap.
    pub truncated: bool,
    /// Provider-generated answer text, when any.
    pub answer: Option<String>,
}

/// Reject a missing, non-string, empty, or whitespace-only `query`.
///
/// # Errors
///
/// `query must be a non-empty string`.
pub fn parse_search_args(args: &Value) -> Result<String, ToolError> {
    let query = args.get("query").and_then(Value::as_str).unwrap_or("");
    if query.trim().is_empty() {
        return Err(ToolError::Other("query must be a non-empty string".into()));
    }
    Ok(query.to_string())
}

/// Project a search result into replayable presentation meta.
#[must_use]
pub fn search_meta_from_result(result: &WebSearchResult) -> WebSearchMeta {
    WebSearchMeta {
        sources: result.sources.clone(),
        truncated: result.truncated,
        answer: result.content.clone().filter(|text| !text.is_empty()),
    }
}

fn source_label(url: &str, title: Option<&str>) -> String {
    if let Some(title) = title {
        if !title.is_empty() {
            return title.to_string();
        }
    }
    parse_hostname(url).unwrap_or_else(|| url.to_string())
}

fn parse_hostname(url: &str) -> Option<String> {
    let scheme_end = url.find("://")?;
    if scheme_end == 0 {
        return None;
    }
    let rest = &url[scheme_end + 3..];
    if rest.is_empty() {
        return None;
    }
    let hostport = match rest.find(['/', '?', '#']) {
        Some(index) => &rest[..index],
        None => rest,
    };
    let hostport = match hostport.rfind('@') {
        Some(index) => &hostport[index + 1..],
        None => hostport,
    };
    if hostport.is_empty() {
        return None;
    }
    if hostport.starts_with('[') {
        let end = hostport.find(']')?;
        if end <= 1 {
            return None;
        }
        return Some(hostport[1..end].to_string());
    }
    let host = match hostport.find(':') {
        Some(index) => &hostport[..index],
        None => hostport,
    };
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

/// Format a search result as one model-facing markdown block (TypeScript `formatSearchOutput`).
#[must_use]
pub fn format_search_output(result: &WebSearchResult) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(content) = &result.content {
        if !content.is_empty() {
            parts.push(content.clone());
        }
    }
    if !result.sources.is_empty() {
        let lines: Vec<String> = result
            .sources
            .iter()
            .map(|source| {
                let label = source_label(&source.url, source.title.as_deref());
                let mut meta: Vec<String> = Vec::new();
                if let Some(snippet) = &source.snippet {
                    if !snippet.is_empty() {
                        meta.push(snippet.clone());
                    }
                }
                if let Some(published) = &source.published_at {
                    if !published.is_empty() {
                        meta.push(format!("({published})"));
                    }
                }
                let suffix = if meta.is_empty() {
                    String::new()
                } else {
                    format!(" — {}", meta.join(" "))
                };
                format!("- [{label}]({}){suffix}", source.url)
            })
            .collect();
        parts.push(format!("Sources:\n{}", lines.join("\n")));
    } else {
        let content_empty = result
            .content
            .as_ref()
            .map(|content| content.is_empty())
            .unwrap_or(true);
        if content_empty {
            parts.push("No results found.".into());
        }
    }
    if result.truncated {
        parts.push(format!(
            "(Showing the first {} sources. Refine the query for more.)",
            result.sources.len()
        ));
    }
    parts.push("Cite the relevant URLs above as markdown links in your answer.".into());
    parts.join("\n\n")
}

fn source_to_json(source: &WebSearchSource) -> Value {
    let mut map = Map::new();
    map.insert("url".into(), json!(source.url));
    if let Some(title) = &source.title {
        if !title.is_empty() {
            map.insert("title".into(), json!(title));
        }
    }
    if let Some(snippet) = &source.snippet {
        if !snippet.is_empty() {
            map.insert("snippet".into(), json!(snippet));
        }
    }
    if let Some(published) = &source.published_at {
        if !published.is_empty() {
            map.insert("publishedAt".into(), json!(published));
        }
    }
    Value::Object(map)
}

fn result_to_json(result: &WebSearchResult) -> Value {
    let mut map = Map::new();
    if let Some(content) = &result.content {
        if !content.is_empty() {
            map.insert("content".into(), json!(content));
        }
    }
    map.insert(
        "sources".into(),
        Value::Array(result.sources.iter().map(source_to_json).collect()),
    );
    map.insert("truncated".into(), json!(result.truncated));
    Value::Object(map)
}

fn result_from_json(value: &Value) -> WebSearchResult {
    let content = value
        .get("content")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string);
    let sources = value
        .get("sources")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let url = item.get("url").and_then(Value::as_str)?;
                    Some(WebSearchSource {
                        url: url.to_string(),
                        title: item
                            .get("title")
                            .and_then(Value::as_str)
                            .filter(|text| !text.is_empty())
                            .map(str::to_string),
                        snippet: item
                            .get("snippet")
                            .and_then(Value::as_str)
                            .filter(|text| !text.is_empty())
                            .map(str::to_string),
                        published_at: item
                            .get("publishedAt")
                            .and_then(Value::as_str)
                            .filter(|text| !text.is_empty())
                            .map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let truncated = value
        .get("truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    WebSearchResult {
        content,
        sources,
        truncated,
    }
}

/// Register the `web_search` tool. Does not register `web_fetch`.
pub fn register_web_search_tool(runtime: &mut ToolRuntime, web: Arc<WebRuntime>, max_results: u32) {
    runtime.register(ToolDefinition {
        name: "web_search".into(),
        description: "Search the web for current information. Returns an optional summary answer and a list of source URLs.".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query."
                }
            },
            "required": ["query"]
        }),
        execute: Box::new(move |args, exec| {
            let web = Arc::clone(&web);
            Box::pin(async move { execute_web_search(web, args, exec.signal, max_results).await })
        }),
        render: Box::new(|_args, value| {
            vec![ContentBlock::Text {
                text: format_search_output(&result_from_json(value)),
            }]
        }),
        is_concurrency_safe: Some(Box::new(|_| true)),
    });
}

async fn execute_web_search(
    web: Arc<WebRuntime>,
    args: Value,
    signal: AbortFlag,
    max_results: u32,
) -> Result<Value, ToolError> {
    let query = parse_search_args(&args)?;
    let result = web
        .search(
            WebSearchRequest {
                query,
                max_results: Some(max_results),
            },
            signal,
        )
        .await
        .map_err(|error| ToolError::Coded {
            message: error.message,
            name: "WebError".into(),
            code: error.code,
        })?;
    Ok(result_to_json(&result))
}

#[cfg(test)]
mod tests {
    use super::{format_search_output, parse_search_args, search_meta_from_result};
    use dsh_web::{WebSearchResult, WebSearchSource};

    #[test]
    fn format_search_output_no_results() {
        assert!(
            format_search_output(&WebSearchResult {
                content: None,
                sources: vec![],
                truncated: false
            })
            .contains("No results found.")
        );
    }

    #[test]
    fn format_search_output_renders_titles_hostnames_snippets_and_cite() {
        let out = format_search_output(&WebSearchResult {
            content: Some("an answer".into()),
            truncated: false,
            sources: vec![
                WebSearchSource {
                    url: "https://a.test/x".into(),
                    title: Some("A".into()),
                    snippet: Some("about a".into()),
                    published_at: Some("2026-01-01".into()),
                },
                WebSearchSource {
                    url: "https://b.test/y".into(),
                    title: None,
                    snippet: None,
                    published_at: None,
                },
            ],
        });
        assert!(out.contains("an answer"));
        assert!(out.contains("[A](https://a.test/x) — about a (2026-01-01)"));
        assert!(out.contains("[b.test](https://b.test/y)"));
        assert!(out.contains("Cite the relevant URLs"));
        assert!(!out.contains("No results found."));
    }

    #[test]
    fn format_search_output_notes_truncation() {
        let out = format_search_output(&WebSearchResult {
            content: None,
            sources: vec![WebSearchSource {
                url: "https://a.test".into(),
                title: None,
                snippet: None,
                published_at: None,
            }],
            truncated: true,
        });
        assert!(out.contains("Showing the first 1 sources"));
    }

    #[test]
    fn empty_query_is_rejected() {
        let err = parse_search_args(&serde_json::json!({ "query": "   " })).unwrap_err();
        assert!(err.to_string().contains("query must be a non-empty string"));
    }

    #[test]
    fn search_meta_from_result_copies_sources_and_answer() {
        let result = WebSearchResult {
            content: Some("an answer".into()),
            truncated: true,
            sources: vec![WebSearchSource {
                url: "https://a.test".into(),
                title: Some("A".into()),
                snippet: None,
                published_at: None,
            }],
        };
        let meta = search_meta_from_result(&result);
        assert_eq!(meta.answer.as_deref(), Some("an answer"));
        assert!(meta.truncated);
        assert_eq!(meta.sources.len(), 1);
    }
}
