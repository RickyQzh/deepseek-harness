//! `POST /api/respond` pending table and [`dsh_rpc::RpcReceipt`] JSON.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use dsh_rpc::{ReceiptReject, RpcId, RpcMessage, RpcReceipt, RpcResult};
use tokio::sync::oneshot;

/// Session-scoped pending `server-request` ids waiting for a `client-response`.
#[derive(Clone, Default)]
pub struct RespondTable {
    pending: Arc<Mutex<HashMap<RpcId, oneshot::Sender<RpcResult>>>>,
}

impl RespondTable {
    /// Empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a pending id and return the receiver the waiter (and tests) wait on.
    pub fn insert_pending(&self, id: RpcId) -> oneshot::Receiver<RpcResult> {
        let (tx, rx) = oneshot::channel();
        self.pending.lock().expect("respond table").insert(id, tx);
        rx
    }

    /// Apply one parsed body. Not a `client-response` is `bad-response`; unknown id is `not-pending`.
    #[must_use]
    pub fn apply(&self, message: &RpcMessage) -> RpcReceipt {
        match message {
            RpcMessage::ClientResponse { rpc_id, result } => {
                let sender = self.pending.lock().expect("respond table").remove(rpc_id);
                match sender {
                    Some(tx) => {
                        let _ = tx.send(result.clone());
                        RpcReceipt::accepted()
                    }
                    None => RpcReceipt::rejected(ReceiptReject::NotPending),
                }
            }
            _ => RpcReceipt::rejected(ReceiptReject::BadResponse),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RespondTable;
    use crate::server::spawn_stub_host;
    use dsh_rpc::{RpcId, RpcMessage, RpcResult};
    use serde_json::json;

    #[tokio::test]
    async fn respond_unknown_id_is_not_pending() {
        let host = spawn_stub_host().await;
        let url = format!("http://{}/api/respond", host.local_addr());
        let response = reqwest::Client::new()
            .post(&url)
            .header("host", host.local_addr().to_string())
            .header("content-type", "application/json")
            .body(r#"{"type":"client-response","rpcId":"ghost","result":{"ok":true,"value":{}}}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let v: serde_json::Value = response.json().await.unwrap();
        assert_eq!(v["accepted"], false);
        assert_eq!(v["reason"], "not-pending");
        assert!(v.get("type").is_none());
        host.shutdown().await;
    }

    #[tokio::test]
    async fn respond_inserted_pending_is_accepted() {
        let table = RespondTable::new();
        let id = RpcId::new("pending-1");
        let rx = table.insert_pending(id.clone());
        let receipt = table.apply(&RpcMessage::client_response(
            id,
            RpcResult::ok(json!({"done": true})),
        ));
        assert!(receipt.is_accepted());
        let got = rx.await.expect("oneshot");
        assert_eq!(got.as_ok(), Some(&json!({"done": true})));
    }
}
