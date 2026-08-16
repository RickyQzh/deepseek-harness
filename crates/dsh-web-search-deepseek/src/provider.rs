//! DeepSeek Messages search: `web_search_20250305`, no redirect follow, rustls only.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use dsh_tools::AbortFlag;
use dsh_web::{
    WebError, WebSearchProvider, WebSearchRequest, WebSearchResult, WebSearchSource, WEB_ABORTED,
    WEB_PROVIDER_CREDENTIAL_MISSING, WEB_PROVIDER_ERROR,
};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde_json::{json, Value};

/// Stable id this provider registers under.
pub const DEEPSEEK_PROVIDER_ID: &str = "deepseek-official";
/// Default Anthropic-compatible Messages base; `/messages` is appended. Not `$DEEPSEEK_BASE_URL`.
pub const DEEPSEEK_DEFAULT_BASE_URL: &str = "https://api.deepseek.com/anthropic/v1";
/// Default Anthropic-format model name.
pub const DEEPSEEK_DEFAULT_MODEL: &str = "deepseek-v4-flash";
/// Default `anthropic-version` header value.
pub const DEEPSEEK_DEFAULT_API_VERSION: &str = "2023-06-01";
/// Default `max_tokens` for the Messages request.
pub const DEEPSEEK_DEFAULT_MAX_TOKENS: u32 = 4096;
/// Default `web_search` server-tool `max_uses`.
pub const DEEPSEEK_DEFAULT_MAX_USES: u32 = 5;

const USER_AGENT: &str = "deepseek-harness/0.0.1";
const SEARCH_BASE_URL_ENV: &str = "DEEPSEEK_SEARCH_BASE_URL";

/// Options for the next search. The plugin supplies a thunk so each call snapshots current config.
pub struct DeepSeekSearchProviderOptions {
    /// Literal API key; when non-empty it wins over [`Self::resolve_api_key`].
    pub api_key: Option<String>,
    /// Resolve the current DeepSeek API key for one search.
    pub resolve_api_key: Option<Arc<dyn Fn() -> Result<Option<String>, WebError> + Send + Sync>>,
    /// Credential reference named by missing-credential diagnostics.
    pub api_key_env: String,
    /// Endpoint base; `/messages` is appended.
    pub base_url: String,
    /// Anthropic-format model name.
    pub model: String,
    /// `anthropic-version` header value.
    pub api_version: String,
    /// Upper bound on generated tokens for the Messages request.
    pub max_tokens: u32,
    /// Maximum `web_search` server-tool uses per request.
    pub max_uses: u32,
}

/// Choose the Messages base: config `baseURL`, else `$DEEPSEEK_SEARCH_BASE_URL`, else the default.
///
/// Never reads `$DEEPSEEK_BASE_URL`.
#[must_use]
pub fn resolve_base_url(config_base_url: Option<&str>, search_env: Option<&str>) -> String {
    if let Some(url) = config_base_url {
        if !url.is_empty() {
            return url.to_string();
        }
    }
    if let Some(url) = search_env {
        if !url.is_empty() {
            return url.to_string();
        }
    }
    DEEPSEEK_DEFAULT_BASE_URL.to_string()
}

/// JSON body posted to `{baseURL}/messages` using default model, token, and use limits.
#[must_use]
pub fn deepseek_search_body(query: &str) -> Value {
    search_body(
        query,
        DEEPSEEK_DEFAULT_MODEL,
        DEEPSEEK_DEFAULT_MAX_TOKENS,
        DEEPSEEK_DEFAULT_MAX_USES,
    )
}

fn search_body(query: &str, model: &str, max_tokens: u32, max_uses: u32) -> Value {
    json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": [{
            "role": "user",
            "content": [{
                "type": "text",
                "text": format!("Perform a web search for the query: {query}")
            }]
        }],
        "tools": [{
            "type": "web_search_20250305",
            "name": "web_search",
            "max_uses": max_uses
        }]
    })
}

/// Map `url → cited_text` from every `text` block's `citations[]`. First occurrence wins.
#[must_use]
pub fn citation_snippets(blocks: &[Value]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for block in blocks {
        if block.get("type").and_then(Value::as_str) != Some("text") {
            continue;
        }
        let Some(citations) = block.get("citations").and_then(Value::as_array) else {
            continue;
        };
        for cite in citations {
            let url = cite.get("url").and_then(Value::as_str).unwrap_or("");
            let cited = cite.get("cited_text").and_then(Value::as_str).unwrap_or("");
            if url.is_empty() || cited.is_empty() || map.contains_key(url) {
                continue;
            }
            map.insert(url.to_string(), cited.to_string());
        }
    }
    map
}

/// Map a DeepSeek Anthropic Messages JSON body to a search result. `truncated` is always false.
///
/// Missing or JSON-null `web_search_tool_result.content` is an empty item list. A present
/// non-array `content` (Anthropic `web_search_tool_result_error` envelope) is unprocessable.
///
/// # Errors
///
/// [`WEB_PROVIDER_ERROR`] when the payload has no `web_search_tool_result` block, or when a
/// result block's `content` is present and not a JSON array.
pub fn map_anthropic_response(response: &Value) -> Result<WebSearchResult, WebError> {
    let blocks = response.get("content").and_then(Value::as_array);
    let Some(blocks) = blocks else {
        return Err(no_result_blocks());
    };
    let result_blocks: Vec<&Value> = blocks
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("web_search_tool_result"))
        .collect();
    if result_blocks.is_empty() {
        return Err(no_result_blocks());
    }
    let snippets = citation_snippets(blocks);
    let mut seen = HashSet::new();
    let mut sources = Vec::new();
    for block in result_blocks {
        let items = match block.get("content") {
            None | Some(Value::Null) => continue,
            Some(Value::Array(items)) => items,
            Some(_) => return Err(unprocessable_tool_result_content()),
        };
        for item in items {
            if item.get("type").and_then(Value::as_str) != Some("web_search_result") {
                continue;
            }
            let url = item.get("url").and_then(Value::as_str).unwrap_or("");
            if url.is_empty() || seen.contains(url) {
                continue;
            }
            seen.insert(url.to_string());
            let title = nonempty_opt(item.get("title").and_then(Value::as_str));
            let snippet = snippets.get(url).cloned().filter(|text| !text.is_empty());
            let published_at = nonempty_opt(item.get("page_age").and_then(Value::as_str));
            sources.push(WebSearchSource {
                url: url.to_string(),
                title,
                snippet,
                published_at,
            });
        }
    }
    Ok(WebSearchResult {
        content: None,
        sources,
        truncated: false,
    })
}

fn nonempty_opt(value: Option<&str>) -> Option<String> {
    value.filter(|text| !text.is_empty()).map(str::to_string)
}

fn no_result_blocks() -> WebError {
    WebError::new(
        "DeepSeek returned no web_search_tool_result blocks; the request may not have triggered native web search",
        WEB_PROVIDER_ERROR,
    )
}

fn unprocessable_tool_result_content() -> WebError {
    WebError::new(
        "DeepSeek returned an unprocessable response body: web_search_tool_result.content is not an array",
        WEB_PROVIDER_ERROR,
    )
}

fn aborted() -> WebError {
    WebError::new("DeepSeek search aborted", WEB_ABORTED)
}

fn is_parseable_url(value: &str) -> bool {
    reqwest::Url::parse(value).is_ok()
}

/// DeepSeek-backed search provider. The HTTP client uses rustls and does not follow redirects.
pub struct DeepSeekSearchProvider {
    resolve_options: Box<dyn Fn() -> DeepSeekSearchProviderOptions + Send + Sync>,
    client: reqwest::Client,
}

impl DeepSeekSearchProvider {
    /// Capture the per-search options thunk. The client is built with `redirect::Policy::none()` and rustls.
    #[must_use]
    pub fn new(
        resolve_options: impl Fn() -> DeepSeekSearchProviderOptions + Send + Sync + 'static,
    ) -> Self {
        Self {
            resolve_options: Box::new(resolve_options),
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .use_rustls_tls()
                .user_agent(USER_AGENT)
                .build()
                .expect("reqwest rustls client"),
        }
    }

    fn options(&self) -> DeepSeekSearchProviderOptions {
        (self.resolve_options)()
    }

    fn resolve_key(options: &DeepSeekSearchProviderOptions) -> Result<String, WebError> {
        if let Some(key) = &options.api_key {
            if !key.is_empty() {
                return Ok(key.clone());
            }
        }
        if let Some(resolve) = &options.resolve_api_key {
            match resolve() {
                Ok(Some(key)) => {
                    if !key.is_empty() {
                        return Ok(key);
                    }
                }
                Ok(None) => {}
                Err(error) => return Err(error),
            }
        }
        Err(WebError::new(
            format!(
                "DeepSeek search has no API key for \"{}\"; store it through the credentials service (the web Models page writes it), export it in the launching environment, or set a literal \"apiKey\" in the web-search-deepseek config",
                options.api_key_env
            ),
            WEB_PROVIDER_CREDENTIAL_MISSING,
        ))
    }

    async fn dispatch(
        &self,
        request: WebSearchRequest,
        signal: AbortFlag,
    ) -> Result<WebSearchResult, WebError> {
        if signal.is_aborted() {
            return Err(aborted());
        }
        let options = self.options();
        let api_key = Self::resolve_key(&options)?;
        if signal.is_aborted() {
            return Err(aborted());
        }
        let endpoint = format!("{}/messages", options.base_url);
        let body = search_body(
            &request.query,
            &options.model,
            options.max_tokens,
            options.max_uses,
        );
        let headers = request_headers(&api_key, &options.api_version)?;
        let send = self
            .client
            .post(&endpoint)
            .headers(headers)
            .json(&body)
            .send();
        let response = tokio::select! {
            _ = signal.cancelled() => return Err(aborted()),
            result = send => match result {
                Ok(response) => response,
                Err(error) => {
                    if signal.is_aborted() {
                        return Err(aborted());
                    }
                    return Err(WebError::new(
                        format!("DeepSeek search request failed: {error}"),
                        WEB_PROVIDER_ERROR,
                    ));
                }
            },
        };
        if signal.is_aborted() {
            return Err(aborted());
        }
        let status = response.status();
        if status.is_redirection() {
            return Err(WebError::new(
                format!("DeepSeek search request failed: HTTP {}", status.as_u16()),
                WEB_PROVIDER_ERROR,
            ));
        }
        if !status.is_success() {
            let message = http_error_message(response).await;
            if signal.is_aborted() {
                return Err(aborted());
            }
            return Err(WebError::new(message, WEB_PROVIDER_ERROR));
        }
        let payload = match response.json::<Value>().await {
            Ok(payload) => payload,
            Err(error) => {
                if signal.is_aborted() {
                    return Err(aborted());
                }
                return Err(WebError::new(
                    format!("DeepSeek returned an unprocessable response body: {error}"),
                    WEB_PROVIDER_ERROR,
                ));
            }
        };
        map_anthropic_response(&payload)
    }
}

fn request_headers(api_key: &str, api_version: &str) -> Result<HeaderMap, WebError> {
    let mut headers = HeaderMap::new();
    let key = HeaderValue::from_str(api_key).map_err(|_| {
        WebError::new(
            "DeepSeek request header value is not a valid HTTP header",
            WEB_PROVIDER_ERROR,
        )
    })?;
    headers.insert(HeaderName::from_static("x-api-key"), key.clone());
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {api_key}")).map_err(|_| {
            WebError::new(
                "DeepSeek request header value is not a valid HTTP header",
                WEB_PROVIDER_ERROR,
            )
        })?,
    );
    headers.insert(
        HeaderName::from_static("anthropic-version"),
        HeaderValue::from_str(api_version).map_err(|_| {
            WebError::new(
                "DeepSeek request header value is not a valid HTTP header",
                WEB_PROVIDER_ERROR,
            )
        })?,
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(
        HeaderName::from_static("user-agent"),
        HeaderValue::from_static(USER_AGENT),
    );
    Ok(headers)
}

async fn http_error_message(response: reqwest::Response) -> String {
    let status = response.status().as_u16();
    let default = format!("DeepSeek API error (HTTP {status})");
    let Ok(parsed) = response.json::<Value>().await else {
        return default;
    };
    let detail = match parsed.get("error") {
        Some(Value::String(text)) => Some(text.as_str()),
        Some(obj) => obj.get("message").and_then(Value::as_str),
        None => parsed.get("message").and_then(Value::as_str),
    };
    match detail {
        Some(text) if !text.is_empty() => text.to_string(),
        _ => default,
    }
}

impl WebSearchProvider for DeepSeekSearchProvider {
    fn id(&self) -> &str {
        DEEPSEEK_PROVIDER_ID
    }

    fn available(&self) -> bool {
        let options = self.options();
        let has_literal = options.api_key.as_ref().is_some_and(|key| !key.is_empty());
        let has_resolved = if has_literal {
            true
        } else {
            match &options.resolve_api_key {
                Some(resolve) => resolve().ok().flatten().is_some_and(|key| !key.is_empty()),
                None => false,
            }
        };
        has_resolved
            && is_parseable_url(&options.base_url)
            && options.max_tokens > 0
            && options.max_uses > 0
    }

    fn search<'a>(
        &'a self,
        request: WebSearchRequest,
        signal: AbortFlag,
    ) -> Pin<Box<dyn Future<Output = Result<WebSearchResult, WebError>> + Send + 'a>> {
        Box::pin(self.dispatch(request, signal))
    }
}

/// Read `$DEEPSEEK_SEARCH_BASE_URL` when present and non-empty.
#[must_use]
pub fn search_base_url_from_env() -> Option<String> {
    match std::env::var(SEARCH_BASE_URL_ENV) {
        Ok(value) if !value.is_empty() => Some(value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        citation_snippets, deepseek_search_body, map_anthropic_response, resolve_base_url,
        DeepSeekSearchProvider, DeepSeekSearchProviderOptions, DEEPSEEK_DEFAULT_API_VERSION,
        DEEPSEEK_DEFAULT_BASE_URL, DEEPSEEK_DEFAULT_MAX_TOKENS, DEEPSEEK_DEFAULT_MAX_USES,
        DEEPSEEK_DEFAULT_MODEL, DEEPSEEK_PROVIDER_ID,
    };
    use dsh_tools::AbortFlag;
    use dsh_web::{WebSearchProvider, WebSearchRequest, WEB_PROVIDER_ERROR};
    use serde_json::json;

    fn options_with_key() -> DeepSeekSearchProviderOptions {
        DeepSeekSearchProviderOptions {
            api_key: Some("ds-key".into()),
            resolve_api_key: None,
            api_key_env: "DEEPSEEK_API_KEY".into(),
            base_url: "https://api.deepseek.test/anthropic/v1".into(),
            model: DEEPSEEK_DEFAULT_MODEL.into(),
            api_version: DEEPSEEK_DEFAULT_API_VERSION.into(),
            max_tokens: DEEPSEEK_DEFAULT_MAX_TOKENS,
            max_uses: DEEPSEEK_DEFAULT_MAX_USES,
        }
    }

    fn search_fixture() -> serde_json::Value {
        json!({
            "content": [
                {
                    "type": "text",
                    "text": "Here is what I found.",
                    "citations": [{
                        "type": "web_search_result_location",
                        "url": "https://a.test",
                        "cited_text": "excerpt for A"
                    }]
                },
                {
                    "type": "web_search_tool_result",
                    "content": [
                        {
                            "type": "web_search_result",
                            "url": "https://a.test",
                            "title": "A",
                            "page_age": "2026-02-02"
                        },
                        {
                            "type": "web_search_result",
                            "url": "https://b.test",
                            "title": "B"
                        }
                    ]
                }
            ]
        })
    }

    #[test]
    fn deepseek_request_body_uses_web_search_20250305() {
        let body = deepseek_search_body("what is rust");
        assert_eq!(body["tools"][0]["type"], "web_search_20250305");
        assert_eq!(body["tools"][0]["name"], "web_search");
        assert_eq!(body["model"], DEEPSEEK_DEFAULT_MODEL);
        assert_eq!(body["max_tokens"], DEEPSEEK_DEFAULT_MAX_TOKENS);
        assert_eq!(
            body["messages"][0]["content"][0]["text"],
            "Perform a web search for the query: what is rust"
        );
    }

    #[test]
    fn map_anthropic_response_joins_citations_without_network() {
        let result = map_anthropic_response(&search_fixture()).unwrap();
        assert_eq!(result.sources.len(), 2);
        assert!(!result.truncated);
        assert!(result.content.is_none());
        assert_eq!(result.sources[0].url, "https://a.test");
        assert_eq!(result.sources[0].title.as_deref(), Some("A"));
        assert_eq!(result.sources[0].snippet.as_deref(), Some("excerpt for A"));
        assert_eq!(
            result.sources[0].published_at.as_deref(),
            Some("2026-02-02")
        );
        assert_eq!(result.sources[1].url, "https://b.test");
        assert_eq!(result.sources[1].title.as_deref(), Some("B"));
        assert!(result.sources[1].snippet.is_none());
    }

    #[test]
    fn map_anthropic_response_requires_result_blocks() {
        let err = map_anthropic_response(&json!({
            "content": [{ "type": "text", "text": "just prose, no search" }]
        }))
        .unwrap_err();
        assert_eq!(err.code, WEB_PROVIDER_ERROR);
    }

    #[test]
    fn map_anthropic_response_rejects_tool_result_error_envelope() {
        let err = map_anthropic_response(&json!({
            "content": [{
                "type": "web_search_tool_result",
                "content": {
                    "type": "web_search_tool_result_error",
                    "error_code": "max_uses_exceeded"
                }
            }]
        }))
        .unwrap_err();
        assert_eq!(err.code, WEB_PROVIDER_ERROR);
        assert!(
            err.message.contains("unprocessable response body"),
            "expected unprocessable-body diagnostic, got {:?}",
            err.message
        );
    }

    #[test]
    fn map_anthropic_response_treats_missing_or_null_content_as_empty_items() {
        let missing = map_anthropic_response(&json!({
            "content": [
                { "type": "web_search_tool_result" },
                {
                    "type": "web_search_tool_result",
                    "content": [{ "type": "web_search_result", "url": "https://a.test" }]
                }
            ]
        }))
        .unwrap();
        assert_eq!(missing.sources.len(), 1);
        assert_eq!(missing.sources[0].url, "https://a.test");

        let null_only = map_anthropic_response(&json!({
            "content": [{ "type": "web_search_tool_result", "content": null }]
        }))
        .unwrap();
        assert!(null_only.sources.is_empty());
    }

    #[test]
    fn citation_snippets_first_occurrence_wins() {
        let map = citation_snippets(&[
            json!({
                "type": "text",
                "citations": [
                    { "url": "https://a.test", "cited_text": "first" },
                    { "url": "https://a.test", "cited_text": "second" }
                ]
            }),
            json!({
                "type": "text",
                "citations": [{ "url": "https://b.test", "cited_text": "b text" }]
            }),
        ]);
        assert_eq!(map.get("https://a.test").map(String::as_str), Some("first"));
        assert_eq!(
            map.get("https://b.test").map(String::as_str),
            Some("b text")
        );
    }

    #[test]
    fn resolve_base_url_ignores_chat_completions_base() {
        assert_eq!(resolve_base_url(None, None), DEEPSEEK_DEFAULT_BASE_URL);
        assert_eq!(
            resolve_base_url(
                Some("https://gateway.internal/anthropic/v1"),
                Some("https://env")
            ),
            "https://gateway.internal/anthropic/v1"
        );
        assert_eq!(
            resolve_base_url(None, Some("https://search.env/anthropic/v1")),
            "https://search.env/anthropic/v1"
        );
        assert_eq!(resolve_base_url(Some(""), None), DEEPSEEK_DEFAULT_BASE_URL);
    }

    #[test]
    fn available_requires_nonempty_key_and_parseable_url() {
        let provider = DeepSeekSearchProvider::new(options_with_key);
        assert!(provider.available());
        assert_eq!(provider.id(), DEEPSEEK_PROVIDER_ID);
        let provider = DeepSeekSearchProvider::new(|| DeepSeekSearchProviderOptions {
            api_key: Some(String::new()),
            ..options_with_key()
        });
        assert!(!provider.available());
        let provider = DeepSeekSearchProvider::new(|| DeepSeekSearchProviderOptions {
            base_url: "not a url".into(),
            ..options_with_key()
        });
        assert!(!provider.available());
        let provider = DeepSeekSearchProvider::new(|| DeepSeekSearchProviderOptions {
            max_tokens: 0,
            ..options_with_key()
        });
        assert!(!provider.available());
    }

    #[tokio::test]
    async fn redirect_is_provider_error_and_does_not_follow() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let followed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let followed_flag = std::sync::Arc::clone(&followed);
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let followed_flag = std::sync::Arc::clone(&followed_flag);
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buf = vec![0u8; 4096];
                    let _ = stream.read(&mut buf).await;
                    let request = String::from_utf8_lossy(&buf);
                    if request.contains(" /followed") {
                        followed_flag.store(true, std::sync::atomic::Ordering::SeqCst);
                        let _ = stream
                            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                            .await;
                    } else {
                        let location = format!("http://{addr}/followed");
                        let body = format!(
                            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\n\r\n"
                        );
                        let _ = stream.write_all(body.as_bytes()).await;
                    }
                });
            }
        });
        let base = format!("http://{addr}");
        let provider = DeepSeekSearchProvider::new(move || DeepSeekSearchProviderOptions {
            base_url: base.clone(),
            ..options_with_key()
        });
        let err = provider
            .search(
                WebSearchRequest {
                    query: "q".into(),
                    max_results: None,
                },
                AbortFlag::new(),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, WEB_PROVIDER_ERROR);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(!followed.load(std::sync::atomic::Ordering::SeqCst));
    }
}
