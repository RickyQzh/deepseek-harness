//! ACP JSON-RPC method dispatch over NDJSON transport.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use dsh_agent::{AgentHandle, AgentRegistry, CreateAgentOptions};
use dsh_session::{
    ContentBlock, LogEvent, Message, MessageId, MessageRole, MessageSource, SessionEvent,
    SessionId, TurnEndReason,
};
use dsh_session_persist::JsonlSessionStore;
use serde_json::value::RawValue;
use serde_json::{Value, json};
use tokio::sync::oneshot;

use crate::codec::{
    StopReason, acp_prompt_to_text, prompt_has_unsupported_content, prompt_stop_reason,
};
use crate::error::{AcpError, internal_error, invalid_params, method_not_found};
use crate::rpc::{AcpNdjsonTransport, AcpTransportError, serialize_result};
use crate::types::{
    AuthenticateRequest, InitializeRequest, InitializeResult, NewSessionRequest, NewSessionResult,
    PromptRequest, PromptResult, SessionUpdateParams,
};

struct InFlight {
    tx: Mutex<Option<oneshot::Sender<Result<StopReason, AcpError>>>>,
    end_reason: Mutex<Option<TurnEndReason>>,
}

struct SessionRecord {
    handle: AgentHandle,
    inflight: Option<Arc<InFlight>>,
}

struct AcpBridgeInner {
    transport: AcpNdjsonTransport,
    agents: Arc<AgentRegistry>,
    provider: String,
    model: String,
    sessions: Mutex<HashMap<String, SessionRecord>>,
    store: Mutex<Option<Arc<JsonlSessionStore>>>,
    closed: AtomicBool,
    pending_notifies: Arc<AtomicUsize>,
}

/// Automation-only ACP server bound to one NDJSON transport.
#[derive(Clone)]
pub struct AcpBridge {
    inner: Arc<AcpBridgeInner>,
}

async fn wait_until_providers(
    mut list: impl FnMut() -> Vec<String>,
) -> Result<Vec<String>, AcpError> {
    const ATTEMPTS: u32 = 64;
    for attempt in 0..ATTEMPTS {
        let providers = list();
        if !providers.is_empty() {
            return Ok(providers);
        }
        if attempt + 1 == ATTEMPTS {
            break;
        }
        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    Err(internal_error("no LLM provider registered"))
}

fn validate_session_params(params: Value) -> Result<String, AcpError> {
    let request = serde_json::from_value::<NewSessionRequest>(params).unwrap_or_default();
    let cwd = request.cwd();
    if !Path::new(cwd).is_absolute() {
        return Err(invalid_params(&format!(
            "cwd must be an absolute path: {cwd}"
        )));
    }
    if request.additional_directories_unsupported() {
        return Err(invalid_params("additionalDirectories is not supported"));
    }
    if request.mcp_servers_unsupported() {
        return Err(invalid_params("mcpServers is not supported"));
    }
    Ok(cwd.to_string())
}

fn on_session_event(inner: &AcpBridgeInner, session_id: &str, event: &LogEvent) {
    let LogEvent::Known(session_event) = event else {
        return;
    };
    match session_event {
        SessionEvent::AssistantMessage { data, .. } => {
            emit_assistant_chunks(inner, session_id, &data.message.content);
        }
        SessionEvent::TurnEnd { data, .. } => {
            settle_turn_end(inner, session_id, &data.reason);
        }
        _ => {}
    }
}

fn emit_assistant_chunks(inner: &AcpBridgeInner, session_id: &str, blocks: &[ContentBlock]) {
    for block in blocks {
        match block {
            ContentBlock::Text { text } => {
                if !text.is_empty() {
                    notify_agent_message(inner, session_id, text.clone());
                }
            }
            ContentBlock::Image { attachment } => {
                let Some(attachment_id) = attachment.get("attachmentId").and_then(Value::as_str)
                else {
                    continue;
                };
                notify_agent_message(
                    inner,
                    session_id,
                    format!("[image attachment {attachment_id}]"),
                );
            }
            _ => {}
        }
    }
}

fn notify_agent_message(inner: &AcpBridgeInner, session_id: &str, text: String) {
    let params = SessionUpdateParams::agent_message_chunk(session_id.to_string(), text);
    let transport = inner.transport.clone();
    let pending = Arc::clone(&inner.pending_notifies);
    pending.fetch_add(1, Ordering::SeqCst);
    tokio::spawn(async move {
        let _ = transport.notify("session/update", &params).await;
        pending.fetch_sub(1, Ordering::SeqCst);
    });
}

fn settle_turn_end(inner: &AcpBridgeInner, session_id: &str, reason: &TurnEndReason) {
    let slot = {
        let sessions = inner.sessions.lock().expect("sessions");
        sessions
            .get(session_id)
            .and_then(|record| record.inflight.clone())
    };
    let Some(slot) = slot else {
        return;
    };
    match reason {
        TurnEndReason::Error { error } => {
            {
                let mut sessions = inner.sessions.lock().expect("sessions");
                if let Some(record) = sessions.get_mut(session_id) {
                    let same = match record.inflight.as_ref() {
                        Some(current) => Arc::ptr_eq(current, &slot),
                        None => false,
                    };
                    if same {
                        record.inflight = None;
                    }
                }
            }
            if let Some(tx) = slot.tx.lock().expect("inflight tx").take() {
                let _ = tx.send(Err(internal_error(&format!(
                    "turn failed: {}",
                    error.message
                ))));
            }
        }
        other => {
            *slot.end_reason.lock().expect("end_reason") = Some(other.clone());
        }
    }
}

impl AcpBridgeInner {
    fn clear_slot(&self, session_id: &str, slot: &Arc<InFlight>) {
        let mut sessions = self.sessions.lock().expect("sessions");
        if let Some(record) = sessions.get_mut(session_id) {
            let same = match record.inflight.as_ref() {
                Some(current) => Arc::ptr_eq(current, slot),
                None => false,
            };
            if same {
                record.inflight = None;
            }
        }
    }

    async fn drain_notifies(&self) {
        // Spawned session/update writes must finish before the prompt RPC result.
        while self.pending_notifies.load(Ordering::SeqCst) > 0 {
            tokio::task::yield_now().await;
        }
    }

    async fn dispatch(&self, method: &str, params: Value) -> Result<Box<RawValue>, AcpError> {
        match method {
            "initialize" => {
                let _ = serde_json::from_value::<InitializeRequest>(params);
                Ok(serialize_result(&InitializeResult::automation_only()))
            }
            "authenticate" => {
                let _ = serde_json::from_value::<AuthenticateRequest>(params);
                Ok(serialize_result(&json!({})))
            }
            "session/new" => self.session_new(params).await,
            "session/prompt" => self.session_prompt(params).await,
            "session/cancel" => {
                let _map = self.sessions.lock().expect("sessions");
                Ok(serialize_result(&json!({})))
            }
            _ => Err(method_not_found(method)),
        }
    }

    async fn session_new(&self, params: Value) -> Result<Box<RawValue>, AcpError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(internal_error("the ACP bridge has been disposed"));
        }
        let cwd = validate_session_params(params)?;
        wait_until_providers(|| self.agents.list_providers()).await?;
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let session_id = SessionId::new(format!("acp-{pid}-{nanos}"));
        let key = session_id.as_str().to_string();
        let handle = self
            .agents
            .create(CreateAgentOptions {
                session_id,
                cwd: Some(cwd),
                provider: self.provider.clone(),
                model: self.model.clone(),
                max_tokens: None,
            })
            .map_err(|error| internal_error(&error.to_string()))?;
        self.sessions.lock().expect("sessions").insert(
            key.clone(),
            SessionRecord {
                handle,
                inflight: None,
            },
        );
        Ok(serialize_result(&NewSessionResult::new(key)))
    }

    async fn session_prompt(&self, params: Value) -> Result<Box<RawValue>, AcpError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(internal_error("the ACP bridge has been disposed"));
        }
        let request = serde_json::from_value::<PromptRequest>(params).unwrap_or_default();
        let session_id = request.session_id().to_string();
        {
            let sessions = self.sessions.lock().expect("sessions");
            match sessions.get(&session_id) {
                None => {
                    return Err(invalid_params(&format!("unknown session: {session_id}")));
                }
                Some(record) => {
                    if record.inflight.is_some() {
                        return Err(invalid_params(
                            "a prompt is already in flight for this session",
                        ));
                    }
                }
            }
        }
        if prompt_has_unsupported_content(request.prompt()) {
            return Err(invalid_params(
                "only text and resource_link prompt content is supported",
            ));
        }
        let text = acp_prompt_to_text(request.prompt());
        if text.trim().is_empty() {
            return Err(invalid_params("empty prompt"));
        }
        if self.agents.get(&session_id).is_none() {
            return Err(internal_error(
                "prompt was not queued: the agent was disposed outside the bridge",
            ));
        }
        let (tx, rx) = oneshot::channel();
        let slot = Arc::new(InFlight {
            tx: Mutex::new(Some(tx)),
            end_reason: Mutex::new(None),
        });
        {
            let mut sessions = self.sessions.lock().expect("sessions");
            match sessions.get_mut(&session_id) {
                None => {
                    return Err(invalid_params(&format!("unknown session: {session_id}")));
                }
                Some(record) => {
                    if record.inflight.is_some() {
                        return Err(invalid_params(
                            "a prompt is already in flight for this session",
                        ));
                    }
                    record.inflight = Some(Arc::clone(&slot));
                }
            }
        }
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let message = Message {
            id: MessageId::new(format!("msg-{pid}-{nanos}")),
            role: MessageRole::User,
            content: vec![ContentBlock::Text { text }],
            source: MessageSource::User,
        };
        if let Err(error) = self.agents.followup(&session_id, message).await {
            self.clear_slot(&session_id, &slot);
            return Err(internal_error(&format!("prompt was not queued: {error}")));
        }
        let _ = self.agents.when_idle(&session_id).await;
        {
            let mut sessions = self.sessions.lock().expect("sessions");
            if let Some(record) = sessions.get_mut(&session_id) {
                let still = match record.inflight.as_ref() {
                    Some(current) => Arc::ptr_eq(current, &slot),
                    None => false,
                };
                if still {
                    record.inflight = None;
                    let end_reason = slot.end_reason.lock().expect("end_reason").clone();
                    if let Some(tx) = slot.tx.lock().expect("inflight tx").take() {
                        let _ = tx.send(Ok(prompt_stop_reason(end_reason.as_ref())));
                    }
                }
            }
        }
        if let Some(store) = self.store.lock().expect("store").clone() {
            let handle = {
                let sessions = self.sessions.lock().expect("sessions");
                sessions
                    .get(&session_id)
                    .map(|record| record.handle.clone())
            };
            if let Some(handle) = handle {
                let _ = store.flush(&handle.lock().session);
            }
        }
        self.drain_notifies().await;
        match rx.await {
            Ok(Ok(reason)) => Ok(serialize_result(&PromptResult::new(reason))),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(internal_error("prompt settlement dropped")),
        }
    }
}

impl AcpBridge {
    /// Own `transport` and the registry used to create ACP sessions.
    ///
    /// # Parameters
    ///
    /// * `transport` - NDJSON JSON-RPC endpoint. Reading starts in [`serve`](Self::serve).
    /// * `agents` - live agent registry for ACP-created sessions.
    /// * `provider` - provider route stored on the bridge.
    /// * `model` - model id stored on the bridge.
    ///
    /// # Returns
    ///
    /// An unbound bridge. Call [`bind`](Self::bind) before [`serve`](Self::serve).
    #[must_use]
    pub fn new(
        transport: AcpNdjsonTransport,
        agents: Arc<AgentRegistry>,
        provider: String,
        model: String,
    ) -> Self {
        Self {
            inner: Arc::new(AcpBridgeInner {
                transport,
                agents,
                provider,
                model,
                sessions: Mutex::new(HashMap::new()),
                store: Mutex::new(None),
                closed: AtomicBool::new(false),
                pending_notifies: Arc::new(AtomicUsize::new(0)),
            }),
        }
    }

    /// Persist ACP sessions through `store` after each settled prompt.
    ///
    /// # Parameters
    ///
    /// * `store` - JSONL session store flushed after whole-agent idle.
    ///
    /// # Returns
    ///
    /// The same bridge with persistence attached.
    #[must_use]
    pub fn with_sessions(self, store: Arc<JsonlSessionStore>) -> Self {
        *self.inner.store.lock().expect("store") = Some(store);
        self
    }

    /// Install request and notification handlers on the transport.
    pub fn bind(&self) {
        let sink_inner = Arc::clone(&self.inner);
        self.inner
            .agents
            .set_sink_factory(Some(Arc::new(move |id: &str| {
                let inner = Arc::clone(&sink_inner);
                let session_id = id.to_string();
                Arc::new(move |event: &LogEvent| {
                    on_session_event(&inner, &session_id, event);
                })
            })));
        let inner = Arc::clone(&self.inner);
        self.inner
            .transport
            .on_request(Arc::new(move |method, params| {
                let inner = Arc::clone(&inner);
                Box::pin(async move { inner.dispatch(&method, params).await })
            }));
        self.inner
            .transport
            .on_notification(Arc::new(|_method, _params| {}));
    }

    /// Read NDJSON until EOF by delegating to the transport.
    ///
    /// # Errors
    ///
    /// Transport read/write failure.
    ///
    /// # Returns
    ///
    /// `Ok(())` when the input stream reaches EOF.
    pub async fn serve(&self) -> Result<(), AcpTransportError> {
        self.inner.transport.serve().await
    }
}

#[cfg(test)]
#[allow(clippy::await_holding_lock)]
mod tests {
    use super::AcpBridge;
    use crate::{AcpContentBlock, AcpNdjsonTransport, acp_prompt_to_text};
    use dsh_agent::AgentRegistry;
    use dsh_llm::{
        LlmError, LlmRuntime, MockAdapter, MockScript, max_tokens_response, text_response,
        tool_call_response,
    };
    use dsh_session::{ContentBlock, SessionId};
    use dsh_session_persist::JsonlSessionStore;
    use dsh_system_prompt::{SystemPrompt, SystemPromptConfig};
    use dsh_tools::{ToolDefinition, ToolPresentationMode, ToolRuntime};
    use serde_json::{Value, json};
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, duplex};

    const HANDSHAKE_INITIALIZE_LINE: &str = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentInfo":{"name":"deepseek-harness-acp","version":"0.0.1"},"agentCapabilities":{"promptCapabilities":{"image":false,"audio":false,"embeddedContext":false}},"authMethods":[]}}"#;
    const REJECT_EXTRA_DIRS_ERROR_LINE: &str = r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32602,"message":"Invalid params: additionalDirectories is not supported"}}"#;

    static SESSION_ROOT_LOCK: Mutex<()> = Mutex::new(());

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

    fn restore_session_root(previous: Option<String>) {
        match previous {
            Some(value) => unsafe { std::env::set_var("DSH_SESSION_ROOT", value) },
            None => unsafe { std::env::remove_var("DSH_SESSION_ROOT") },
        }
    }

    fn pin_session_root() -> (std::sync::MutexGuard<'static, ()>, Option<String>) {
        let guard = SESSION_ROOT_LOCK.lock().expect("session root");
        let root = test_temp_dir("dsh-acp");
        let previous = std::env::var("DSH_SESSION_ROOT").ok();
        unsafe {
            std::env::set_var("DSH_SESSION_ROOT", root.as_os_str());
        }
        (guard, previous)
    }

    fn registry_with_scripts(
        scripts: Vec<MockScript>,
        register_tools: impl FnOnce(&mut ToolRuntime),
    ) -> (AgentRegistry, Arc<MockAdapter>) {
        let adapter = Arc::new(MockAdapter::new(scripts));
        let mut llm = LlmRuntime::new();
        llm.register_adapter("mock", adapter.clone());
        let mut tools = ToolRuntime::new(ToolPresentationMode::Native);
        register_tools(&mut tools);
        (
            AgentRegistry::new(
                llm,
                tools,
                SystemPrompt::new(SystemPromptConfig::default()).unwrap(),
            ),
            adapter,
        )
    }

    fn registry_with_text(text: &str) -> AgentRegistry {
        registry_with_scripts(vec![MockScript::Chunks(text_response(text))], |_| {}).0
    }

    async fn start_bridge_with(
        registry: Arc<AgentRegistry>,
        store: Option<Arc<JsonlSessionStore>>,
    ) -> (
        tokio::io::WriteHalf<tokio::io::DuplexStream>,
        tokio::io::Lines<BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>>,
        tokio::task::JoinHandle<Result<(), crate::AcpTransportError>>,
        Arc<AgentRegistry>,
    ) {
        let (client, server) = duplex(64 * 1024);
        let (client_read, client_write) = tokio::io::split(client);
        let (server_read, server_write) = tokio::io::split(server);
        let transport = AcpNdjsonTransport::new(BufReader::new(server_read), server_write);
        let mut bridge = AcpBridge::new(
            transport,
            Arc::clone(&registry),
            "mock".into(),
            "mock".into(),
        );
        if let Some(store) = store {
            bridge = bridge.with_sessions(store);
        }
        bridge.bind();
        let serve = tokio::spawn({
            let bridge = bridge.clone();
            async move { bridge.serve().await }
        });
        (
            client_write,
            BufReader::new(client_read).lines(),
            serve,
            registry,
        )
    }

    async fn start_bridge() -> (
        tokio::io::WriteHalf<tokio::io::DuplexStream>,
        tokio::io::Lines<BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>>,
        tokio::task::JoinHandle<Result<(), crate::AcpTransportError>>,
        Arc<AgentRegistry>,
    ) {
        start_bridge_with(Arc::new(registry_with_text("unused")), None).await
    }

    async fn write_line(writer: &mut tokio::io::WriteHalf<tokio::io::DuplexStream>, line: &str) {
        writer.write_all(line.as_bytes()).await.unwrap();
        writer.write_all(b"\n").await.unwrap();
    }

    async fn create_session(
        writer: &mut tokio::io::WriteHalf<tokio::io::DuplexStream>,
        lines: &mut tokio::io::Lines<BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>>,
    ) -> String {
        write_line(
            writer,
            r#"{"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd":"/tmp","mcpServers":[]}}"#,
        )
        .await;
        let line = lines.next_line().await.unwrap().expect("session/new");
        let value: Value = serde_json::from_str(&line).unwrap();
        value["result"]["sessionId"]
            .as_str()
            .expect("sessionId")
            .to_string()
    }

    async fn read_until_id(
        lines: &mut tokio::io::Lines<BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>>,
        id: i64,
    ) -> (Vec<String>, Value) {
        let mut updates = Vec::new();
        loop {
            let line = lines.next_line().await.unwrap().expect("rpc line");
            let value: Value = serde_json::from_str(&line).unwrap();
            if value.get("id") == Some(&json!(id)) {
                return (updates, value);
            }
            if value.get("method") == Some(&json!("session/update")) {
                updates.push(line);
            }
        }
    }

    fn prompt_request(id: i64, session_id: &str, prompt: Value) -> String {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "session/prompt",
            "params": {
                "sessionId": session_id,
                "prompt": prompt,
            }
        })
        .to_string()
    }

    fn last_model_user_text(adapter: &MockAdapter) -> String {
        let requests = adapter.requests.lock().expect("requests");
        let last = requests.last().expect("model request");
        let message = last.messages.last().expect("model message");
        message
            .content
            .iter()
            .find_map(|block| match block {
                ContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .expect("user text")
    }

    fn register_echo(tools: &mut ToolRuntime) {
        tools.register(ToolDefinition {
            name: "echo".into(),
            description: "echo".into(),
            parameters: json!({"type": "object"}),
            execute: Box::new(|_args, _exec| Box::pin(async move { Ok(json!({"ok": true})) })),
            render: Box::new(|_, _| Vec::new()),
            is_concurrency_safe: None,
        });
    }

    async fn handshake(
        writer: &mut tokio::io::WriteHalf<tokio::io::DuplexStream>,
        lines: &mut tokio::io::Lines<BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>>,
    ) {
        writer
            .write_all(
                br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":0}}"#,
            )
            .await
            .unwrap();
        writer.write_all(b"\n").await.unwrap();
        let _ = lines.next_line().await.unwrap().expect("initialize");
    }

    #[tokio::test]
    async fn initialize_advertises_automation_only_agent() {
        let (mut writer, mut lines, serve, _registry) = start_bridge().await;
        writer
            .write_all(
                br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":0}}"#,
            )
            .await
            .unwrap();
        writer.write_all(b"\n").await.unwrap();
        let line = lines.next_line().await.unwrap().expect("initialize");
        assert_eq!(line, HANDSHAKE_INITIALIZE_LINE);
        let value: Value = serde_json::from_str(&line).unwrap();
        let result = value.get("result").unwrap();
        assert!(result.get("sessionCapabilities").is_none());
        assert!(result.get("mcpCapabilities").is_none());
        writer.shutdown().await.unwrap();
        let _ = serve.await;
    }

    #[tokio::test]
    async fn authenticate_is_noop_object() {
        let (mut writer, mut lines, serve, _registry) = start_bridge().await;
        writer
            .write_all(
                br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":0}}"#,
            )
            .await
            .unwrap();
        writer.write_all(b"\n").await.unwrap();
        let _ = lines.next_line().await.unwrap().expect("initialize");
        writer
            .write_all(
                br#"{"jsonrpc":"2.0","id":2,"method":"authenticate","params":{"methodId":"unused"}}"#,
            )
            .await
            .unwrap();
        writer.write_all(b"\n").await.unwrap();
        let line = lines.next_line().await.unwrap().expect("authenticate");
        let value: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["id"], json!(2));
        assert_eq!(value["result"], json!({}));
        writer.shutdown().await.unwrap();
        let _ = serve.await;
    }

    #[tokio::test]
    async fn session_load_is_method_not_found() {
        let (mut writer, mut lines, serve, _registry) = start_bridge().await;
        writer
            .write_all(
                br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":0}}"#,
            )
            .await
            .unwrap();
        writer.write_all(b"\n").await.unwrap();
        let _ = lines.next_line().await.unwrap().expect("initialize");
        writer
            .write_all(br#"{"jsonrpc":"2.0","id":2,"method":"session/load","params":{}}"#)
            .await
            .unwrap();
        writer.write_all(b"\n").await.unwrap();
        let line = lines.next_line().await.unwrap().expect("session/load");
        let value: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["id"], json!(2));
        assert_eq!(value["error"]["code"], json!(-32601));
        assert_eq!(
            value["error"]["message"],
            json!("\"Method not found\": session/load")
        );
        assert_eq!(value["error"]["data"]["method"], json!("session/load"));
        writer.shutdown().await.unwrap();
        let _ = serve.await;
    }

    #[tokio::test]
    async fn session_new_requires_absolute_cwd() {
        let (_lock, previous) = pin_session_root();
        let (mut writer, mut lines, serve, _registry) = start_bridge().await;
        handshake(&mut writer, &mut lines).await;
        writer
            .write_all(
                br#"{"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd":"relative","mcpServers":[]}}"#,
            )
            .await
            .unwrap();
        writer.write_all(b"\n").await.unwrap();
        let line = lines.next_line().await.unwrap().expect("session/new");
        let value: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["id"], json!(2));
        assert_eq!(value["error"]["code"], json!(-32602));
        assert_eq!(
            value["error"]["message"],
            json!("Invalid params: cwd must be an absolute path: relative")
        );
        assert!(value["error"].get("data").is_none());
        writer.shutdown().await.unwrap();
        let _ = serve.await;
        restore_session_root(previous);
    }

    #[tokio::test]
    async fn session_new_rejects_additional_directories() {
        let (_lock, previous) = pin_session_root();
        let (mut writer, mut lines, serve, _registry) = start_bridge().await;
        handshake(&mut writer, &mut lines).await;
        writer
            .write_all(
                br#"{"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd":"/tmp","mcpServers":[],"additionalDirectories":["/extra-dir"]}}"#,
            )
            .await
            .unwrap();
        writer.write_all(b"\n").await.unwrap();
        let line = lines.next_line().await.unwrap().expect("session/new");
        assert_eq!(line, REJECT_EXTRA_DIRS_ERROR_LINE);
        let value: Value = serde_json::from_str(&line).unwrap();
        assert!(value["error"].get("data").is_none());
        writer.shutdown().await.unwrap();
        let _ = serve.await;
        restore_session_root(previous);
    }

    #[tokio::test]
    async fn session_new_rejects_mcp_servers() {
        let (_lock, previous) = pin_session_root();
        let (mut writer, mut lines, serve, _registry) = start_bridge().await;
        handshake(&mut writer, &mut lines).await;
        writer
            .write_all(
                br#"{"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd":"/tmp","mcpServers":[{"name":"fs","command":"node"}]}}"#,
            )
            .await
            .unwrap();
        writer.write_all(b"\n").await.unwrap();
        let line = lines.next_line().await.unwrap().expect("session/new");
        let value: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["id"], json!(2));
        assert_eq!(value["error"]["code"], json!(-32602));
        assert_eq!(
            value["error"]["message"],
            json!("Invalid params: mcpServers is not supported")
        );
        assert!(value["error"].get("data").is_none());
        writer.shutdown().await.unwrap();
        let _ = serve.await;
        restore_session_root(previous);
    }

    #[tokio::test]
    async fn session_new_returns_session_id() {
        let (_lock, previous) = pin_session_root();
        let (mut writer, mut lines, serve, registry) = start_bridge().await;
        handshake(&mut writer, &mut lines).await;
        writer
            .write_all(
                br#"{"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd":"/tmp","mcpServers":[]}}"#,
            )
            .await
            .unwrap();
        writer.write_all(b"\n").await.unwrap();
        let line = lines.next_line().await.unwrap().expect("session/new");
        let value: Value = serde_json::from_str(&line).unwrap();
        let session_id = value["result"]["sessionId"]
            .as_str()
            .expect("sessionId string");
        assert!(!session_id.is_empty());
        assert!(session_id.starts_with("acp-"), "sessionId={session_id}");
        assert!(registry.get(session_id).is_some());
        writer.shutdown().await.unwrap();
        let _ = serve.await;
        restore_session_root(previous);
    }

    #[tokio::test]
    async fn empty_additional_directories_is_ok() {
        let (_lock, previous) = pin_session_root();
        let (mut writer, mut lines, serve, _registry) = start_bridge().await;
        handshake(&mut writer, &mut lines).await;
        writer
            .write_all(
                br#"{"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd":"/tmp","mcpServers":[],"additionalDirectories":[]}}"#,
            )
            .await
            .unwrap();
        writer.write_all(b"\n").await.unwrap();
        let line = lines.next_line().await.unwrap().expect("session/new");
        let value: Value = serde_json::from_str(&line).unwrap();
        let session_id = value["result"]["sessionId"]
            .as_str()
            .expect("sessionId string");
        assert!(!session_id.is_empty());
        writer.shutdown().await.unwrap();
        let _ = serve.await;
        restore_session_root(previous);
    }

    #[tokio::test]
    async fn prompt_emits_committed_text_and_end_turn() {
        let (_lock, previous) = pin_session_root();
        let (registry, _adapter) = registry_with_scripts(
            vec![MockScript::Chunks(text_response("hello there"))],
            |_| {},
        );
        let root = std::env::var("DSH_SESSION_ROOT").expect("pinned root");
        let store = Arc::new(JsonlSessionStore::with_root(&root));
        let (mut writer, mut lines, serve, _registry) =
            start_bridge_with(Arc::new(registry), Some(Arc::clone(&store))).await;
        handshake(&mut writer, &mut lines).await;
        let session_id = create_session(&mut writer, &mut lines).await;
        write_line(
            &mut writer,
            &prompt_request(3, &session_id, json!([{"type":"text","text":"say hello"}])),
        )
        .await;
        let (updates, response) = read_until_id(&mut lines, 3).await;
        assert_eq!(updates.len(), 1, "{updates:?}");
        assert_eq!(
            updates[0],
            format!(
                r#"{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"{session_id}","update":{{"sessionUpdate":"agent_message_chunk","content":{{"type":"text","text":"hello there"}}}}}}}}"#
            )
        );
        assert_eq!(response["id"], json!(3));
        assert_eq!(response["result"]["stopReason"], json!("end_turn"));
        let jsonl = store.path_for(&SessionId::new(session_id));
        assert!(jsonl.is_file(), "{}", jsonl.display());
        writer.shutdown().await.unwrap();
        let _ = serve.await;
        restore_session_root(previous);
    }

    #[tokio::test]
    async fn max_tokens_prompt_settles_end_turn() {
        let (_lock, previous) = pin_session_root();
        let (registry, _adapter) = registry_with_scripts(
            vec![MockScript::Chunks(max_tokens_response("cut off"))],
            |_| {},
        );
        let (mut writer, mut lines, serve, _registry) =
            start_bridge_with(Arc::new(registry), None).await;
        handshake(&mut writer, &mut lines).await;
        let session_id = create_session(&mut writer, &mut lines).await;
        write_line(
            &mut writer,
            &prompt_request(3, &session_id, json!([{"type":"text","text":"go"}])),
        )
        .await;
        let (updates, response) = read_until_id(&mut lines, 3).await;
        assert_eq!(updates.len(), 1, "{updates:?}");
        let update: Value = serde_json::from_str(&updates[0]).unwrap();
        assert_eq!(
            update["params"]["update"]["content"]["text"],
            json!("cut off")
        );
        assert_eq!(response["result"]["stopReason"], json!("end_turn"));
        writer.shutdown().await.unwrap();
        let _ = serve.await;
        restore_session_root(previous);
    }

    #[tokio::test]
    async fn empty_prompt_is_invalid_params() {
        let (_lock, previous) = pin_session_root();
        let (mut writer, mut lines, serve, _registry) = start_bridge().await;
        handshake(&mut writer, &mut lines).await;
        let session_id = create_session(&mut writer, &mut lines).await;
        write_line(
            &mut writer,
            &prompt_request(3, &session_id, json!([{"type":"text","text":"  "}])),
        )
        .await;
        let (updates, response) = read_until_id(&mut lines, 3).await;
        assert!(updates.is_empty(), "{updates:?}");
        assert_eq!(response["id"], json!(3));
        assert_eq!(response["error"]["code"], json!(-32602));
        assert_eq!(
            response["error"]["message"],
            json!("Invalid params: empty prompt")
        );
        writer.shutdown().await.unwrap();
        let _ = serve.await;
        restore_session_root(previous);
    }

    #[tokio::test]
    async fn image_prompt_is_invalid_params() {
        let (_lock, previous) = pin_session_root();
        let (mut writer, mut lines, serve, _registry) = start_bridge().await;
        handshake(&mut writer, &mut lines).await;
        let session_id = create_session(&mut writer, &mut lines).await;
        write_line(
            &mut writer,
            &prompt_request(
                3,
                &session_id,
                json!([{"type":"image","data":"","mimeType":"image/png"}]),
            ),
        )
        .await;
        let (_updates, response) = read_until_id(&mut lines, 3).await;
        assert_eq!(response["error"]["code"], json!(-32602));
        assert_eq!(
            response["error"]["message"],
            json!("Invalid params: only text and resource_link prompt content is supported")
        );
        writer.shutdown().await.unwrap();
        let _ = serve.await;
        restore_session_root(previous);
    }

    #[tokio::test]
    async fn unknown_session_prompt_is_invalid_params() {
        let (_lock, previous) = pin_session_root();
        let (mut writer, mut lines, serve, _registry) = start_bridge().await;
        handshake(&mut writer, &mut lines).await;
        let _ = create_session(&mut writer, &mut lines).await;
        write_line(
            &mut writer,
            &prompt_request(3, "missing", json!([{"type":"text","text":"go"}])),
        )
        .await;
        let (_updates, response) = read_until_id(&mut lines, 3).await;
        assert_eq!(response["error"]["code"], json!(-32602));
        assert_eq!(
            response["error"]["message"],
            json!("Invalid params: unknown session: missing")
        );
        writer.shutdown().await.unwrap();
        let _ = serve.await;
        restore_session_root(previous);
    }

    #[tokio::test]
    async fn resource_link_reaches_the_model_as_bracketed_text() {
        let (_lock, previous) = pin_session_root();
        let (registry, adapter) =
            registry_with_scripts(vec![MockScript::Chunks(text_response("done"))], |_| {});
        let (mut writer, mut lines, serve, _registry) =
            start_bridge_with(Arc::new(registry), None).await;
        handshake(&mut writer, &mut lines).await;
        let session_id = create_session(&mut writer, &mut lines).await;
        let name = "notes.md";
        let uri = "file:///tmp/notes.md";
        write_line(
            &mut writer,
            &prompt_request(
                3,
                &session_id,
                json!([{"type":"resource_link","name":name,"uri":uri}]),
            ),
        )
        .await;
        let (_updates, response) = read_until_id(&mut lines, 3).await;
        assert_eq!(response["result"]["stopReason"], json!("end_turn"));
        let expected = acp_prompt_to_text(&[AcpContentBlock::ResourceLink {
            name: name.into(),
            uri: uri.into(),
        }]);
        assert_eq!(last_model_user_text(&adapter), expected);
        writer.shutdown().await.unwrap();
        let _ = serve.await;
        restore_session_root(previous);
    }

    #[tokio::test]
    async fn failed_turn_rejects_prompt() {
        let (_lock, previous) = pin_session_root();
        let (registry, _adapter) = registry_with_scripts(
            vec![MockScript::Fail(LlmError::new("boom", "UNKNOWN"))],
            |_| {},
        );
        let (mut writer, mut lines, serve, _registry) =
            start_bridge_with(Arc::new(registry), None).await;
        handshake(&mut writer, &mut lines).await;
        let session_id = create_session(&mut writer, &mut lines).await;
        write_line(
            &mut writer,
            &prompt_request(3, &session_id, json!([{"type":"text","text":"go"}])),
        )
        .await;
        let (_updates, response) = read_until_id(&mut lines, 3).await;
        assert_eq!(response["error"]["code"], json!(-32603));
        assert_eq!(
            response["error"]["message"],
            json!("Internal error: turn failed: boom")
        );
        writer.shutdown().await.unwrap();
        let _ = serve.await;
        restore_session_root(previous);
    }

    #[tokio::test]
    async fn does_not_emit_tool_presentation_updates() {
        let (_lock, previous) = pin_session_root();
        let (registry, _adapter) = registry_with_scripts(
            vec![
                MockScript::Chunks(tool_call_response("c1", "echo", &json!({}), None)),
                MockScript::Chunks(text_response("done")),
            ],
            register_echo,
        );
        let (mut writer, mut lines, serve, _registry) =
            start_bridge_with(Arc::new(registry), None).await;
        handshake(&mut writer, &mut lines).await;
        let session_id = create_session(&mut writer, &mut lines).await;
        write_line(
            &mut writer,
            &prompt_request(3, &session_id, json!([{"type":"text","text":"go"}])),
        )
        .await;
        let (updates, response) = read_until_id(&mut lines, 3).await;
        assert_eq!(updates.len(), 1, "{updates:?}");
        let update: Value = serde_json::from_str(&updates[0]).unwrap();
        assert_eq!(
            update["params"]["update"]["sessionUpdate"],
            json!("agent_message_chunk")
        );
        assert_eq!(update["params"]["update"]["content"]["text"], json!("done"));
        assert_eq!(response["result"]["stopReason"], json!("end_turn"));
        writer.shutdown().await.unwrap();
        let _ = serve.await;
        restore_session_root(previous);
    }
}
