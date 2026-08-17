//! ACP JSON-RPC method dispatch over NDJSON transport.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use dsh_agent::{AgentHandle, AgentRegistry, CreateAgentOptions};
use dsh_session::SessionId;
use serde_json::value::RawValue;
use serde_json::{Value, json};

use crate::error::{AcpError, internal_error, invalid_params, method_not_found};
use crate::rpc::{AcpNdjsonTransport, AcpTransportError, serialize_result};
use crate::types::{
    AuthenticateRequest, InitializeRequest, InitializeResult, NewSessionRequest, NewSessionResult,
};

#[allow(dead_code)]
struct SessionRecord {
    handle: AgentHandle,
    inflight: Option<()>,
}

struct AcpBridgeInner {
    transport: AcpNdjsonTransport,
    agents: Arc<AgentRegistry>,
    provider: String,
    model: String,
    sessions: Mutex<HashMap<String, SessionRecord>>,
    closed: AtomicBool,
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

impl AcpBridgeInner {
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
                closed: AtomicBool::new(false),
            }),
        }
    }

    /// Install request and notification handlers on the transport.
    pub fn bind(&self) {
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
    use crate::AcpNdjsonTransport;
    use dsh_agent::AgentRegistry;
    use dsh_llm::{LlmRuntime, MockAdapter, MockScript, text_response};
    use dsh_system_prompt::{SystemPrompt, SystemPromptConfig};
    use dsh_tools::{ToolPresentationMode, ToolRuntime};
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

    fn registry_with_text(text: &str) -> AgentRegistry {
        let adapter = Arc::new(MockAdapter::new(vec![MockScript::Chunks(text_response(
            text,
        ))]));
        let mut llm = LlmRuntime::new();
        llm.register_adapter("mock", adapter);
        AgentRegistry::new(
            llm,
            ToolRuntime::new(ToolPresentationMode::Native),
            SystemPrompt::new(SystemPromptConfig::default()).unwrap(),
        )
    }

    async fn start_bridge() -> (
        tokio::io::WriteHalf<tokio::io::DuplexStream>,
        tokio::io::Lines<BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>>,
        tokio::task::JoinHandle<Result<(), crate::AcpTransportError>>,
        Arc<AgentRegistry>,
    ) {
        let (client, server) = duplex(64 * 1024);
        let (client_read, client_write) = tokio::io::split(client);
        let (server_read, server_write) = tokio::io::split(server);
        let transport = AcpNdjsonTransport::new(BufReader::new(server_read), server_write);
        let registry = Arc::new(registry_with_text("unused"));
        let bridge = AcpBridge::new(
            transport,
            Arc::clone(&registry),
            "mock".into(),
            "mock".into(),
        );
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
}
