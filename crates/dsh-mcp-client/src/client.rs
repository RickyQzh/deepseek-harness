//! MCP JSON-RPC session: `initialize`, `tools/list`, and `tools/call`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

use crate::rpc::{McpRpcError, encode_frame, read_frame};

const JSONRPC_VERSION: &str = "2.0";
const PROTOCOL_VERSION: &str = "2025-03-26";
const CLIENT_NAME: &str = "dsh-mcp-client";
const CLIENT_VERSION: &str = "0.0.1";
const DEFAULT_TOOL_CALL_TIMEOUT_MS: u64 = 60_000;

struct McpSessionInner {
    reader: BufReader<Box<dyn AsyncRead + Unpin + Send>>,
    writer: Box<dyn AsyncWrite + Unpin + Send>,
    next_id: u64,
}

/// MCP client session over Content-Length JSON-RPC byte streams.
///
/// Clone shares the byte streams so tool executors can call `tools/call` while
/// `sync_tools` still takes `&mut Self`.
#[derive(Clone)]
pub struct McpSession {
    inner: Arc<Mutex<McpSessionInner>>,
    tool_call_timeout_ms: Arc<AtomicU64>,
}

impl McpSession {
    pub(crate) fn new(
        server_stdout: impl AsyncRead + Unpin + Send + 'static,
        server_stdin: impl AsyncWrite + Unpin + Send + 'static,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(McpSessionInner {
                reader: BufReader::new(Box::new(server_stdout)),
                writer: Box::new(server_stdin),
                next_id: 0,
            })),
            tool_call_timeout_ms: Arc::new(AtomicU64::new(DEFAULT_TOOL_CALL_TIMEOUT_MS)),
        }
    }

    /// Per-call timeout applied by MCP tool executors.
    ///
    /// # Returns
    ///
    /// Duration from `toolCallTimeoutMs` (default 60000 ms).
    #[must_use]
    pub(crate) fn tool_call_timeout(&self) -> Duration {
        Duration::from_millis(self.tool_call_timeout_ms.load(Ordering::Relaxed))
    }

    /// Store YAML `toolCallTimeoutMs` for later `tools/call` executors.
    ///
    /// # Parameters
    ///
    /// * `ms` - Timeout in milliseconds.
    pub(crate) fn set_tool_call_timeout_ms(&self, ms: u64) {
        self.tool_call_timeout_ms.store(ms, Ordering::Relaxed);
    }

    /// Send MCP `initialize`, then `notifications/initialized`.
    ///
    /// Writes JSON-RPC request method `initialize` with `protocolVersion`
    /// `"2025-03-26"`, `capabilities` `{}`, and `clientInfo` name
    /// `dsh-mcp-client` version `0.0.1`. After a successful result, writes
    /// notification `notifications/initialized` (method set, no `id`).
    ///
    /// # Returns
    ///
    /// The initialize result JSON object.
    ///
    /// # Errors
    ///
    /// [`McpRpcError`] when framing, JSON, or a JSON-RPC error object fails.
    pub async fn initialize(&mut self) -> Result<Value, McpRpcError> {
        let mut inner = self.inner.lock().await;
        let result = inner
            .request(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": {
                        "name": CLIENT_NAME,
                        "version": CLIENT_VERSION,
                    },
                }),
            )
            .await?;
        inner
            .write_json(&json!({
                "jsonrpc": JSONRPC_VERSION,
                "method": "notifications/initialized",
            }))
            .await?;
        Ok(result)
    }

    /// Drain paginated MCP `tools/list` into drafts.
    ///
    /// Repeats `tools/list` until `nextCursor` is absent.
    ///
    /// # Returns
    ///
    /// One [`McpToolDraft`] per listed tool, in server order across pages.
    ///
    /// # Errors
    ///
    /// [`McpRpcError`] when framing, JSON, a JSON-RPC error object, or a listed
    /// tool without `name` fails.
    pub async fn list_tools(&mut self) -> Result<Vec<McpToolDraft>, McpRpcError> {
        let mut drafts = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let params = match cursor {
                Some(ref value) => json!({ "cursor": value }),
                None => json!({}),
            };
            let result = self.request("tools/list", params).await?;
            let Some(tools) = result.get("tools").and_then(Value::as_array) else {
                return Err(McpRpcError::InvalidResponse(
                    "tools/list result is missing tools".to_string(),
                ));
            };
            for tool in tools {
                drafts.push(parse_tool_draft(tool)?);
            }
            cursor = match result.get("nextCursor") {
                Some(Value::String(next)) if !next.is_empty() => Some(next.clone()),
                _ => None,
            };
            if cursor.is_none() {
                break;
            }
        }
        Ok(drafts)
    }

    /// Send MCP `tools/call` with the server's raw tool name.
    ///
    /// # Parameters
    ///
    /// * `raw_name` - MCP tool name (`add`), never an `mcp__` public name.
    /// * `arguments` - JSON-RPC `arguments` object for the tool.
    ///
    /// # Returns
    ///
    /// The MCP `tools/call` result JSON object.
    ///
    /// # Errors
    ///
    /// [`McpRpcError`] when framing, JSON, or a JSON-RPC error object fails.
    pub async fn call_tool(
        &mut self,
        raw_name: &str,
        arguments: Value,
    ) -> Result<Value, McpRpcError> {
        self.request(
            "tools/call",
            json!({
                "name": raw_name,
                "arguments": arguments,
            }),
        )
        .await
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, McpRpcError> {
        self.inner.lock().await.request(method, params).await
    }
}

impl McpSessionInner {
    async fn request(&mut self, method: &str, params: Value) -> Result<Value, McpRpcError> {
        self.next_id += 1;
        let id = self.next_id;
        self.write_json(&json!({
            "jsonrpc": JSONRPC_VERSION,
            "id": id,
            "method": method,
            "params": params,
        }))
        .await?;
        loop {
            let body = read_frame(&mut self.reader).await?;
            let value: Value = serde_json::from_slice(&body)
                .map_err(|err| McpRpcError::InvalidJson(err.to_string()))?;
            let Some(response_id) = value.get("id").and_then(Value::as_u64) else {
                continue;
            };
            if response_id != id {
                continue;
            }
            match value.get("error") {
                Some(error) if !error.is_null() => {
                    return Err(McpRpcError::Rpc(error.to_string()));
                }
                _ => {}
            }
            match value.get("result") {
                Some(result) => return Ok(result.clone()),
                None => {
                    return Err(McpRpcError::InvalidResponse(
                        "JSON-RPC response has no result".to_string(),
                    ));
                }
            }
        }
    }

    async fn write_json(&mut self, value: &Value) -> Result<(), McpRpcError> {
        let bytes =
            serde_json::to_vec(value).map_err(|err| McpRpcError::InvalidJson(err.to_string()))?;
        self.writer
            .write_all(&encode_frame(&bytes))
            .await
            .map_err(|err| McpRpcError::Write(err.to_string()))?;
        self.writer
            .flush()
            .await
            .map_err(|err| McpRpcError::Write(err.to_string()))?;
        Ok(())
    }
}

/// One tool from MCP `tools/list`, before public-name registration.
pub struct McpToolDraft {
    name: String,
    description: String,
    input_schema: Value,
    task_required: bool,
}

impl McpToolDraft {
    /// Raw MCP tool name sent on `tools/call`.
    ///
    /// # Returns
    ///
    /// The server's own tool name, not an `mcp__` public name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Tool description from `tools/list`, or empty when omitted.
    ///
    /// # Returns
    ///
    /// The listed description string.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// JSON Schema from the listed `inputSchema`.
    ///
    /// # Returns
    ///
    /// The listed schema value.
    #[must_use]
    pub fn input_schema(&self) -> &Value {
        &self.input_schema
    }

    /// Whether the MCP tool advertises execution `taskSupport` `required`.
    ///
    /// # Returns
    ///
    /// `true` only when `execution.taskSupport` is the string `required`.
    #[must_use]
    pub fn task_required(&self) -> bool {
        self.task_required
    }
}

fn parse_tool_draft(tool: &Value) -> Result<McpToolDraft, McpRpcError> {
    let Some(name) = tool.get("name").and_then(Value::as_str) else {
        return Err(McpRpcError::InvalidResponse(
            "tools/list entry is missing name".to_string(),
        ));
    };
    let description = tool
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let input_schema = match tool.get("inputSchema") {
        Some(schema) => schema.clone(),
        None => json!({}),
    };
    let task_required = matches!(
        tool.get("execution")
            .and_then(|execution| execution.get("taskSupport"))
            .and_then(Value::as_str),
        Some("required")
    );
    Ok(McpToolDraft {
        name: name.to_string(),
        description,
        input_schema,
        task_required,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    #[tokio::test]
    async fn initialize_advertises_dsh_mcp_client() {
        let (mut session, fixture) = crate::test_server::spawn_loopback();
        session.initialize().await.expect("initialize");
        let params = fixture.initialize_params();
        assert_eq!(
            params.get("protocolVersion").and_then(Value::as_str),
            Some("2025-03-26")
        );
        let client_info = params.get("clientInfo").expect("clientInfo");
        assert_eq!(
            client_info.get("name").and_then(Value::as_str),
            Some("dsh-mcp-client")
        );
        assert_eq!(
            client_info.get("version").and_then(Value::as_str),
            Some("0.0.1")
        );
        fixture.wait_initialized().await;
    }

    #[tokio::test]
    async fn list_tools_returns_add() {
        let (mut session, _fixture) = crate::test_server::spawn_loopback();
        session.initialize().await.expect("initialize");
        let tools = session.list_tools().await.expect("list_tools");
        assert!(
            tools.iter().any(|tool| tool.name() == "add"),
            "listed tools must include add"
        );
    }

    #[tokio::test]
    async fn call_tool_add_returns_sum_text() {
        let (mut session, _fixture) = crate::test_server::spawn_loopback();
        session.initialize().await.expect("initialize");
        let result = session
            .call_tool("add", json!({"a": 2, "b": 3}))
            .await
            .expect("call_tool");
        let text = result
            .get("content")
            .and_then(Value::as_array)
            .and_then(|blocks| blocks.first())
            .and_then(|block| block.get("text"))
            .and_then(Value::as_str);
        assert_eq!(text, Some("5"));
    }
}
