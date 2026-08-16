//! Newline-delimited JSON-RPC 2.0 over async byte streams.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::oneshot;

/// JSON-RPC version written on every outbound frame.
pub const JSONRPC_VERSION: &str = "2.0";
/// Missing request handler.
pub const ERR_METHOD_NOT_FOUND: i64 = -32601;
/// Handler returned `Err`.
pub const ERR_INTERNAL: i64 = -32603;

/// JSON-RPC request id.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcId {
    /// String id (`req_…`).
    String(String),
    /// Numeric id.
    Number(i64),
}

/// Peer error or local write/close failure.
#[derive(Debug, thiserror::Error)]
pub enum JsonRpcResponseError {
    /// Error object from a response frame.
    #[error("{message}")]
    Response {
        /// Wire `error.code` when present.
        code: Option<i64>,
        /// Wire `error.message`.
        message: String,
        /// Optional `error.data`.
        data: Option<Value>,
    },
    /// Local IO or closed transport.
    #[error("{0}")]
    Transport(String),
}

impl JsonRpcResponseError {
    /// Wire `error.code` for [`Self::Response`]; `None` for [`Self::Transport`].
    #[must_use]
    pub fn code(&self) -> Option<i64> {
        match self {
            Self::Response { code, .. } => *code,
            Self::Transport(_) => None,
        }
    }

    /// Wire `error.message`, or the local transport failure text.
    #[must_use]
    pub fn message(&self) -> &str {
        match self {
            Self::Response { message, .. } => message,
            Self::Transport(message) => message,
        }
    }

    /// Wire `error.data` for [`Self::Response`]; `None` for [`Self::Transport`].
    #[must_use]
    pub fn data(&self) -> Option<&Value> {
        match self {
            Self::Response { data, .. } => data.as_ref(),
            Self::Transport(_) => None,
        }
    }
}

/// Decoded inbound line.
#[derive(Clone, Debug, PartialEq)]
pub enum DecodedFrame {
    /// `id` + `method`.
    Request {
        /// Request id.
        id: JsonRpcId,
        /// Method name.
        method: String,
        /// Normalized params object.
        params: Value,
    },
    /// `id` without `method`.
    Response {
        /// Request id.
        id: JsonRpcId,
        /// Result member when present.
        result: Option<Value>,
        /// Error member when present.
        error: Option<Value>,
    },
    /// `method` without `id`.
    Notification {
        /// Method name.
        method: String,
        /// Normalized params object.
        params: Value,
    },
}

impl DecodedFrame {
    /// Request or response id.
    #[must_use]
    pub fn id(&self) -> Option<&JsonRpcId> {
        match self {
            Self::Request { id, .. } | Self::Response { id, .. } => Some(id),
            Self::Notification { .. } => None,
        }
    }

    /// Request or notification method name.
    #[must_use]
    pub fn method(&self) -> Option<&str> {
        match self {
            Self::Request { method, .. } | Self::Notification { method, .. } => Some(method),
            Self::Response { .. } => None,
        }
    }

    /// Normalized params for a request or notification.
    #[must_use]
    pub fn params(&self) -> Option<&Value> {
        match self {
            Self::Request { params, .. } | Self::Notification { params, .. } => Some(params),
            Self::Response { .. } => None,
        }
    }

    /// Success `result` member when this is a response that includes it.
    #[must_use]
    pub fn result(&self) -> Option<&Value> {
        match self {
            Self::Response { result, .. } => result.as_ref(),
            Self::Request { .. } | Self::Notification { .. } => None,
        }
    }

    /// Error member when this is a response that includes it.
    #[must_use]
    pub fn error(&self) -> Option<&Value> {
        match self {
            Self::Response { error, .. } => error.as_ref(),
            Self::Request { .. } | Self::Notification { .. } => None,
        }
    }
}

fn object_params(params: Option<&Value>) -> Value {
    match params {
        Some(obj) if obj.is_object() => obj.clone(),
        _ => json!({}),
    }
}

/// Encode a request frame ending in `\n`.
#[must_use]
pub fn encode_request(id: &JsonRpcId, method: &str, params: &Value) -> String {
    let value = json!({"jsonrpc": JSONRPC_VERSION, "id": id, "method": method, "params": params});
    format!("{value}\n")
}

/// Encode a notification frame ending in `\n`.
#[must_use]
pub fn encode_notification(method: &str, params: Option<&Value>) -> String {
    let value = match params {
        None => json!({"jsonrpc": JSONRPC_VERSION, "method": method}),
        Some(params) => json!({"jsonrpc": JSONRPC_VERSION, "method": method, "params": params}),
    };
    format!("{value}\n")
}

/// Encode a success response ending in `\n`.
#[must_use]
pub fn encode_result(id: &JsonRpcId, result: &Value) -> String {
    let value = json!({"jsonrpc": JSONRPC_VERSION, "id": id, "result": result});
    format!("{value}\n")
}

/// Encode an error response ending in `\n`.
#[must_use]
pub fn encode_error(id: &JsonRpcId, code: i64, message: &str) -> String {
    let value =
        json!({"jsonrpc": JSONRPC_VERSION, "id": id, "error": {"code": code, "message": message}});
    format!("{value}\n")
}

/// Decode one trimmed line. Malformed JSON and non-objects yield `None`.
#[must_use]
pub fn decode_line(line: &str) -> Option<DecodedFrame> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let value: Value = serde_json::from_str(line).ok()?;
    let obj = value.as_object()?;
    let id = match obj.get("id") {
        Some(Value::String(s)) => Some(JsonRpcId::String(s.clone())),
        Some(Value::Number(n)) => n.as_i64().map(JsonRpcId::Number),
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

/// Async request handler. `Err` becomes `-32603`.
pub type RequestHandler = Arc<
    dyn Fn(
            String,
            Value,
        ) -> Pin<Box<dyn std::future::Future<Output = Result<Value, String>> + Send>>
        + Send
        + Sync,
>;
/// Notification handler. Missing handler drops the notification.
pub type NotificationHandler = Arc<dyn Fn(String, Value) + Send + Sync>;

struct TransportInner {
    writer: tokio::sync::Mutex<Box<dyn AsyncWrite + Unpin + Send>>,
    pending: tokio::sync::Mutex<
        HashMap<JsonRpcId, oneshot::Sender<Result<Value, JsonRpcResponseError>>>,
    >,
    request: std::sync::Mutex<Option<RequestHandler>>,
    notification: std::sync::Mutex<Option<NotificationHandler>>,
    next_id: tokio::sync::Mutex<u64>,
}

/// Line-delimited endpoint over caller-owned async streams.
#[derive(Clone)]
pub struct JsonRpcLineTransport {
    inner: Arc<TransportInner>,
    reader: Arc<tokio::sync::Mutex<Option<Box<dyn AsyncBufRead + Unpin + Send>>>>,
}

impl JsonRpcLineTransport {
    /// Own the input and output halves. Reading starts in [`serve`](Self::serve).
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

    /// Install the request handler, replacing any prior handler. Synchronous so callers can register then spawn [`serve`](Self::serve).
    pub fn on_request(&self, handler: RequestHandler) {
        *self.inner.request.lock().expect("request handler") = Some(handler);
    }

    /// Install the notification handler, replacing any prior handler.
    pub fn on_notification(&self, handler: NotificationHandler) {
        *self.inner.notification.lock().expect("note handler") = Some(handler);
    }

    async fn write_frame(&self, frame: String) -> Result<(), JsonRpcResponseError> {
        let mut writer = self.inner.writer.lock().await;
        writer
            .write_all(frame.as_bytes())
            .await
            .map_err(|error| JsonRpcResponseError::Transport(error.to_string()))?;
        writer
            .flush()
            .await
            .map_err(|error| JsonRpcResponseError::Transport(error.to_string()))
    }

    /// Read NDJSON until EOF. Malformed lines are ignored.
    ///
    /// # Errors
    ///
    /// `Transport` when the input cannot be read or a reply frame cannot be written.
    pub async fn serve(&self) -> Result<(), JsonRpcResponseError> {
        let mut reader = self
            .reader
            .lock()
            .await
            .take()
            .ok_or_else(|| JsonRpcResponseError::Transport("serve already running".into()))?;
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader
                .read_line(&mut line)
                .await
                .map_err(|error| JsonRpcResponseError::Transport(error.to_string()))?;
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
                    if let Some(handler) = self.inner.notification.lock().expect("note").clone() {
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
    ) -> Result<(), JsonRpcResponseError> {
        let handler = self.inner.request.lock().expect("request").clone();
        let Some(handler) = handler else {
            return self
                .write_frame(encode_error(
                    &id,
                    ERR_METHOD_NOT_FOUND,
                    &format!("method not found: {method}"),
                ))
                .await;
        };
        match handler(method, params).await {
            Ok(result) => self.write_frame(encode_result(&id, &result)).await,
            Err(message) => {
                self.write_frame(encode_error(&id, ERR_INTERNAL, &message))
                    .await
            }
        }
    }

    async fn handle_response(&self, id: JsonRpcId, result: Option<Value>, error: Option<Value>) {
        let waiter = self.inner.pending.lock().await.remove(&id);
        let Some(waiter) = waiter else {
            return;
        };
        if let Some(error) = error {
            let code = error.get("code").and_then(Value::as_i64);
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("JSON-RPC error")
                .to_string();
            let data = error.get("data").cloned();
            let _ = waiter.send(Err(JsonRpcResponseError::Response {
                code,
                message,
                data,
            }));
            return;
        }
        let _ = waiter.send(Ok(result.unwrap_or(Value::Null)));
    }

    /// Send a request and await its response.
    ///
    /// # Errors
    ///
    /// `Response` on an error frame; `Transport` on write failure or close.
    pub async fn request(
        &self,
        method: &str,
        params: Value,
    ) -> Result<Value, JsonRpcResponseError> {
        let id = {
            let mut next = self.inner.next_id.lock().await;
            let n = *next;
            *next += 1;
            JsonRpcId::String(format!("req_{n}"))
        };
        let (tx, rx) = oneshot::channel();
        self.inner.pending.lock().await.insert(id.clone(), tx);
        self.write_frame(encode_request(&id, method, &params))
            .await?;
        rx.await
            .map_err(|_| JsonRpcResponseError::Transport("JSON-RPC transport closed".into()))?
    }

    /// Send a notification.
    ///
    /// # Errors
    ///
    /// `Transport` on write failure.
    pub async fn notify(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<(), JsonRpcResponseError> {
        self.write_frame(encode_notification(method, params.as_ref()))
            .await
    }

    /// Reject pending requests and shut down the writer so the peer `serve` reaches EOF. Safe to call more than once.
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
            let _ = waiter.send(Err(JsonRpcResponseError::Transport(message.into())));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DecodedFrame, ERR_INTERNAL, ERR_METHOD_NOT_FOUND, JSONRPC_VERSION, JsonRpcId,
        JsonRpcLineTransport, JsonRpcResponseError, decode_line, encode_error, encode_notification,
        encode_request, encode_result,
    };
    use serde_json::json;
    use std::sync::Arc;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, duplex};

    #[test]
    fn encode_request_is_one_ndjson_line_with_jsonrpc_2() {
        let line = encode_request(
            &JsonRpcId::String("req_1".into()),
            "initialize",
            &json!({"cwd": "/"}),
        );
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        let value: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(value["jsonrpc"], json!(JSONRPC_VERSION));
        assert_eq!(value["jsonrpc"], json!("2.0"));
        assert_eq!(value["id"], json!("req_1"));
        assert_eq!(value["method"], json!("initialize"));
    }

    #[test]
    fn decode_line_ignores_malformed_json_and_non_objects() {
        assert!(decode_line("not-json").is_none());
        assert!(decode_line("[]").is_none());
        assert!(decode_line("42").is_none());
        assert!(decode_line("").is_none());
        assert!(decode_line("   ").is_none());
    }

    #[test]
    fn decode_line_classifies_request_response_notification() {
        let req = decode_line(r#"{"jsonrpc":"2.0","id":"1","method":"shutdown","params":{}}"#)
            .expect("request");
        match req {
            DecodedFrame::Request { method, .. } => assert_eq!(method, "shutdown"),
            other => panic!("{other:?}"),
        }
        let n = decode_line(r#"{"jsonrpc":"2.0","method":"session.status","params":{"sessionId":"s","status":"idle"}}"#)
            .expect("note");
        match n {
            DecodedFrame::Notification { method, .. } => assert_eq!(method, "session.status"),
            other => panic!("{other:?}"),
        }
        let resp = decode_line(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#).expect("resp");
        match resp {
            DecodedFrame::Response { id, result, error } => {
                assert_eq!(id, JsonRpcId::Number(1));
                assert_eq!(result, Some(json!({})));
                assert!(error.is_none());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn encode_error_uses_minus_32601_and_minus_32603() {
        let not_found = encode_error(
            &JsonRpcId::String("a".into()),
            ERR_METHOD_NOT_FOUND,
            "method not found: nope",
        );
        let value: serde_json::Value = serde_json::from_str(not_found.trim_end()).unwrap();
        assert_eq!(value["error"]["code"], json!(-32601));
        let internal = encode_error(&JsonRpcId::Number(2), ERR_INTERNAL, "boom");
        let value: serde_json::Value = serde_json::from_str(internal.trim_end()).unwrap();
        assert_eq!(value["error"]["code"], json!(-32603));
        let _ = encode_result(&JsonRpcId::String("a".into()), &json!({"ok": true}));
        let _ = encode_notification(
            "session.status",
            Some(&json!({"sessionId":"s","status":"idle"})),
        );
    }

    #[tokio::test]
    async fn serve_ignores_malformed_lines_and_answers_unknown_method() {
        let (client, server) = duplex(64 * 1024);
        let (client_read, client_write) = tokio::io::split(client);
        let (server_read, server_write) = tokio::io::split(server);
        let transport = JsonRpcLineTransport::new(BufReader::new(server_read), server_write);
        let serve = tokio::spawn({
            let transport = transport.clone();
            async move { transport.serve().await }
        });
        let mut writer = client_write;
        writer.write_all(b"this is not json\n").await.unwrap();
        writer
            .write_all(br#"{"jsonrpc":"2.0","id":"1","method":"nope","params":{}}"#)
            .await
            .unwrap();
        writer.write_all(b"\n").await.unwrap();
        writer.shutdown().await.unwrap();
        let mut lines = BufReader::new(client_read).lines();
        let line = lines.next_line().await.unwrap().expect("error frame");
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["id"], json!("1"));
        assert_eq!(value["error"]["code"], json!(-32601));
        assert!(value["error"]["message"].as_str().unwrap().contains("nope"));
        serve.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn handler_throw_becomes_minus_32603() {
        let (client, server) = duplex(64 * 1024);
        let (client_read, client_write) = tokio::io::split(client);
        let (server_read, server_write) = tokio::io::split(server);
        let transport = JsonRpcLineTransport::new(BufReader::new(server_read), server_write);
        transport.on_request(Arc::new(|_method, _params| {
            Box::pin(async { Err("exploded".into()) })
        }));
        let serve = tokio::spawn({
            let transport = transport.clone();
            async move { transport.serve().await }
        });
        let mut writer = client_write;
        writer
            .write_all(br#"{"jsonrpc":"2.0","id":7,"method":"initialize","params":{}}"#)
            .await
            .unwrap();
        writer.write_all(b"\n").await.unwrap();
        writer.shutdown().await.unwrap();
        let mut lines = BufReader::new(client_read).lines();
        let line = lines.next_line().await.unwrap().expect("error frame");
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["error"]["code"], json!(-32603));
        assert_eq!(value["error"]["message"], json!("exploded"));
        serve.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn request_round_trip_result() {
        let (a, b) = duplex(64 * 1024);
        let (a_read, a_write) = tokio::io::split(a);
        let (b_read, b_write) = tokio::io::split(b);
        let server = JsonRpcLineTransport::new(BufReader::new(b_read), b_write);
        server.on_request(Arc::new(|method, _params| {
            Box::pin(async move {
                assert_eq!(method, "shutdown");
                Ok(json!({}))
            })
        }));
        let client = JsonRpcLineTransport::new(BufReader::new(a_read), a_write);
        let serve_server = tokio::spawn({
            let server = server.clone();
            async move { server.serve().await }
        });
        let serve_client = tokio::spawn({
            let client = client.clone();
            async move { client.serve().await }
        });
        let result = client.request("shutdown", json!({})).await.expect("ok");
        assert_eq!(result, json!({}));
        client.close().await;
        server.close().await;
        let _ = serve_server.await;
        let _ = serve_client.await;
    }

    #[test]
    fn decoded_frame_and_response_error_are_readable_through_accessors() {
        let req =
            decode_line(r#"{"jsonrpc":"2.0","id":"1","method":"shutdown","params":{"cwd":"/"}}"#)
                .expect("request");
        assert_eq!(req.id(), Some(&JsonRpcId::String("1".into())));
        assert_eq!(req.method(), Some("shutdown"));
        assert_eq!(req.params(), Some(&json!({"cwd":"/"})));
        assert!(req.result().is_none());
        assert!(req.error().is_none());

        let note = decode_line(
            r#"{"jsonrpc":"2.0","method":"session.status","params":{"sessionId":"s"}}"#,
        )
        .expect("note");
        assert!(note.id().is_none());
        assert_eq!(note.method(), Some("session.status"));
        assert_eq!(note.params(), Some(&json!({"sessionId":"s"})));
        assert!(note.result().is_none());
        assert!(note.error().is_none());

        let resp = decode_line(r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#).expect("resp");
        assert_eq!(resp.id(), Some(&JsonRpcId::Number(1)));
        assert!(resp.method().is_none());
        assert!(resp.params().is_none());
        assert_eq!(resp.result(), Some(&json!({"ok": true})));
        assert!(resp.error().is_none());

        let err_frame =
            decode_line(r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32601,"message":"nope"}}"#)
                .expect("error frame");
        assert_eq!(err_frame.id(), Some(&JsonRpcId::Number(2)));
        assert!(err_frame.method().is_none());
        assert!(err_frame.params().is_none());
        assert!(err_frame.result().is_none());
        assert_eq!(
            err_frame.error(),
            Some(&json!({"code":-32601,"message":"nope"}))
        );

        let rpc_err = JsonRpcResponseError::Response {
            code: Some(-32601),
            message: "method not found: nope".into(),
            data: Some(json!({"hint": true})),
        };
        assert_eq!(rpc_err.code(), Some(-32601));
        assert_eq!(rpc_err.message(), "method not found: nope");
        assert_eq!(rpc_err.data(), Some(&json!({"hint": true})));

        let transport = JsonRpcResponseError::Transport("broken pipe".into());
        assert!(transport.code().is_none());
        assert_eq!(transport.message(), "broken pipe");
        assert!(transport.data().is_none());
    }
}
