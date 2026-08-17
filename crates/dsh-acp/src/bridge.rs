//! ACP JSON-RPC method dispatch over NDJSON transport.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use dsh_agent::AgentRegistry;
use serde_json::value::RawValue;
use serde_json::{Value, json};

use crate::error::{AcpError, method_not_found};
use crate::rpc::{AcpNdjsonTransport, AcpTransportError, serialize_result};
use crate::types::{AuthenticateRequest, InitializeRequest, InitializeResult};

struct AcpBridgeInner {
    transport: AcpNdjsonTransport,
    #[allow(dead_code)]
    agents: Arc<AgentRegistry>,
    #[allow(dead_code)]
    provider: String,
    #[allow(dead_code)]
    model: String,
    sessions: Mutex<HashMap<String, ()>>,
}

/// Automation-only ACP server bound to one NDJSON transport.
#[derive(Clone)]
pub struct AcpBridge {
    inner: Arc<AcpBridgeInner>,
}

impl AcpBridgeInner {
    fn dispatch(&self, method: &str, params: Value) -> Result<Box<RawValue>, AcpError> {
        match method {
            "initialize" => {
                let _ = serde_json::from_value::<InitializeRequest>(params);
                Ok(serialize_result(&InitializeResult::automation_only()))
            }
            "authenticate" => {
                let _ = serde_json::from_value::<AuthenticateRequest>(params);
                Ok(serialize_result(&json!({})))
            }
            "session/cancel" => {
                let _map = self.sessions.lock().expect("sessions");
                Ok(serialize_result(&json!({})))
            }
            _ => Err(method_not_found(method)),
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
                Box::pin(async move { inner.dispatch(&method, params) })
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
mod tests {
    use super::AcpBridge;
    use crate::AcpNdjsonTransport;
    use dsh_agent::AgentRegistry;
    use dsh_llm::{LlmRuntime, MockAdapter, MockScript, text_response};
    use dsh_system_prompt::{SystemPrompt, SystemPromptConfig};
    use dsh_tools::{ToolPresentationMode, ToolRuntime};
    use serde_json::{Value, json};
    use std::sync::Arc;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, duplex};

    const HANDSHAKE_INITIALIZE_LINE: &str = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentInfo":{"name":"deepseek-harness-acp","version":"0.0.1"},"agentCapabilities":{"promptCapabilities":{"image":false,"audio":false,"embeddedContext":false}},"authMethods":[]}}"#;

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
    ) {
        let (client, server) = duplex(64 * 1024);
        let (client_read, client_write) = tokio::io::split(client);
        let (server_read, server_write) = tokio::io::split(server);
        let transport = AcpNdjsonTransport::new(BufReader::new(server_read), server_write);
        let bridge = AcpBridge::new(
            transport,
            Arc::new(registry_with_text("unused")),
            "mock".into(),
            "mock".into(),
        );
        bridge.bind();
        let serve = tokio::spawn({
            let bridge = bridge.clone();
            async move { bridge.serve().await }
        });
        (client_write, BufReader::new(client_read).lines(), serve)
    }

    #[tokio::test]
    async fn initialize_advertises_automation_only_agent() {
        let (mut writer, mut lines, serve) = start_bridge().await;
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
        let (mut writer, mut lines, serve) = start_bridge().await;
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
        let (mut writer, mut lines, serve) = start_bridge().await;
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
}
