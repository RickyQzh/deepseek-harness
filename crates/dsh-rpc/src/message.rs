//! Four-quadrant [`RpcMessage`] union, [`RpcResult`], and carrier [`RpcReceipt`].

use serde::de::Error as DeError;
use serde::ser::{SerializeStruct, Serializer};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::error::RpcError;

/// Message correlation id: the initiator mints it; a response echoes the matching request.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct RpcId(String);

impl RpcId {
    /// Wrap a raw id string. Callers mint ids; this constructor does not validate uniqueness.
    #[must_use]
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    /// Borrow the raw id string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Authoritative four-quadrant wire union. Discriminant field is `type`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RpcMessage {
    /// Call initiated by the client.
    #[serde(rename = "client-request", rename_all = "camelCase")]
    ClientRequest {
        /// Correlation id minted by the client.
        rpc_id: RpcId,
        /// Dotted or slash method name.
        method: String,
        /// Method payload JSON.
        payload: Value,
    },
    /// Response to a [`RpcMessage::ClientRequest`]; `rpcId` is echoed.
    #[serde(rename = "server-response", rename_all = "camelCase")]
    ServerResponse {
        /// Correlation id echoed from the matching request.
        rpc_id: RpcId,
        /// Business success or error.
        result: RpcResult,
    },
    /// Message initiated by the server (downlink frame).
    #[serde(rename = "server-request", rename_all = "camelCase")]
    ServerRequest {
        /// Correlation id minted by the host.
        rpc_id: RpcId,
        /// Method name; whether a response is expected is fixed per method.
        method: String,
        /// Method payload JSON.
        payload: Value,
    },
    /// Response to a [`RpcMessage::ServerRequest`]; `rpcId` is echoed, never minted anew.
    #[serde(rename = "client-response", rename_all = "camelCase")]
    ClientResponse {
        /// Correlation id echoed from the matching server request.
        rpc_id: RpcId,
        /// Business success or error.
        result: RpcResult,
    },
}

impl RpcMessage {
    /// Client-initiated call (`type`: `client-request`).
    #[must_use]
    pub fn client_request(rpc_id: RpcId, method: impl Into<String>, payload: Value) -> Self {
        Self::ClientRequest {
            rpc_id,
            method: method.into(),
            payload,
        }
    }

    /// Response to a client request (`type`: `server-response`).
    #[must_use]
    pub fn server_response(rpc_id: RpcId, result: RpcResult) -> Self {
        Self::ServerResponse { rpc_id, result }
    }

    /// Server-initiated message (`type`: `server-request`).
    #[must_use]
    pub fn server_request(rpc_id: RpcId, method: impl Into<String>, payload: Value) -> Self {
        Self::ServerRequest {
            rpc_id,
            method: method.into(),
            payload,
        }
    }

    /// Response to a server request (`type`: `client-response`).
    #[must_use]
    pub fn client_response(rpc_id: RpcId, result: RpcResult) -> Self {
        Self::ClientResponse { rpc_id, result }
    }

    /// Correlation id: minted on a request, echoed on the matching response.
    #[must_use]
    pub fn rpc_id(&self) -> &RpcId {
        match self {
            Self::ClientRequest { rpc_id, .. }
            | Self::ServerResponse { rpc_id, .. }
            | Self::ServerRequest { rpc_id, .. }
            | Self::ClientResponse { rpc_id, .. } => rpc_id,
        }
    }

    /// Method name on a request; `None` on a response.
    #[must_use]
    pub fn method(&self) -> Option<&str> {
        match self {
            Self::ClientRequest { method, .. } | Self::ServerRequest { method, .. } => Some(method),
            Self::ServerResponse { .. } | Self::ClientResponse { .. } => None,
        }
    }

    /// Request payload JSON; `None` on a response.
    #[must_use]
    pub fn payload(&self) -> Option<&Value> {
        match self {
            Self::ClientRequest { payload, .. } | Self::ServerRequest { payload, .. } => {
                Some(payload)
            }
            Self::ServerResponse { .. } | Self::ClientResponse { .. } => None,
        }
    }

    /// Response result; `None` on a request.
    #[must_use]
    pub fn result(&self) -> Option<&RpcResult> {
        match self {
            Self::ServerResponse { result, .. } | Self::ClientResponse { result, .. } => {
                Some(result)
            }
            Self::ClientRequest { .. } | Self::ServerRequest { .. } => None,
        }
    }
}

/// Business success or failure in a unary response. Methods never throw business errors.
#[derive(Clone, Debug, PartialEq)]
pub enum RpcResult {
    /// `{ "ok": true, "value": ... }`.
    Ok {
        /// Success payload JSON.
        value: Value,
    },
    /// `{ "ok": false, "error": { "code", "message", "details" } }`.
    Err {
        /// Closed-code wire error.
        error: RpcError,
    },
}

impl RpcResult {
    /// Success branch.
    #[must_use]
    pub fn ok(value: Value) -> Self {
        Self::Ok { value }
    }

    /// Error branch.
    #[must_use]
    pub fn err(error: RpcError) -> Self {
        Self::Err { error }
    }

    /// Success payload when `ok` is true.
    #[must_use]
    pub fn as_ok(&self) -> Option<&Value> {
        match self {
            Self::Ok { value } => Some(value),
            Self::Err { .. } => None,
        }
    }

    /// Closed-code error when `ok` is false.
    #[must_use]
    pub fn as_err(&self) -> Option<&RpcError> {
        match self {
            Self::Err { error } => Some(error),
            Self::Ok { .. } => None,
        }
    }
}

impl Serialize for RpcResult {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Ok { value } => {
                let mut state = serializer.serialize_struct("RpcResult", 2)?;
                state.serialize_field("ok", &true)?;
                state.serialize_field("value", value)?;
                state.end()
            }
            Self::Err { error } => {
                let mut state = serializer.serialize_struct("RpcResult", 2)?;
                state.serialize_field("ok", &false)?;
                state.serialize_field("error", error)?;
                state.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for RpcResult {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let obj = value
            .as_object()
            .ok_or_else(|| D::Error::custom("RpcResult must be a JSON object"))?;
        let ok = match obj.get("ok") {
            Some(Value::Bool(ok)) => *ok,
            _ => return Err(D::Error::custom("RpcResult.ok must be a boolean")),
        };
        match (ok, obj.get("value"), obj.get("error")) {
            (true, Some(value), None) => Ok(Self::Ok {
                value: value.clone(),
            }),
            (false, None, Some(error)) => {
                let error = serde_json::from_value(error.clone()).map_err(D::Error::custom)?;
                Ok(Self::Err { error })
            }
            _ => Err(D::Error::custom(
                "RpcResult must be {\"ok\":true,\"value\":...} or {\"ok\":false,\"error\":...}",
            )),
        }
    }
}

/// Carrier receipt for `POST /api/respond`. Not an [`RpcMessage`]; there is no `type` field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RpcReceipt {
    /// `{ "accepted": true }`.
    Accepted,
    /// `{ "accepted": false, "reason": ... }`.
    Rejected {
        /// Why the client-response was not applied.
        reason: ReceiptReject,
    },
}

impl RpcReceipt {
    /// Accepted receipt.
    #[must_use]
    pub fn accepted() -> Self {
        Self::Accepted
    }

    /// Rejected receipt.
    #[must_use]
    pub fn rejected(reason: ReceiptReject) -> Self {
        Self::Rejected { reason }
    }

    /// Whether the carrier accepted the client-response.
    #[must_use]
    pub fn is_accepted(&self) -> bool {
        matches!(self, Self::Accepted)
    }

    /// Rejection reason when the client-response was not applied.
    #[must_use]
    pub fn reject_reason(&self) -> Option<ReceiptReject> {
        match self {
            Self::Rejected { reason } => Some(*reason),
            Self::Accepted => None,
        }
    }
}

impl Serialize for RpcReceipt {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Accepted => {
                let mut state = serializer.serialize_struct("RpcReceipt", 1)?;
                state.serialize_field("accepted", &true)?;
                state.end()
            }
            Self::Rejected { reason } => {
                let mut state = serializer.serialize_struct("RpcReceipt", 2)?;
                state.serialize_field("accepted", &false)?;
                state.serialize_field("reason", reason)?;
                state.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for RpcReceipt {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let obj = value
            .as_object()
            .ok_or_else(|| D::Error::custom("RpcReceipt must be a JSON object"))?;
        let accepted = match obj.get("accepted") {
            Some(Value::Bool(accepted)) => *accepted,
            _ => return Err(D::Error::custom("RpcReceipt.accepted must be a boolean")),
        };
        match (accepted, obj.get("reason")) {
            (true, None) => Ok(Self::Accepted),
            (false, Some(reason)) => {
                let reason = serde_json::from_value(reason.clone()).map_err(D::Error::custom)?;
                Ok(Self::Rejected { reason })
            }
            _ => Err(D::Error::custom(
                "RpcReceipt must be {\"accepted\":true} or {\"accepted\":false,\"reason\":...}",
            )),
        }
    }
}

/// Rejection reason when a client-response is not applied.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReceiptReject {
    /// Late or duplicate response; no matching pending server-request.
    NotPending,
    /// Body was not a valid client-response for the pending id.
    BadResponse,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{RpcError, RpcErrorCode};

    #[test]
    fn client_request_round_trips_camel_case() {
        let raw = r#"{"type":"client-request","rpcId":"r1","method":"host.describe","payload":{}}"#;
        let msg: RpcMessage = serde_json::from_str(raw).unwrap();
        match &msg {
            RpcMessage::ClientRequest {
                rpc_id,
                method,
                payload,
            } => {
                assert_eq!(rpc_id.as_str(), "r1");
                assert_eq!(method, "host.describe");
                assert_eq!(payload, &serde_json::json!({}));
            }
            other => panic!("{other:?}"),
        }
        let back: serde_json::Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(back["type"], "client-request");
        assert_eq!(back["rpcId"], "r1");
    }

    #[test]
    fn server_response_ok_and_error_branches() {
        let ok: RpcMessage = serde_json::from_str(
            r#"{"type":"server-response","rpcId":"r1","result":{"ok":true,"value":{"version":"0.0.1"}}}"#,
        )
        .unwrap();
        match ok {
            RpcMessage::ServerResponse {
                result: RpcResult::Ok { value },
                ..
            } => {
                assert_eq!(value["version"], "0.0.1");
            }
            other => panic!("{other:?}"),
        }
        let err: RpcMessage = serde_json::from_str(
            r#"{"type":"server-response","rpcId":"r1","result":{"ok":false,"error":{"code":"session-not-found","message":"gone","details":{"sessionId":"s1"}}}}"#,
        )
        .unwrap();
        match err {
            RpcMessage::ServerResponse {
                result: RpcResult::Err { error },
                ..
            } => {
                assert_eq!(error.code(), RpcErrorCode::SessionNotFound);
                assert_eq!(error.message(), "gone");
                assert_eq!(error.details()["sessionId"], "s1");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn receipt_is_not_an_rpc_message() {
        let accepted: RpcReceipt = serde_json::from_str(r#"{"accepted":true}"#).unwrap();
        assert!(matches!(accepted, RpcReceipt::Accepted));
        let rejected: RpcReceipt =
            serde_json::from_str(r#"{"accepted":false,"reason":"not-pending"}"#).unwrap();
        match rejected {
            RpcReceipt::Rejected { reason } => assert!(matches!(reason, ReceiptReject::NotPending)),
            RpcReceipt::Accepted => panic!("accepted"),
        }
        assert!(serde_json::from_str::<RpcMessage>(r#"{"accepted":true}"#).is_err());
    }

    #[test]
    fn unknown_error_code_refuses_decode() {
        let err = serde_json::from_str::<RpcError>(
            r#"{"code":"not-a-real-code","message":"x","details":{}}"#,
        );
        assert!(err.is_err(), "{err:?}");
    }
}
