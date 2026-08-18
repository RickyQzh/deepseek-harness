//! JSON-RPC error objects with TypeScript ACP message strings.

use serde::Serialize;
use serde_json::{Value, json};

/// JSON-RPC parse error.
pub const ERR_PARSE: i64 = -32700;
/// JSON-RPC invalid request.
pub const ERR_INVALID_REQUEST: i64 = -32600;
/// JSON-RPC method not found.
pub const ERR_METHOD_NOT_FOUND: i64 = -32601;
/// JSON-RPC invalid params.
pub const ERR_INVALID_PARAMS: i64 = -32602;
/// JSON-RPC internal error.
pub const ERR_INTERNAL: i64 = -32603;

/// JSON-RPC `error` object. `data` is omitted on the wire when `None`.
#[derive(Clone, Debug, PartialEq, Serialize, thiserror::Error)]
#[error("{message}")]
pub struct AcpError {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

impl AcpError {
    /// Reconstruct an error from a response frame.
    pub(crate) fn from_parts(code: i64, message: String, data: Option<Value>) -> Self {
        Self {
            code,
            message,
            data,
        }
    }

    /// JSON-RPC `error.code`.
    #[must_use]
    pub fn code(&self) -> i64 {
        self.code
    }

    /// JSON-RPC `error.message`.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// JSON-RPC `error.data` when present.
    #[must_use]
    pub fn data(&self) -> Option<&Value> {
        self.data.as_ref()
    }
}

/// `-32601` with TypeScript `"Method not found": {method}` and `data.method`.
#[must_use]
pub fn method_not_found(method: &str) -> AcpError {
    AcpError::from_parts(
        ERR_METHOD_NOT_FOUND,
        format!("\"Method not found\": {method}"),
        Some(json!({ "method": method })),
    )
}

/// `-32602` with TypeScript `Invalid params: {detail}` and no `data`.
#[must_use]
pub fn invalid_params(detail: &str) -> AcpError {
    AcpError::from_parts(
        ERR_INVALID_PARAMS,
        format!("Invalid params: {detail}"),
        None,
    )
}

/// `-32603` with TypeScript `Internal error: {detail}` and no `data`.
#[must_use]
pub fn internal_error(detail: &str) -> AcpError {
    AcpError::from_parts(ERR_INTERNAL, format!("Internal error: {detail}"), None)
}
