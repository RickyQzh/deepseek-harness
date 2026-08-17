//! Unary POST `/api/<dotted>` JSON carrier.

use std::future::Future;
use std::pin::Pin;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use dsh_rpc::{RpcError, RpcId, RpcMessage, RpcResult};

use crate::trust::{is_privileged_method, privileged_requires_loopback};

/// Dotted GUI method handler. Unknown methods are HTTP 404 (carrier), not [`RpcResult::err`].
pub trait RpcHandler: Send + Sync {
    /// Whether this handler owns `method`. Dispatch answers HTTP 404 when this is false.
    fn accepts_dotted(&self, method: &str) -> bool;

    /// Run one dotted method. Call only when [`RpcHandler::accepts_dotted`] is true.
    ///
    /// `rpc_id` is echoed on the HTTP 200 `server-response`. `payload` is the request JSON object.
    ///
    /// Returns a business [`RpcResult`]. The carrier wraps it as HTTP 200 + `server-response`.
    fn handle_dotted(
        &self,
        method: &str,
        rpc_id: &RpcId,
        payload: serde_json::Value,
    ) -> impl Future<Output = RpcResult> + Send;

    /// Run one slash remote (`commands/list`, `commands/execute`). `None` is HTTP 404.
    fn handle_slash(
        &self,
        method: &str,
        rpc_id: &RpcId,
        payload: serde_json::Value,
    ) -> impl Future<Output = Option<RpcResult>> + Send {
        let _ = (method, rpc_id, payload);
        async { None }
    }
}

pub(crate) trait ErasedRpcHandler: Send + Sync {
    fn accepts_dotted(&self, method: &str) -> bool;
    fn handle_dotted<'a>(
        &'a self,
        method: &'a str,
        rpc_id: &'a RpcId,
        payload: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = RpcResult> + Send + 'a>>;
    fn handle_slash<'a>(
        &'a self,
        method: &'a str,
        rpc_id: &'a RpcId,
        payload: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = Option<RpcResult>> + Send + 'a>>;
}

impl<T: RpcHandler> ErasedRpcHandler for T {
    fn accepts_dotted(&self, method: &str) -> bool {
        RpcHandler::accepts_dotted(self, method)
    }

    fn handle_dotted<'a>(
        &'a self,
        method: &'a str,
        rpc_id: &'a RpcId,
        payload: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = RpcResult> + Send + 'a>> {
        Box::pin(RpcHandler::handle_dotted(self, method, rpc_id, payload))
    }

    fn handle_slash<'a>(
        &'a self,
        method: &'a str,
        rpc_id: &'a RpcId,
        payload: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = Option<RpcResult>> + Send + 'a>> {
        Box::pin(RpcHandler::handle_slash(self, method, rpc_id, payload))
    }
}

/// Answers `host.describe` only. Other dotted methods are a carrier HTTP 404.
pub struct StubHandler;

impl RpcHandler for StubHandler {
    fn accepts_dotted(&self, method: &str) -> bool {
        method == "host.describe"
    }

    fn handle_dotted(
        &self,
        method: &str,
        _rpc_id: &RpcId,
        _payload: serde_json::Value,
    ) -> impl Future<Output = RpcResult> + Send {
        let accepted = method == "host.describe";
        async move {
            if accepted {
                RpcResult::ok(host_describe_value())
            } else {
                RpcResult::err(RpcError::internal("uninstalled dotted method"))
            }
        }
    }
}

pub(crate) fn host_describe_value() -> serde_json::Value {
    describe_host(0, None, None)
}

pub(crate) fn describe_host(
    attached_sessions: usize,
    provider: Option<&str>,
    model: Option<&str>,
) -> serde_json::Value {
    let cwd = match std::env::var("DSH_CWD") {
        Ok(cwd) => cwd,
        Err(_) => match std::env::current_dir() {
            Ok(path) => path.to_string_lossy().into_owned(),
            Err(_) => String::new(),
        },
    };
    let mut value = serde_json::json!({
        "version": "0.0.1",
        "cwd": cwd,
        "attachedSessions": attached_sessions,
        "canOpenPath": false,
    });
    if let Some(provider) = provider {
        value["provider"] = serde_json::json!(provider);
    }
    if let Some(model) = model {
        value["model"] = serde_json::json!(model);
    }
    value
}

pub(crate) fn forbidden_response() -> Response {
    (StatusCode::FORBIDDEN, "forbidden").into_response()
}

pub(crate) async fn dispatch_dotted(
    handler: &dyn ErasedRpcHandler,
    host_header: Option<&str>,
    path_suffix: &str,
    content_type: Option<&str>,
    body: &[u8],
) -> Response {
    if !is_json_content_type(content_type) {
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "content type must be application/json",
        )
            .into_response();
    }
    let message: RpcMessage = match serde_json::from_slice(body) {
        Ok(message) => message,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    if !matches!(message, RpcMessage::ClientRequest { .. }) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let rpc_id = message.rpc_id().clone();
    let Some(method) = message.method() else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if method != path_suffix {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if is_privileged_method(method) && !privileged_requires_loopback(host_header) {
        return forbidden_response();
    }
    let Some(payload) = message.payload() else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if path_suffix.contains('/') {
        match handler.handle_slash(method, &rpc_id, payload.clone()).await {
            Some(result) => {
                return (
                    StatusCode::OK,
                    axum::Json(RpcMessage::server_response(rpc_id, result)),
                )
                    .into_response();
            }
            None => {
                return (StatusCode::NOT_FOUND, "not found").into_response();
            }
        }
    }
    if !handler.accepts_dotted(method) {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    let result = handler
        .handle_dotted(method, &rpc_id, payload.clone())
        .await;
    (
        StatusCode::OK,
        axum::Json(RpcMessage::server_response(rpc_id, result)),
    )
        .into_response()
}

pub(crate) fn is_json_content_type(content_type: Option<&str>) -> bool {
    let Some(raw) = content_type else {
        return false;
    };
    let media = match raw.split_once(';') {
        Some((media, _)) => media,
        None => raw,
    };
    media.trim().eq_ignore_ascii_case("application/json")
}
