//! NDJSON JSON-RPC 2.0 framing for ACP. Not LSP Content-Length.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::oneshot;

use crate::error::{AcpError, ERR_INTERNAL, internal_error, method_not_found};

/// JSON-RPC version written on every outbound frame.
pub const JSONRPC_VERSION: &str = "2.0";

/// JSON-RPC request id. Variant payloads are private; use constructors.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcId {
    /// String id (`req_…`).
    String(String),
    /// Numeric id.
    Number(i64),
}

impl JsonRpcId {
    /// String request id.
    #[must_use]
    pub fn string(value: impl Into<String>) -> Self {
        Self::String(value.into())
    }

    /// Numeric request id.
    #[must_use]
    pub fn number(value: i64) -> Self {
        Self::Number(value)
    }

    /// Borrow a string id.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            Self::Number(_) => None,
        }
    }

    /// Copy a numeric id.
    #[must_use]
    pub fn as_number(&self) -> Option<i64> {
        match self {
            Self::Number(value) => Some(*value),
            Self::String(_) => None,
        }
    }
}

/// Local IO or closed transport.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct AcpTransportError(String);

/// Async request handler. `Err` becomes a JSON-RPC error response with the same id.
pub type RequestHandler = Arc<
    dyn Fn(
            String,
            Value,
        ) -> Pin<Box<dyn std::future::Future<Output = Result<Value, AcpError>> + Send>>
        + Send
        + Sync,
>;

/// Notification handler. A missing handler drops the notification.
pub type NotificationHandler = Arc<dyn Fn(String, Value) + Send + Sync>;

struct TransportInner {
    writer: tokio::sync::Mutex<Box<dyn AsyncWrite + Unpin + Send>>,
    pending: tokio::sync::Mutex<HashMap<JsonRpcId, oneshot::Sender<Result<Value, AcpError>>>>,
    request: std::sync::Mutex<Option<RequestHandler>>,
    notification: std::sync::Mutex<Option<NotificationHandler>>,
    next_id: tokio::sync::Mutex<u64>,
}

/// Line-delimited ACP JSON-RPC endpoint over caller-owned async streams.
#[derive(Clone)]
pub struct AcpNdjsonTransport {
    inner: Arc<TransportInner>,
    reader: Arc<tokio::sync::Mutex<Option<Box<dyn AsyncBufRead + Unpin + Send>>>>,
}

enum DecodedFrame {
    Request {
        id: JsonRpcId,
        method: String,
        params: Value,
    },
    Response {
        id: JsonRpcId,
        result: Option<Value>,
        error: Option<Value>,
    },
    Notification {
        method: String,
        params: Value,
    },
}

fn object_params(params: Option<&Value>) -> Value {
    match params {
        Some(obj) if obj.is_object() => obj.clone(),
        _ => json!({}),
    }
}

fn frame_line(value: &Value) -> String {
    let mut line = serde_json::to_string(value).expect("jsonrpc frame");
    line.push('\n');
    line
}

fn encode_request(id: &JsonRpcId, method: &str, params: &Value) -> String {
    frame_line(&json!({
        "jsonrpc": JSONRPC_VERSION,
        "id": id,
        "method": method,
        "params": params,
    }))
}

fn encode_notification(method: &str, params: &Value) -> String {
    frame_line(&json!({
        "jsonrpc": JSONRPC_VERSION,
        "method": method,
        "params": params,
    }))
}

fn encode_result(id: &JsonRpcId, result: &Value) -> String {
    frame_line(&json!({
        "jsonrpc": JSONRPC_VERSION,
        "id": id,
        "result": result,
    }))
}

fn encode_error(id: &JsonRpcId, error: &AcpError) -> String {
    let error_value = serde_json::to_value(error).expect("acp error");
    frame_line(&json!({
        "jsonrpc": JSONRPC_VERSION,
        "id": id,
        "error": error_value,
    }))
}

fn decode_line(line: &str) -> Option<DecodedFrame> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let value: Value = serde_json::from_str(line).ok()?;
    let obj = value.as_object()?;
    let id = match obj.get("id") {
        Some(Value::String(s)) => Some(JsonRpcId::string(s.clone())),
        Some(Value::Number(n)) => n.as_i64().map(JsonRpcId::number),
        _ => None,
    };
    let method = obj
        .get("method")
        .and_then(Value::as_str)
        .map(str::to_string);
    match (id, method) {
        (Some(id), Some(method)) => Some(DecodedFrame::Request {
            id,
            method,
            params: object_params(obj.get("params")),
        }),
        (Some(id), None) => Some(DecodedFrame::Response {
            id,
            result: obj.get("result").cloned(),
            error: obj.get("error").cloned(),
        }),
        (None, Some(method)) => Some(DecodedFrame::Notification {
            method,
            params: object_params(obj.get("params")),
        }),
        (None, None) => None,
    }
}

impl AcpNdjsonTransport {
    /// Own the input and output halves. Reading starts in [`serve`](Self::serve).
    #[must_use]
    pub fn new<R, W>(input: R, output: W) -> Self
    where
        R: AsyncBufRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        Self {
            inner: Arc::new(TransportInner {
                writer: tokio::sync::Mutex::new(Box::new(output)),
                pending: tokio::sync::Mutex::new(HashMap::new()),
                request: std::sync::Mutex::new(None),
                notification: std::sync::Mutex::new(None),
                next_id: tokio::sync::Mutex::new(1),
            }),
            reader: Arc::new(tokio::sync::Mutex::new(Some(Box::new(input)))),
        }
    }

    /// Install the request handler, replacing any prior handler.
    pub fn on_request(&self, handler: RequestHandler) {
        *self.inner.request.lock().expect("request handler") = Some(handler);
    }

    /// Install the notification handler, replacing any prior handler.
    pub fn on_notification(&self, handler: NotificationHandler) {
        *self.inner.notification.lock().expect("note handler") = Some(handler);
    }

    async fn write_frame(&self, frame: String) -> Result<(), AcpTransportError> {
        let mut writer = self.inner.writer.lock().await;
        writer
            .write_all(frame.as_bytes())
            .await
            .map_err(|error| AcpTransportError(error.to_string()))?;
        writer
            .flush()
            .await
            .map_err(|error| AcpTransportError(error.to_string()))
    }

    /// Read NDJSON until EOF. Empty and whitespace-only lines are skipped. Malformed lines are ignored.
    ///
    /// # Errors
    ///
    /// Transport read/write failure.
    pub async fn serve(&self) -> Result<(), AcpTransportError> {
        let mut reader = self
            .reader
            .lock()
            .await
            .take()
            .ok_or_else(|| AcpTransportError("serve already running".to_string()))?;
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader
                .read_line(&mut line)
                .await
                .map_err(|error| AcpTransportError(error.to_string()))?;
            if n == 0 {
                self.fail_pending("JSON-RPC input closed").await;
                return Ok(());
            }
            let Some(frame) = decode_line(&line) else {
                continue;
            };
            match frame {
                DecodedFrame::Request { id, method, params } => {
                    self.handle_request(id, method, params).await?;
                }
                DecodedFrame::Response { id, result, error } => {
                    self.handle_response(id, result, error).await;
                }
                DecodedFrame::Notification { method, params } => {
                    let handler = self.inner.notification.lock().expect("note").clone();
                    if let Some(handler) = handler {
                        handler(method, params);
                    }
                }
            }
        }
    }

    async fn handle_request(
        &self,
        id: JsonRpcId,
        method: String,
        params: Value,
    ) -> Result<(), AcpTransportError> {
        let handler = self.inner.request.lock().expect("request").clone();
        let Some(handler) = handler else {
            return self
                .write_frame(encode_error(&id, &method_not_found(&method)))
                .await;
        };
        match handler(method, params).await {
            Ok(result) => self.write_frame(encode_result(&id, &result)).await,
            Err(error) => self.write_frame(encode_error(&id, &error)).await,
        }
    }

    async fn handle_response(&self, id: JsonRpcId, result: Option<Value>, error: Option<Value>) {
        let waiter = self.inner.pending.lock().await.remove(&id);
        let Some(waiter) = waiter else {
            return;
        };
        if let Some(error) = error {
            let code = error
                .get("code")
                .and_then(Value::as_i64)
                .unwrap_or(ERR_INTERNAL);
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("JSON-RPC error")
                .to_string();
            let data = error.get("data").cloned();
            let _ = waiter.send(Err(AcpError::from_parts(code, message, data)));
            return;
        }
        let _ = waiter.send(Ok(result.unwrap_or(Value::Null)));
    }

    /// Send a notification.
    ///
    /// # Errors
    ///
    /// Transport write failure.
    pub async fn notify(&self, method: &str, params: Value) -> Result<(), AcpTransportError> {
        self.write_frame(encode_notification(method, &params)).await
    }

    /// Send a request with id `req_{n}` starting at 1 and await its response.
    ///
    /// # Errors
    ///
    /// Peer JSON-RPC error, or a transport failure as `-32603`.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, AcpError> {
        let id = {
            let mut next = self.inner.next_id.lock().await;
            let n = *next;
            *next += 1;
            JsonRpcId::string(format!("req_{n}"))
        };
        let (tx, rx) = oneshot::channel();
        self.inner.pending.lock().await.insert(id.clone(), tx);
        self.write_frame(encode_request(&id, method, &params))
            .await
            .map_err(|error| internal_error(&error.to_string()))?;
        rx.await
            .map_err(|_| internal_error("JSON-RPC transport closed"))?
    }

    /// Reject pending requests and shut down the writer so the peer `serve` reaches EOF.
    pub async fn close(&self) {
        self.fail_pending("JSON-RPC transport closed").await;
        let mut writer = self.inner.writer.lock().await;
        let _ = writer.shutdown().await;
    }

    async fn fail_pending(&self, message: &str) {
        let mut pending = self.inner.pending.lock().await;
        let waiters: Vec<_> = pending.drain().map(|(_, tx)| tx).collect();
        drop(pending);
        for waiter in waiters {
            let _ = waiter.send(Err(internal_error(message)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AcpNdjsonTransport, JSONRPC_VERSION, JsonRpcId};
    use crate::error::{internal_error, invalid_params, method_not_found};
    use serde_json::{Value, json};
    use std::sync::Arc;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, duplex};

    #[test]
    fn method_not_found_message_matches_typescript() {
        let err = method_not_found("session/load");
        assert_eq!(err.code(), -32601);
        assert_eq!(err.message(), "\"Method not found\": session/load");
        assert_eq!(err.data().unwrap()["method"], "session/load");
        let value = serde_json::to_value(&err).unwrap();
        assert_eq!(value["data"]["method"], json!("session/load"));
    }

    #[test]
    fn invalid_params_message_matches_typescript() {
        let err = invalid_params("additionalDirectories is not supported");
        assert_eq!(err.code(), -32602);
        assert_eq!(
            err.message(),
            "Invalid params: additionalDirectories is not supported"
        );
        let value = serde_json::to_value(&err).unwrap();
        assert!(value.get("data").is_none());
    }

    #[test]
    fn internal_error_message_matches_typescript() {
        let err = internal_error("the ACP bridge has been disposed");
        assert_eq!(err.code(), -32603);
        assert_eq!(
            err.message(),
            "Internal error: the ACP bridge has been disposed"
        );
        let value = serde_json::to_value(&err).unwrap();
        assert!(value.get("data").is_none());
    }

    #[tokio::test]
    async fn encodes_one_json_object_per_line() {
        let (client, server) = duplex(64 * 1024);
        let (client_read, _client_write) = tokio::io::split(client);
        let (server_read, server_write) = tokio::io::split(server);
        let transport = AcpNdjsonTransport::new(BufReader::new(server_read), server_write);
        let pending = tokio::spawn({
            let transport = transport.clone();
            async move { transport.request("initialize", json!({"cwd": "/"})).await }
        });
        let mut line = String::new();
        BufReader::new(client_read)
            .read_line(&mut line)
            .await
            .unwrap();
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        let value: Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(value["jsonrpc"], json!(JSONRPC_VERSION));
        assert_eq!(value["id"], json!("req_1"));
        assert_eq!(value["method"], json!("initialize"));
        transport.close().await;
        let _ = pending.await;
    }

    #[tokio::test]
    async fn ping_handler_round_trips_numeric_id() {
        let (client, server) = duplex(64 * 1024);
        let (client_read, client_write) = tokio::io::split(client);
        let (server_read, server_write) = tokio::io::split(server);
        let transport = AcpNdjsonTransport::new(BufReader::new(server_read), server_write);
        transport.on_request(Arc::new(|method, params| {
            Box::pin(async move {
                assert_eq!(method, "ping");
                assert_eq!(params, json!({}));
                Ok(json!({"ok": true}))
            })
        }));
        let serve = tokio::spawn({
            let transport = transport.clone();
            async move { transport.serve().await }
        });
        let mut writer = client_write;
        writer.write_all(b"\n  \n").await.unwrap();
        writer
            .write_all(br#"{"jsonrpc":"2.0","id":1,"method":"ping","params":{}}"#)
            .await
            .unwrap();
        writer.write_all(b"\n").await.unwrap();
        let mut lines = BufReader::new(client_read).lines();
        let line = lines.next_line().await.unwrap().expect("response");
        let value: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["jsonrpc"], json!("2.0"));
        assert_eq!(value["id"], json!(1));
        assert_eq!(value["result"], json!({"ok": true}));
        writer.shutdown().await.unwrap();
        let _ = serve.await;
    }

    #[tokio::test]
    async fn handler_err_echoes_id_and_omits_invalid_params_data() {
        let (client, server) = duplex(64 * 1024);
        let (client_read, client_write) = tokio::io::split(client);
        let (server_read, server_write) = tokio::io::split(server);
        let transport = AcpNdjsonTransport::new(BufReader::new(server_read), server_write);
        transport.on_request(Arc::new(|_method, _params| {
            Box::pin(async { Err(invalid_params("additionalDirectories is not supported")) })
        }));
        let serve = tokio::spawn({
            let transport = transport.clone();
            async move { transport.serve().await }
        });
        let mut writer = client_write;
        writer
            .write_all(br#"{"jsonrpc":"2.0","id":2,"method":"session/new","params":{}}"#)
            .await
            .unwrap();
        writer.write_all(b"\n").await.unwrap();
        let mut lines = BufReader::new(client_read).lines();
        let line = lines.next_line().await.unwrap().expect("error");
        let value: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["id"], json!(2));
        assert_eq!(value["error"]["code"], json!(-32602));
        assert_eq!(
            value["error"]["message"],
            json!("Invalid params: additionalDirectories is not supported")
        );
        assert!(value["error"].get("data").is_none());
        writer.shutdown().await.unwrap();
        let _ = serve.await;
    }

    #[test]
    fn json_rpc_id_constructors_hide_variant_fields() {
        assert_eq!(JsonRpcId::string("req_1").as_str(), Some("req_1"));
        assert_eq!(JsonRpcId::number(3).as_number(), Some(3));
        assert!(JsonRpcId::string("req_1").as_number().is_none());
        assert!(JsonRpcId::number(3).as_str().is_none());
    }
}
