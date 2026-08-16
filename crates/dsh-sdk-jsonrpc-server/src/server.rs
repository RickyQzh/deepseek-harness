//! JSON-RPC methods for the SDK runtime.

use std::sync::{Arc, Mutex};

use dsh_agent::{AgentRegistry, CreateAgentOptions};
use dsh_sdk_protocol::{
    InitializeParams, InitializeResult, JsonRpcLineTransport, SessionEventNotification,
    SessionPromptParams, SessionPromptResult, SessionStatus, SessionStatusNotification,
    ShutdownResult,
};
use dsh_session::{
    AppendSink, LogEvent, Message, MessageId, MessageRole, MessageSource, SessionId,
};
use dsh_session_persist::JsonlSessionStore;
use serde_json::Value;

struct Inner {
    cwd: String,
    provider: String,
    model: String,
    max_tokens: Option<u64>,
    initialized: bool,
    shutting_down: bool,
    exit: Option<Arc<dyn Fn(i32) + Send + Sync>>,
}

/// SDK server over one transport and one agent registry.
pub struct HarnessSdkJsonRpcServer {
    inner: Arc<Mutex<Inner>>,
    agents: Arc<AgentRegistry>,
    sessions: Arc<JsonlSessionStore>,
    transport: JsonRpcLineTransport,
}

fn mint_message_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("msg-{}-{}", std::process::id(), nanos)
}

impl HarnessSdkJsonRpcServer {
    /// Construct a server. Call [`bind`](Self::bind) before [`JsonRpcLineTransport::serve`].
    #[must_use]
    pub fn new(
        agents: Arc<AgentRegistry>,
        sessions: Arc<JsonlSessionStore>,
        transport: JsonRpcLineTransport,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                cwd: String::new(),
                provider: String::new(),
                model: String::new(),
                max_tokens: None,
                initialized: false,
                shutting_down: false,
                exit: None,
            })),
            agents,
            sessions,
            transport,
        }
    }

    /// Install the append sink factory and the JSON-RPC request handler.
    pub fn bind(&self) {
        let transport = self.transport.clone();
        let factory: Arc<dyn Fn(&str) -> AppendSink + Send + Sync> =
            Arc::new(move |session_id: &str| {
                let sid = session_id.to_string();
                let transport = transport.clone();
                Arc::new(move |event: &LogEvent| {
                    let LogEvent::Known(session_event) = event else {
                        return;
                    };
                    let payload = SessionEventNotification::new(sid.clone(), session_event.clone());
                    let transport = transport.clone();
                    tokio::spawn(async move {
                        let params = serde_json::to_value(payload).expect("session.event");
                        let _ = transport.notify("session.event", Some(params)).await;
                    });
                })
            });
        self.agents.set_sink_factory(Some(factory));
        let server = self.clone_handles();
        self.transport.on_request(Arc::new(move |method, params| {
            let server = server.clone_handles();
            Box::pin(async move { server.handle_request(method, params).await })
        }));
    }

    fn clone_handles(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            agents: Arc::clone(&self.agents),
            sessions: Arc::clone(&self.sessions),
            transport: self.transport.clone(),
        }
    }

    /// Dispatch one JSON-RPC method. `Err` becomes `-32603`.
    ///
    /// # Errors
    ///
    /// Unknown method, re-initialize, invalid `maxTokens`, missing session after shutdown, or loop failure.
    pub async fn handle_request(&self, method: String, params: Value) -> Result<Value, String> {
        match method.as_str() {
            "initialize" => {
                let parsed: InitializeParams =
                    serde_json::from_value(params).map_err(|error| error.to_string())?;
                self.initialize(parsed).await
            }
            "session/prompt" => {
                let parsed: SessionPromptParams =
                    serde_json::from_value(params).map_err(|error| error.to_string())?;
                self.prompt(parsed).await
            }
            "shutdown" => self.shutdown().await,
            other => Err(format!(
                "unknown DeepSeek Harness SDK runtime method: {other}"
            )),
        }
    }

    async fn initialize(&self, params: InitializeParams) -> Result<Value, String> {
        if let Some(max_tokens) = params.max_tokens() {
            if max_tokens == 0 {
                return Err("initialize maxTokens must be a positive safe integer".into());
            }
        }
        {
            let mut inner = self.inner.lock().expect("inner");
            if inner.initialized {
                return Err("re-initialize is unsupported".into());
            }
            inner.cwd = params.cwd().to_string();
            inner.provider = params.provider().to_string();
            inner.model = params.model().to_string();
            inner.max_tokens = params.max_tokens();
            inner.initialized = true;
        }
        let provider = params.provider().to_string();
        if !self
            .agents
            .list_providers()
            .iter()
            .any(|id| id == &provider)
        {
            return Err(format!("no adapter registered for provider \"{provider}\""));
        }
        serde_json::to_value(InitializeResult::harness_runtime()).map_err(|error| error.to_string())
    }

    async fn prompt(&self, params: SessionPromptParams) -> Result<Value, String> {
        let (cwd, provider, model, max_tokens) = {
            let inner = self.inner.lock().expect("inner");
            if inner.shutting_down {
                return Err("SDK server is shutting down".into());
            }
            if !inner.initialized {
                return Err("initialize must be called before session/prompt".into());
            }
            (
                inner.cwd.clone(),
                inner.provider.clone(),
                inner.model.clone(),
                inner.max_tokens,
            )
        };
        if self.agents.get(params.session_id()).is_none() {
            self.agents
                .create(CreateAgentOptions {
                    session_id: SessionId::new(params.session_id()),
                    cwd: Some(cwd),
                    provider,
                    model,
                    max_tokens,
                })
                .map_err(|error| error.to_string())?;
        }
        let message_id = mint_message_id();
        let message = Message {
            id: MessageId::new(message_id.clone()),
            role: MessageRole::User,
            content: params.content_blocks().to_vec(),
            source: MessageSource::User,
        };
        self.agents
            .followup(params.session_id(), message)
            .await
            .map_err(|error| error.to_string())?;
        let session_id = params.session_id().to_string();
        let agents = Arc::clone(&self.agents);
        let sessions = Arc::clone(&self.sessions);
        let transport = self.transport.clone();
        tokio::spawn(async move {
            let running =
                SessionStatusNotification::new(session_id.clone(), SessionStatus::Running);
            let _ = transport
                .notify(
                    "session.status",
                    Some(serde_json::to_value(running).expect("status")),
                )
                .await;
            let _ = agents.when_idle(&session_id).await;
            if let Some(handle) = agents.get(&session_id) {
                let guard = handle.lock().await;
                let _ = sessions.flush(&guard.session);
            }
            let idle = SessionStatusNotification::new(session_id, SessionStatus::Idle);
            let _ = transport
                .notify(
                    "session.status",
                    Some(serde_json::to_value(idle).expect("status")),
                )
                .await;
        });
        serde_json::to_value(SessionPromptResult::new(message_id))
            .map_err(|error| error.to_string())
    }

    async fn shutdown(&self) -> Result<Value, String> {
        let hook = {
            let mut inner = self.inner.lock().expect("inner");
            inner.shutting_down = true;
            inner.exit.clone()
        };
        if let Some(hook) = hook {
            tokio::spawn(async move {
                tokio::task::yield_now().await;
                hook(0);
            });
        }
        serde_json::to_value(ShutdownResult {}).map_err(|error| error.to_string())
    }

    /// Process-exit hook used by the stdio plugin after a `shutdown` result is returned. Tests leave this unset.
    pub fn set_exit_hook(&self, hook: Arc<dyn Fn(i32) + Send + Sync>) {
        self.inner.lock().expect("inner").exit = Some(hook);
    }
}

#[cfg(test)]
mod tests {
    use super::HarnessSdkJsonRpcServer;
    use dsh_agent::AgentRegistry;
    use dsh_agent_loop::{CancelCause, CancelOptions};
    use dsh_llm::{LlmAdapter, LlmRuntime, MockAdapter, MockScript, text_response};
    use dsh_sdk_protocol::{
        JsonRpcId, JsonRpcLineTransport, SDK_SERVER_NAME, SessionPromptParams, encode_request,
    };
    use dsh_session::ContentBlock;
    use dsh_session_persist::JsonlSessionStore;
    use dsh_system_prompt::{SystemPrompt, SystemPromptConfig};
    use dsh_tools::{ToolPresentationMode, ToolRuntime};
    use serde_json::{Value, json};
    use std::sync::Arc;
    use tokio::io::{AsyncBufReadExt, BufReader, duplex};

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

    fn registry_with(script: Vec<MockScript>) -> AgentRegistry {
        let adapter = Arc::new(MockAdapter::new(script));
        let mut llm = LlmRuntime::new();
        llm.register_adapter("mock", adapter);
        AgentRegistry::new(
            llm,
            ToolRuntime::new(ToolPresentationMode::Native),
            SystemPrompt::new(SystemPromptConfig::default()).unwrap(),
        )
    }

    #[tokio::test]
    async fn initialize_server_info_name_is_stable() {
        let root = test_temp_dir("dsh-jsonrpc-init");
        let (a, b) = duplex(64 * 1024);
        let (a_read, a_write) = tokio::io::split(a);
        let (b_read, b_write) = tokio::io::split(b);
        let transport = JsonRpcLineTransport::new(BufReader::new(b_read), b_write);
        let agents = Arc::new(registry_with(vec![MockScript::Chunks(text_response("hi"))]));
        let sessions = Arc::new(JsonlSessionStore::with_root(&root));
        let server = HarnessSdkJsonRpcServer::new(agents, sessions, transport.clone());
        server.bind();
        let serve = tokio::spawn({
            let transport = transport.clone();
            async move { transport.serve().await }
        });
        let client = JsonRpcLineTransport::new(BufReader::new(a_read), a_write);
        let serve_client = tokio::spawn({
            let client = client.clone();
            async move { client.serve().await }
        });
        let result = client
            .request(
                "initialize",
                json!({"cwd": "/work", "provider": "mock", "model": "mock"}),
            )
            .await
            .unwrap();
        assert_eq!(result["serverInfo"]["name"], json!(SDK_SERVER_NAME));
        assert_eq!(result["serverInfo"]["version"], json!("0.0.1"));
        let _ = client.request("shutdown", json!({})).await;
        // Client close EOFs the server reader; server close EOFs the client reader.
        client.close().await;
        transport.close().await;
        let _ = serve.await;
        let _ = serve_client.await;
    }

    #[tokio::test]
    async fn prompt_returns_message_id_before_turn_ends() {
        let root = test_temp_dir("dsh-jsonrpc-prompt");
        let (a, b) = duplex(64 * 1024);
        let (client_read, client_write) = tokio::io::split(a);
        let (server_read, server_write) = tokio::io::split(b);
        let transport = JsonRpcLineTransport::new(BufReader::new(server_read), server_write);
        let adapter = Arc::new(MockAdapter::new(vec![MockScript::Hang]));
        let mut llm = LlmRuntime::new();
        llm.register_adapter("mock", Arc::clone(&adapter) as Arc<dyn LlmAdapter>);
        let agents = Arc::new(AgentRegistry::new(
            llm,
            ToolRuntime::new(ToolPresentationMode::Native),
            SystemPrompt::new(SystemPromptConfig::default()).unwrap(),
        ));
        let sessions = Arc::new(JsonlSessionStore::with_root(&root));
        let server = HarnessSdkJsonRpcServer::new(Arc::clone(&agents), sessions, transport.clone());
        server.bind();
        let serve = tokio::spawn(async move { transport.serve().await });
        let mut writer = client_write;
        let mut lines = BufReader::new(client_read).lines();
        use tokio::io::AsyncWriteExt;
        writer
            .write_all(
                encode_request(
                    &JsonRpcId::Number(1),
                    "initialize",
                    &json!({"cwd": "/work", "provider": "mock", "model": "mock"}),
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let init_line = lines.next_line().await.unwrap().expect("init");
        let init: Value = serde_json::from_str(&init_line).unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], json!(SDK_SERVER_NAME));
        writer
            .write_all(
                encode_request(
                    &JsonRpcId::Number(2),
                    "session/prompt",
                    &serde_json::to_value(SessionPromptParams::new(
                        "sess-lazy",
                        vec![ContentBlock::Text { text: "go".into() }],
                    ))
                    .unwrap(),
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let prompt_line = lines.next_line().await.unwrap().expect("prompt result");
        let prompt: Value = serde_json::from_str(&prompt_line).unwrap();
        assert!(prompt.get("result").is_some(), "{prompt}");
        let message_id = prompt["result"]["messageId"].as_str().expect("messageId");
        assert!(message_id.starts_with("msg-"), "{message_id}");
        assert!(prompt.get("error").is_none());
        let types: Vec<String> = Vec::new();
        // Hang never emits turn/end; the result already returned.
        let _ = types;
        // `when_idle` holds the agent mutex for the Hang stream. Abort the recorded
        // request signal so `LoopAgent::cancel` can lock afterwards.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let aborted = {
                let requests = adapter.requests.lock().expect("requests");
                if let Some(request) = requests.first() {
                    request.signal.abort();
                    true
                } else {
                    false
                }
            };
            if aborted {
                break;
            }
            if std::time::Instant::now() > deadline {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let handle = agents.get("sess-lazy").expect("agent");
        {
            let mut guard = handle.lock().await;
            guard
                .cancel(CancelCause::User, CancelOptions::default())
                .expect("cancel");
        }
        writer
            .write_all(encode_request(&JsonRpcId::Number(3), "shutdown", &json!({})).as_bytes())
            .await
            .unwrap();
        writer.shutdown().await.unwrap();
        let _ = serve.await;
    }

    #[tokio::test]
    async fn lazy_prompt_then_idle_on_text_mock() {
        let root = test_temp_dir("dsh-jsonrpc-idle");
        let (a, b) = duplex(64 * 1024);
        let (a_read, a_write) = tokio::io::split(a);
        let (b_read, b_write) = tokio::io::split(b);
        let transport = JsonRpcLineTransport::new(BufReader::new(b_read), b_write);
        let agents = Arc::new(registry_with(vec![MockScript::Chunks(text_response(
            "sdk-text",
        ))]));
        let sessions = Arc::new(JsonlSessionStore::with_root(&root));
        let server = HarnessSdkJsonRpcServer::new(Arc::clone(&agents), sessions, transport.clone());
        server.bind();
        let serve = tokio::spawn({
            let transport = transport.clone();
            async move { transport.serve().await }
        });
        let client = JsonRpcLineTransport::new(BufReader::new(a_read), a_write);
        let notes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let notes_clone = Arc::clone(&notes);
        client.on_notification(Arc::new(move |method, params| {
            notes_clone.lock().expect("notes").push((method, params));
        }));
        let serve_client = tokio::spawn({
            let client = client.clone();
            async move { client.serve().await }
        });
        client
            .request(
                "initialize",
                json!({"cwd": "/work", "provider": "mock", "model": "mock"}),
            )
            .await
            .unwrap();
        let prompt = client
            .request(
                "session/prompt",
                serde_json::to_value(SessionPromptParams::new(
                    "sess-text",
                    vec![ContentBlock::Text { text: "go".into() }],
                ))
                .unwrap(),
            )
            .await
            .unwrap();
        assert!(prompt["messageId"].as_str().unwrap().starts_with("msg-"));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let idle = notes.lock().expect("notes").iter().any(|(method, params)| {
                method == "session.status"
                    && params.get("status").and_then(Value::as_str) == Some("idle")
            });
            if idle {
                break;
            }
            if std::time::Instant::now() > deadline {
                panic!("no idle status: {:?}", notes.lock().expect("notes"));
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(agents.get("sess-text").is_some());
        let _ = client.request("shutdown", json!({})).await;
        client.close().await;
        transport.close().await;
        let _ = serve.await;
        let _ = serve_client.await;
    }

    #[tokio::test]
    async fn reinitialize_is_unsupported() {
        let root = test_temp_dir("dsh-jsonrpc-reinit");
        let (a, b) = duplex(64 * 1024);
        let (a_read, a_write) = tokio::io::split(a);
        let (b_read, b_write) = tokio::io::split(b);
        let transport = JsonRpcLineTransport::new(BufReader::new(b_read), b_write);
        let agents = Arc::new(registry_with(vec![MockScript::Chunks(text_response("x"))]));
        let sessions = Arc::new(JsonlSessionStore::with_root(&root));
        let server = HarnessSdkJsonRpcServer::new(agents, sessions, transport.clone());
        server.bind();
        let serve = tokio::spawn({
            let transport = transport.clone();
            async move { transport.serve().await }
        });
        let client = JsonRpcLineTransport::new(BufReader::new(a_read), a_write);
        let serve_client = tokio::spawn({
            let client = client.clone();
            async move { client.serve().await }
        });
        let params = json!({"cwd": "/work", "provider": "mock", "model": "mock"});
        client.request("initialize", params.clone()).await.unwrap();
        let err = client.request("initialize", params).await.unwrap_err();
        assert!(err.to_string().contains("re-initialize"), "{err}");
        let _ = client.request("shutdown", json!({})).await;
        client.close().await;
        transport.close().await;
        let _ = serve.await;
        let _ = serve_client.await;
    }
}
