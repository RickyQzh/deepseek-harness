//! GUI four-quadrant RPC envelopes (`RpcMessage`, `RpcResult`, `RpcReceipt`).
//!
//! This crate is the GUI wire, not SDK JSON-RPC. Details stay [`serde_json::Value`].

mod error;
mod message;

pub use error::{RpcError, RpcErrorCode};
pub use message::{ReceiptReject, RpcId, RpcMessage, RpcReceipt, RpcResult};

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn client_request_accessors() {
        let msg: RpcMessage = serde_json::from_str(
            r#"{"type":"client-request","rpcId":"r1","method":"host.describe","payload":{}}"#,
        )
        .unwrap();
        assert_eq!(msg.rpc_id().as_str(), "r1");
        assert_eq!(msg.method(), Some("host.describe"));
        assert_eq!(msg.payload(), Some(&json!({})));
        assert!(msg.result().is_none());
    }

    #[test]
    fn server_response_ok_accessors() {
        let msg: RpcMessage = serde_json::from_str(
            r#"{"type":"server-response","rpcId":"r1","result":{"ok":true,"value":{"version":"0.0.1"}}}"#,
        )
        .unwrap();
        assert_eq!(msg.rpc_id().as_str(), "r1");
        assert!(msg.method().is_none());
        assert!(msg.payload().is_none());
        let result = msg.result().expect("response result");
        assert_eq!(result.as_ok(), Some(&json!({"version": "0.0.1"})));
        assert!(result.as_err().is_none());
    }

    #[test]
    fn server_response_err_accessors() {
        let msg: RpcMessage = serde_json::from_str(
            r#"{"type":"server-response","rpcId":"r1","result":{"ok":false,"error":{"code":"session-not-found","message":"gone","details":{"sessionId":"s1"}}}}"#,
        )
        .unwrap();
        assert_eq!(msg.rpc_id().as_str(), "r1");
        assert!(msg.method().is_none());
        assert!(msg.payload().is_none());
        let error = msg
            .result()
            .and_then(RpcResult::as_err)
            .expect("error result");
        assert_eq!(error.code(), RpcErrorCode::SessionNotFound);
        assert_eq!(error.message(), "gone");
        assert_eq!(error.details()["sessionId"], "s1");
        assert!(msg.result().and_then(RpcResult::as_ok).is_none());
    }

    #[test]
    fn rejected_receipt_accessors() {
        let receipt: RpcReceipt =
            serde_json::from_str(r#"{"accepted":false,"reason":"not-pending"}"#).unwrap();
        assert!(!receipt.is_accepted());
        assert_eq!(receipt.reject_reason(), Some(ReceiptReject::NotPending));
    }
}
