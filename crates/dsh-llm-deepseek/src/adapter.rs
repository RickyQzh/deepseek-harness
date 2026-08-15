//! DeepSeek HTTP SSE adapter.
//!
//! [`DeepSeekAdapter::stream`] yields [`Result`](std::result::Result) chunks.
//! Idle-timeout is mapped to `Ok(Finish { Error { code: TIMEOUT } })` inside
//! this adapter so a direct consumer (without [`dsh_llm::LlmRuntime`]) still
//! observes a terminal finish. Unexpected transport errors remain `Err` for the
//! runtime wrap.

use std::pin::Pin;
use std::time::Duration;

use dsh_credentials::CredentialRef;
use dsh_llm::{GenerateOptions, LlmAdapter, LlmError, LlmPurpose, attribution_headers};
use dsh_session::{FinishReason, StreamChunk};
use dsh_tools::AbortFlag;
use futures::Stream;
use futures::StreamExt;
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};

use crate::serialize::{RequestDefaults, serialize_request};
use crate::sse::{DONE, SseParser};
use crate::translate::translate;

/// Default maximum idle interval while one stream read is outstanding, in milliseconds.
pub const DEFAULT_STREAM_IDLE_TIMEOUT_MS: u64 = 300_000;
/// Default combined request/response context capacity.
pub const DEFAULT_CONTEXT_WINDOW: u64 = 1_000_000;
/// Default per-request output-token cap.
pub const DEFAULT_MAX_TOKENS: u64 = 256_000;

/// Validated connection facts for one `stream` call.
pub struct DeepSeekConnectionOptions {
    /// Endpoint base; `/chat/completions` is appended after trimming a trailing `/`.
    pub base_url: String,
    /// Credential reference resolved per request; never stored on the adapter.
    pub api_key_env: CredentialRef,
    /// Request defaults applied to every call (thinking mode, effort).
    pub defaults: RequestDefaults,
    /// Default per-request output cap; explicit request values win.
    pub max_tokens: u64,
    /// Context capacity used when an exact model value is unavailable.
    pub default_context_window: u64,
    /// Maximum provider idle time while one stream read is outstanding.
    pub stream_idle_timeout: Duration,
}

type ConnectionOptionsFn = Box<dyn Fn() -> DeepSeekConnectionOptions + Send + Sync>;
type ResolveApiKeyFn =
    Box<dyn Fn(&DeepSeekConnectionOptions) -> Result<String, LlmError> + Send + Sync>;

/// DeepSeek chat-completions SSE adapter.
///
/// The bearer token is resolved on every [`LlmAdapter::stream`] call from that
/// call's connection snapshot. This struct does not store the key.
pub struct DeepSeekAdapter {
    options: ConnectionOptionsFn,
    resolve_api_key: ResolveApiKeyFn,
    user_id: String,
    client: reqwest::Client,
}

impl DeepSeekAdapter {
    /// Capture per-call connection and credential hooks plus the harness user id.
    #[must_use]
    pub fn new(
        options: impl Fn() -> DeepSeekConnectionOptions + Send + Sync + 'static,
        resolve_api_key: impl Fn(&DeepSeekConnectionOptions) -> Result<String, LlmError>
        + Send
        + Sync
        + 'static,
        user_id: impl Into<String>,
    ) -> Self {
        Self {
            options: Box::new(options),
            resolve_api_key: Box::new(resolve_api_key),
            user_id: user_id.into(),
            client: reqwest::Client::builder()
                .use_rustls_tls()
                .build()
                .expect("reqwest rustls client"),
        }
    }
}

/// Map a non-success HTTP status to a stable [`LlmError::code`].
#[must_use]
pub fn http_error_code(status: u16) -> &'static str {
    match status {
        401 | 403 => "AUTH",
        429 => "RATE_LIMIT",
        400 => "INVALID_REQUEST",
        s if s >= 500 => "SERVER",
        other => Box::leak(format!("HTTP_{other}").into_boxed_str()),
    }
}

fn aborted() -> LlmError {
    LlmError::new("DeepSeek request aborted by caller", "ABORTED")
}

fn timeout_error(idle: Duration) -> LlmError {
    LlmError::new(
        format!("DeepSeek stream idle timeout after {}ms", idle.as_millis()),
        "TIMEOUT",
    )
}

fn header_value(value: &str) -> Result<HeaderValue, LlmError> {
    HeaderValue::from_str(value).map_err(|_| {
        LlmError::new(
            "DeepSeek request header value is not a valid HTTP header",
            "INVALID_REQUEST",
        )
    })
}

fn request_headers(
    api_key: &str,
    user_id: &str,
    options: &GenerateOptions,
) -> Result<HeaderMap, LlmError> {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, header_value(&format!("Bearer {api_key}"))?);
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));
    for (name, value) in attribution_headers() {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| LlmError::new("invalid attribution header name", "INVALID_REQUEST"))?;
        headers.insert(name, header_value(&value)?);
    }
    headers.insert(
        HeaderName::from_static("x-deepseek-harness-user-id"),
        header_value(user_id)?,
    );
    if let Some(session_id) = &options.session_id {
        headers.insert(
            HeaderName::from_static("x-deepseek-harness-session-id"),
            header_value(session_id.as_str())?,
        );
    }
    if options.purpose == Some(LlmPurpose::Compaction) {
        headers.insert(
            HeaderName::from_static("x-deepseek-harness-compact"),
            HeaderValue::from_static("1"),
        );
    }
    Ok(headers)
}

fn finish_timeout(error: LlmError) -> StreamChunk {
    StreamChunk::Finish {
        reason: FinishReason::Error {
            failure: error.failure(),
        },
        replay_state: None,
    }
}

fn as_stream_items(
    result: Result<Vec<StreamChunk>, LlmError>,
) -> Vec<Result<StreamChunk, LlmError>> {
    match result {
        Ok(chunks) => chunks.into_iter().map(Ok).collect(),
        Err(error) if error.code == "TIMEOUT" => vec![Ok(finish_timeout(error))],
        Err(error) => vec![Err(error)],
    }
}

async fn read_payloads(
    response: reqwest::Response,
    idle: Duration,
    signal: AbortFlag,
) -> Result<Vec<String>, LlmError> {
    let mut stream = response.bytes_stream();
    let mut parser = SseParser::new();
    let mut payloads = Vec::new();
    let mut on_comment = |_comment: &str| {};
    loop {
        if signal.is_aborted() {
            return Err(aborted());
        }
        let timed = tokio::select! {
            () = signal.cancelled() => return Err(aborted()),
            timed = tokio::time::timeout(idle, stream.next()) => timed,
        };
        match timed {
            Err(_) => return Err(timeout_error(idle)),
            Ok(None) => {
                return Err(LlmError::new(
                    "SSE stream ended without [DONE]",
                    "STREAM_CLOSED",
                ));
            }
            Ok(Some(Err(error))) => {
                return Err(LlmError::new(
                    format!("DeepSeek API stream failed: {error}"),
                    "TRANSPORT",
                ));
            }
            Ok(Some(Ok(bytes))) => {
                for payload in parser.push_bytes(&bytes, &mut on_comment)? {
                    payloads.push(payload.clone());
                    if payload == DONE {
                        return Ok(payloads);
                    }
                }
            }
        }
    }
}

async fn run_request(
    client: reqwest::Client,
    user_id: String,
    connection: DeepSeekConnectionOptions,
    api_key: String,
    options: GenerateOptions,
) -> Result<Vec<StreamChunk>, LlmError> {
    if options.signal.is_aborted() {
        return Err(aborted());
    }
    let idle = connection.stream_idle_timeout;
    let body = serialize_request(&options, &connection.defaults)?;
    let url = format!(
        "{}/chat/completions",
        connection.base_url.trim_end_matches('/')
    );
    let headers = request_headers(&api_key, &user_id, &options)?;
    let send = client.post(&url).headers(headers).json(&body).send();
    let response = tokio::select! {
        () = options.signal.cancelled() => return Err(aborted()),
        timed = tokio::time::timeout(idle, send) => match timed {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                return Err(LlmError::new(
                    format!(
                        "DeepSeek API request to {} failed: {error}",
                        connection.base_url
                    ),
                    "TRANSPORT",
                ));
            }
            Err(_) => return Err(timeout_error(idle)),
        },
    };
    let status = response.status();
    if !status.is_success() {
        let code = status.as_u16();
        return Err(LlmError::new(
            format!("DeepSeek API error (HTTP {code})"),
            http_error_code(code),
        )
        .with_status(code));
    }
    let payloads = read_payloads(response, idle, options.signal.clone()).await?;
    translate(futures::stream::iter(payloads)).await
}

impl LlmAdapter for DeepSeekAdapter {
    fn stream(
        &self,
        options: GenerateOptions,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, LlmError>> + Send + '_>> {
        let connection = (self.options)();
        let api_key = match (self.resolve_api_key)(&connection) {
            Ok(key) => key,
            Err(error) => return Box::pin(futures::stream::once(async move { Err(error) })),
        };
        let client = self.client.clone();
        let user_id = self.user_id.clone();
        Box::pin(
            futures::stream::once(async move {
                as_stream_items(run_request(client, user_id, connection, api_key, options).await)
            })
            .map(futures::stream::iter)
            .flatten(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::DeepSeekAdapter;
    use crate::{DeepSeekConnectionOptions, RequestDefaults};
    use dsh_credentials::{CredentialProvider, LayeredCredentials, credential_ref};
    use dsh_llm::{GenerateOptions, LlmAdapter, LlmError, LlmPurpose, assert_usable_api_key};
    use dsh_session::{
        ContentBlock, FinishReason, Message, MessageId, MessageRole, MessageSource, SessionId,
        StreamChunk,
    };
    use dsh_tools::AbortFlag;
    use futures::StreamExt;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[derive(Default, Clone)]
    struct Captured {
        authorization: String,
        user_agent: String,
        user_id: String,
        session_id: Option<String>,
        compact: Option<String>,
        path: String,
        body: String,
    }

    async fn spawn_sse(
        status: u16,
        sse_body: &'static str,
        delay: Duration,
    ) -> (String, Arc<Mutex<Captured>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let captured = Arc::new(Mutex::new(Captured::default()));
        let slot = captured.clone();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let n = stream.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).into_owned();
            {
                let mut cap = slot.lock().unwrap();
                cap.path = request.lines().next().unwrap_or("").to_string();
                for line in request.lines() {
                    let lower = line.to_ascii_lowercase();
                    if let Some(v) = line
                        .strip_prefix("Authorization: ")
                        .or_else(|| line.strip_prefix("authorization: "))
                    {
                        cap.authorization = v.trim().to_string();
                    }
                    if lower.starts_with("user-agent:") {
                        cap.user_agent = line.split_once(':').unwrap().1.trim().to_string();
                    }
                    if lower.starts_with("x-deepseek-harness-user-id:") {
                        cap.user_id = line.split_once(':').unwrap().1.trim().to_string();
                    }
                    if lower.starts_with("x-deepseek-harness-session-id:") {
                        cap.session_id = Some(line.split_once(':').unwrap().1.trim().to_string());
                    }
                    if lower.starts_with("x-deepseek-harness-compact:") {
                        cap.compact = Some(line.split_once(':').unwrap().1.trim().to_string());
                    }
                }
                if let Some(idx) = request.find("\r\n\r\n") {
                    cap.body = request[idx + 4..].to_string();
                }
            }
            tokio::time::sleep(delay).await;
            let response = format!(
                "HTTP/1.1 {status} OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{sse_body}"
            );
            let _ = stream.write_all(response.as_bytes()).await;
        });
        (format!("http://{addr}"), captured)
    }

    fn user_hi() -> Vec<Message> {
        vec![Message {
            id: MessageId::new("u1"),
            role: MessageRole::User,
            content: vec![ContentBlock::Text { text: "hi".into() }],
            source: MessageSource::User,
        }]
    }

    #[tokio::test]
    async fn posts_chat_completions_with_identity_and_compact_headers() {
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1}}\n\ndata: [DONE]\n\n";
        let (base, captured) = spawn_sse(200, sse, Duration::ZERO).await;
        let mut creds = LayeredCredentials::new();
        let r = credential_ref("PHASE3_DS_KEY").unwrap();
        creds.set(&r, "sk-one".into()).unwrap();
        let creds = Arc::new(Mutex::new(creds));
        let creds_opt = creds.clone();
        let adapter = DeepSeekAdapter::new(
            {
                let base = base.clone();
                move || DeepSeekConnectionOptions {
                    base_url: base.clone(),
                    api_key_env: credential_ref("PHASE3_DS_KEY").unwrap(),
                    defaults: RequestDefaults::default(),
                    max_tokens: 256_000,
                    default_context_window: 1_000_000,
                    stream_idle_timeout: Duration::from_secs(5),
                }
            },
            move |connection| {
                let creds = creds_opt.lock().unwrap();
                let resolved = creds
                    .resolve(&connection.api_key_env)
                    .unwrap()
                    .ok_or_else(|| LlmError::new("missing", "MISSING_CREDENTIAL"))?;
                assert_usable_api_key(
                    &resolved.value,
                    "dsh-llm-deepseek",
                    connection.api_key_env.as_str(),
                )
            },
            "anon-1",
        );
        let options = GenerateOptions {
            provider: "deepseek".into(),
            model: "deepseek-chat".into(),
            reasoning_effort: None,
            messages: user_hi(),
            system: None,
            tools: None,
            temperature: None,
            max_tokens: None,
            stop: None,
            signal: AbortFlag::new(),
            session_id: Some(SessionId::new("sess-1")),
            purpose: Some(LlmPurpose::Compaction),
        };
        let mut stream = adapter.stream(options);
        let mut chunks = Vec::new();
        while let Some(chunk) = stream.next().await {
            chunks.push(chunk.unwrap());
        }
        assert!(matches!(
            chunks.last(),
            Some(StreamChunk::Finish {
                reason: FinishReason::Stop,
                ..
            })
        ));
        let cap = captured.lock().unwrap();
        assert!(cap.path.contains("POST /chat/completions"));
        assert_eq!(cap.authorization, "Bearer sk-one");
        assert!(cap.user_agent.starts_with("deepseek-harness/"));
        assert_eq!(cap.user_id, "anon-1");
        assert_eq!(cap.session_id.as_deref(), Some("sess-1"));
        assert_eq!(cap.compact.as_deref(), Some("1"));
        let body: serde_json::Value = serde_json::from_str(&cap.body).unwrap();
        assert_eq!(body["stream"], serde_json::json!(true));
        assert_eq!(
            body["stream_options"]["include_usage"],
            serde_json::json!(true)
        );
    }

    #[tokio::test]
    async fn resolve_api_key_is_per_request() {
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1}}\n\ndata: [DONE]\n\n";
        let (base1, cap1) = spawn_sse(200, sse, Duration::ZERO).await;
        let mut creds = LayeredCredentials::new();
        let r = credential_ref("PHASE3_DS_KEY").unwrap();
        creds.set(&r, "sk-one".into()).unwrap();
        let creds = Arc::new(Mutex::new(creds));
        let make = |base: String, creds: Arc<Mutex<LayeredCredentials>>| {
            DeepSeekAdapter::new(
                move || DeepSeekConnectionOptions {
                    base_url: base.clone(),
                    api_key_env: credential_ref("PHASE3_DS_KEY").unwrap(),
                    defaults: RequestDefaults::default(),
                    max_tokens: 256_000,
                    default_context_window: 1_000_000,
                    stream_idle_timeout: Duration::from_secs(5),
                },
                move |connection| {
                    let creds = creds.lock().unwrap();
                    let resolved = creds
                        .resolve(&connection.api_key_env)
                        .unwrap()
                        .expect("key");
                    assert_usable_api_key(
                        &resolved.value,
                        "dsh-llm-deepseek",
                        connection.api_key_env.as_str(),
                    )
                },
                "anon-1",
            )
        };
        let adapter = make(base1, creds.clone());
        let options = GenerateOptions {
            provider: "deepseek".into(),
            model: "deepseek-chat".into(),
            reasoning_effort: None,
            messages: user_hi(),
            system: None,
            tools: None,
            temperature: None,
            max_tokens: None,
            stop: None,
            signal: AbortFlag::new(),
            session_id: None,
            purpose: None,
        };
        let mut s = adapter.stream(options.clone());
        while s.next().await.is_some() {}
        creds.lock().unwrap().set(&r, "sk-two".into()).unwrap();
        let (base2, cap2) = spawn_sse(200, sse, Duration::ZERO).await;
        let adapter2 = make(base2, creds);
        let mut s = adapter2.stream(options);
        while s.next().await.is_some() {}
        assert_eq!(cap1.lock().unwrap().authorization, "Bearer sk-one");
        assert_eq!(cap2.lock().unwrap().authorization, "Bearer sk-two");
    }

    #[tokio::test]
    async fn idle_watchdog_times_out() {
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        let (base, _) = spawn_sse(200, sse, Duration::from_millis(200)).await;
        let adapter = DeepSeekAdapter::new(
            move || DeepSeekConnectionOptions {
                base_url: base.clone(),
                api_key_env: credential_ref("PHASE3_DS_KEY").unwrap(),
                defaults: RequestDefaults::default(),
                max_tokens: 256_000,
                default_context_window: 1_000_000,
                stream_idle_timeout: Duration::from_millis(20),
            },
            |_| Ok("sk-idle".into()),
            "anon-1",
        );
        let options = GenerateOptions {
            provider: "deepseek".into(),
            model: "deepseek-chat".into(),
            reasoning_effort: None,
            messages: user_hi(),
            system: None,
            tools: None,
            temperature: None,
            max_tokens: None,
            stop: None,
            signal: AbortFlag::new(),
            session_id: None,
            purpose: None,
        };
        let mut stream = adapter.stream(options);
        let mut last = None;
        while let Some(chunk) = stream.next().await {
            last = Some(chunk.unwrap());
        }
        match last {
            Some(StreamChunk::Finish {
                reason: FinishReason::Error { failure },
                ..
            }) => {
                assert_eq!(failure.code, "TIMEOUT");
            }
            other => panic!("{other:?}"),
        }
    }
}
