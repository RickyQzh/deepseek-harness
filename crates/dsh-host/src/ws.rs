//! Mux and host GUI WebSocket downlinks.

use axum::extract::ws::{Message, WebSocket};
use dsh_rpc::{RpcId, RpcMessage};
use futures::{SinkExt, StreamExt};
use tokio::sync::broadcast;

/// Frames buffered per downlink before a slow subscriber is marked lagged.
const DOWNLINK_CAPACITY: usize = 64;

/// Cloneable mux/host downlink publisher. Each WebSocket text frame is one [`RpcMessage::ServerRequest`].
#[derive(Clone)]
pub struct DownlinkHub {
    mux: broadcast::Sender<RpcMessage>,
    host: broadcast::Sender<RpcMessage>,
}

impl DownlinkHub {
    /// Empty hub. `publish_*` with no subscribers drops the frame.
    #[must_use]
    pub fn new() -> Self {
        let (mux, _) = broadcast::channel(DOWNLINK_CAPACITY);
        let (host, _) = broadcast::channel(DOWNLINK_CAPACITY);
        Self { mux, host }
    }

    /// Subscribe to `/api/events.mux` frames. A lagged receiver skips missed frames.
    #[must_use]
    pub fn subscribe_mux(&self) -> broadcast::Receiver<RpcMessage> {
        self.mux.subscribe()
    }

    /// Subscribe to `/api/events.host` frames. A lagged receiver skips missed frames.
    #[must_use]
    pub fn subscribe_host(&self) -> broadcast::Receiver<RpcMessage> {
        self.host.subscribe()
    }

    /// Wrap `method` / `rpc_id` / `payload` as a mux `server-request` and broadcast it.
    pub fn publish_mux(&self, method: &str, rpc_id: RpcId, payload: serde_json::Value) {
        let _ = self
            .mux
            .send(RpcMessage::server_request(rpc_id, method, payload));
    }

    /// Wrap `method` / `rpc_id` / `payload` as a host `server-request` and broadcast it.
    pub fn publish_host(&self, method: &str, rpc_id: RpcId, payload: serde_json::Value) {
        let _ = self
            .host
            .send(RpcMessage::server_request(rpc_id, method, payload));
    }
}

impl Default for DownlinkHub {
    fn default() -> Self {
        Self::new()
    }
}

/// Forward hub frames as text and close on client text/binary application messages.
pub(crate) async fn run_downlink(socket: WebSocket, mut rx: broadcast::Receiver<RpcMessage>) {
    let (mut sender, mut receiver) = socket.split();
    loop {
        tokio::select! {
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Text(_))) | Some(Ok(Message::Binary(_))) => break,
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                }
            }
            outbound = rx.recv() => {
                match outbound {
                    Ok(msg) => {
                        let close_after = msg.method() == Some("stream/error");
                        let text = match serde_json::to_string(&msg) {
                            Ok(text) => text,
                            Err(_) => continue,
                        };
                        if sender.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                        if close_after {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::{SinkExt, StreamExt};

    use crate::server::spawn_stub_host;

    #[tokio::test]
    async fn mux_upgrade_forwards_session_subscribed() {
        let host = spawn_stub_host().await;
        let addr = host.local_addr();
        let url = format!("ws://{addr}/api/events.mux");
        let (mut ws, _resp) = tokio_tungstenite::connect_async(&url)
            .await
            .expect("upgrade");
        host.hub().publish_mux(
            "session/subscribed",
            dsh_rpc::RpcId::new("p1"),
            serde_json::json!({"lastSeq": -1}),
        );
        let msg = tokio::time::timeout(std::time::Duration::from_secs(2), ws.next())
            .await
            .expect("timeout")
            .expect("closed")
            .expect("ws");
        let text = msg.into_text().expect("text");
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["type"], "server-request");
        assert_eq!(v["rpcId"], "p1");
        assert_eq!(v["method"], "session/subscribed");
        assert_eq!(v["payload"]["lastSeq"], -1);
        host.shutdown().await;
    }

    #[tokio::test]
    async fn mux_client_text_closes_socket() {
        let host = spawn_stub_host().await;
        let url = format!("ws://{}/api/events.mux", host.local_addr());
        let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        ws.send(tokio_tungstenite::tungstenite::Message::Text(
            "{\"type\":\"client-request\"}".into(),
        ))
        .await
        .unwrap();
        let next = tokio::time::timeout(std::time::Duration::from_secs(2), ws.next())
            .await
            .expect("timeout");
        assert!(
            next.is_none() || next.unwrap().is_err(),
            "socket must close"
        );
        host.shutdown().await;
    }

    #[tokio::test]
    async fn get_events_mux_still_426() {
        let host = spawn_stub_host().await;
        let url = format!("http://{}/api/events.mux", host.local_addr());
        let response = reqwest::Client::new()
            .get(&url)
            .header("host", host.local_addr().to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 426);
        host.shutdown().await;
    }
}
