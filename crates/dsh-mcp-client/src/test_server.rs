//! In-memory Content-Length MCP fixture for `add` / `fail` loopback tests.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncWriteExt, BufReader, duplex};
use tokio::sync::watch;

use crate::client::McpSession;
use crate::rpc::{McpRpcError, encode_frame, read_frame};

const DUPLEX_BUF: usize = 64 * 1024;

/// Loopback fixture plus recorded initialize handshake state.
pub struct LoopbackFixture {
    initialize_params: Arc<Mutex<Option<Value>>>,
    initialized_rx: watch::Receiver<bool>,
}

impl LoopbackFixture {
    /// Initialize request `params` recorded by the fixture, or `null`.
    pub fn initialize_params(&self) -> Value {
        self.initialize_params
            .lock()
            .expect("initialize params mutex")
            .clone()
            .unwrap_or(Value::Null)
    }

    /// Wait until the fixture reads `notifications/initialized`.
    pub async fn wait_initialized(&self) {
        let mut rx = self.initialized_rx.clone();
        tokio::time::timeout(Duration::from_secs(2), rx.wait_for(|flag| *flag))
            .await
            .expect("notifications/initialized")
            .expect("initialized watch");
    }
}

/// Two duplex pairs: client write → server read, server write → client read.
pub fn spawn_loopback() -> (McpSession, LoopbackFixture) {
    let (client_write, server_read) = duplex(DUPLEX_BUF);
    let (server_write, client_read) = duplex(DUPLEX_BUF);
    let initialize_params = Arc::new(Mutex::new(None));
    let (initialized_tx, initialized_rx) = watch::channel(false);
    let params_for_server = Arc::clone(&initialize_params);
    tokio::spawn(async move {
        run_fixture(server_read, server_write, params_for_server, initialized_tx).await;
    });
    let session = McpSession::from_stdio(client_read, client_write);
    (
        session,
        LoopbackFixture {
            initialize_params,
            initialized_rx,
        },
    )
}

async fn run_fixture(
    server_read: tokio::io::DuplexStream,
    mut server_write: tokio::io::DuplexStream,
    initialize_params: Arc<Mutex<Option<Value>>>,
    initialized_tx: watch::Sender<bool>,
) {
    let mut reader = BufReader::new(server_read);
    loop {
        let body = match read_frame(&mut reader).await {
            Ok(body) => body,
            Err(McpRpcError::IncompleteHeaders | McpRpcError::IncompleteBody) => break,
            Err(_) => break,
        };
        let Ok(message) = serde_json::from_slice::<Value>(&body) else {
            continue;
        };
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            continue;
        };
        let id = message.get("id").cloned();
        let params = match message.get("params") {
            Some(value) => value.clone(),
            None => json!({}),
        };
        match method {
            "initialize" => {
                *initialize_params.lock().expect("initialize params mutex") = Some(params);
                let Some(id) = id else {
                    continue;
                };
                write_result(&mut server_write, id, initialize_result()).await;
            }
            "notifications/initialized" => {
                let _ = initialized_tx.send(true);
            }
            "tools/list" => {
                let Some(id) = id else {
                    continue;
                };
                write_result(&mut server_write, id, list_tools_result()).await;
            }
            "tools/call" => {
                let Some(id) = id else {
                    continue;
                };
                write_result(&mut server_write, id, call_tool_result(&params)).await;
            }
            _ => {}
        }
    }
}

async fn write_result(writer: &mut tokio::io::DuplexStream, id: Value, result: Value) {
    let body = json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    });
    let bytes = serde_json::to_vec(&body).expect("json");
    writer
        .write_all(&encode_frame(&bytes))
        .await
        .expect("fixture write");
    writer.flush().await.expect("fixture flush");
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": "2025-03-26",
        "capabilities": { "tools": { "listChanged": true } },
        "serverInfo": { "name": "fixture-server", "version": "1.0.0" },
    })
}

fn list_tools_result() -> Value {
    json!({
        "tools": [
            {
                "name": "add",
                "description": "Adds two numbers.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "a": { "type": "number", "description": "First number" },
                        "b": { "type": "number", "description": "Second number" }
                    },
                    "required": ["a", "b"]
                }
            },
            {
                "name": "fail",
                "description": "Always returns an error.",
                "inputSchema": { "type": "object", "properties": {} }
            }
        ]
    })
}

fn call_tool_result(params: &Value) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = match params.get("arguments") {
        Some(value) => value.clone(),
        None => json!({}),
    };
    match name {
        "add" => {
            let a = args.get("a").and_then(Value::as_f64).unwrap_or(0.0);
            let b = args.get("b").and_then(Value::as_f64).unwrap_or(0.0);
            json!({
                "content": [{ "type": "text", "text": decimal_text(a + b) }]
            })
        }
        "fail" => json!({
            "content": [{ "type": "text", "text": "Something went wrong" }],
            "isError": true
        }),
        _ => json!({
            "content": [{ "type": "text", "text": "unknown tool" }],
            "isError": true
        }),
    }
}

fn decimal_text(sum: f64) -> String {
    if sum.is_finite() && sum.fract() == 0.0 && sum.abs() <= i64::MAX as f64 {
        return (sum as i64).to_string();
    }
    sum.to_string()
}
