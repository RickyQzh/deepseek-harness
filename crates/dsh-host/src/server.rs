//! Loopback HTTP listener, static SPA/`/plugins` routes, unary `/api` carrier, and WebSocket downlinks.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::ws::{WebSocketUpgrade, rejection::WebSocketUpgradeRejection};
use axum::extract::{Path as PathParam, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use tokio::net::TcpListener;

use crate::boot::WebBootGraph;
#[cfg(test)]
use crate::dispatch::StubHandler;
use crate::dispatch::{
    ErasedRpcHandler, RpcHandler, dispatch_dotted, forbidden_response, is_json_content_type,
};
use crate::plugins::{
    HostError, scan_client_graph_and_dirs, serve_plugin_js, serve_plugin_source_map,
};
use crate::respond::RespondTable;
use crate::static_files::{StaticResponse, serve_spa};
use crate::trust::{assert_trusted_authority, is_trusted_api_request};
use crate::ws::{DownlinkHub, run_downlink};

/// Loopback bind target. `listen_host` must be `127.0.0.1`; port `0` is OS-assigned.
#[derive(Clone, Debug)]
pub struct HostBind {
    listen_host: String,
    port: u16,
}

impl HostBind {
    /// Reject any `listen_host` other than `127.0.0.1`.
    pub fn new(listen_host: impl Into<String>, port: u16) -> Result<Self, HostError> {
        let listen_host = listen_host.into();
        if listen_host != "127.0.0.1" {
            return Err(HostError::invalid_listen_host(listen_host));
        }
        Ok(Self { listen_host, port })
    }

    /// Locked loopback address (`127.0.0.1`).
    #[must_use]
    pub fn listen_host(&self) -> &str {
        &self.listen_host
    }

    /// TCP port. `0` asks the OS to assign one.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }
}

/// Dist directory and `packages/client` tree used by SPA and `/plugins`.
#[derive(Clone, Debug)]
pub struct HostPaths {
    dist: PathBuf,
    client_packages: PathBuf,
}

impl HostPaths {
    /// `dist` is the SPA root; `client_packages` is scanned for `dsh.client` bundles.
    #[must_use]
    pub fn new(dist: PathBuf, client_packages: PathBuf) -> Self {
        Self {
            dist,
            client_packages,
        }
    }

    /// SPA dist directory (`index.html` and hashed assets).
    #[must_use]
    pub fn dist(&self) -> &Path {
        &self.dist
    }

    /// `packages/client` directory passed to [`crate::scan_client_packages`].
    #[must_use]
    pub fn client_packages(&self) -> &Path {
        &self.client_packages
    }
}

/// Listener configuration, boot graph, trust list, downlink hub, and dotted RPC handler.
#[derive(Clone)]
pub struct HostState {
    bind: HostBind,
    paths: HostPaths,
    graph: WebBootGraph,
    trusted_hosts: Vec<String>,
    plugin_dirs: HashMap<String, String>,
    handler: Arc<dyn ErasedRpcHandler>,
    hub: DownlinkHub,
    respond: RespondTable,
}

impl HostState {
    /// Scan `paths.client_packages`, validate `trusted_hosts`, and store `handler`.
    ///
    /// Each `trusted_hosts` entry must pass [`assert_trusted_authority`]. An empty client-package directory yields an empty graph.
    pub fn new(
        bind: HostBind,
        paths: HostPaths,
        trusted_hosts: Vec<String>,
        handler: impl RpcHandler + 'static,
    ) -> Result<Self, HostError> {
        for entry in &trusted_hosts {
            assert_trusted_authority(entry)?;
        }
        let (graph, plugin_dirs) = scan_client_graph_and_dirs(paths.client_packages())?;
        Ok(Self {
            bind,
            paths,
            graph,
            trusted_hosts,
            plugin_dirs,
            handler: Arc::new(handler),
            hub: DownlinkHub::new(),
            respond: RespondTable::new(),
        })
    }

    /// Bind host and port used by [`serve`].
    #[must_use]
    pub fn bind(&self) -> &HostBind {
        &self.bind
    }

    /// Mux and host downlink publisher for this listener.
    #[must_use]
    pub fn hub(&self) -> &DownlinkHub {
        &self.hub
    }

    /// Pending `client-response` table for `POST /api/respond`.
    #[must_use]
    pub fn respond_table(&self) -> &RespondTable {
        &self.respond
    }
}

/// Bound loopback listener. Dropping the value without [`ListeningHost::shutdown`] aborts the accept loop.
#[must_use]
pub struct ListeningHost {
    local_addr: SocketAddr,
    hub: DownlinkHub,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    join: Option<tokio::task::JoinHandle<Result<(), std::io::Error>>>,
}

impl ListeningHost {
    /// Address the OS bound, including the assigned port when [`HostBind::port`] was `0`.
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Downlink hub bound with this listener. `publish_*` wraps [`dsh_rpc::RpcMessage::server_request`].
    #[must_use]
    pub fn hub(&self) -> &DownlinkHub {
        &self.hub
    }

    /// Stop accepting and wait for in-flight requests to finish.
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(join) = self.join.take() {
            let _ = join.await;
        }
    }
}

/// Bind `127.0.0.1` and serve the GUI HTTP table until [`ListeningHost::shutdown`].
pub async fn serve(state: HostState) -> Result<ListeningHost, HostError> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, state.bind.port()));
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|source| HostError::bind(addr.to_string(), source))?;
    let local_addr = listener
        .local_addr()
        .map_err(|source| HostError::bind(addr.to_string(), source))?;
    let hub = state.hub.clone();
    let app = router(state);
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let join = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await
    });
    Ok(ListeningHost {
        local_addr,
        hub,
        shutdown: Some(shutdown_tx),
        join: Some(join),
    })
}

fn router(state: HostState) -> Router {
    Router::new()
        .route("/api/respond", post(post_respond))
        .route("/api/events.mux", get(events_mux).head(upgrade_required))
        .route("/api/events.host", get(events_host).head(upgrade_required))
        .route("/api/{*method}", post(post_dotted))
        .fallback(fallback)
        .with_state(state)
}

fn header_text(headers: &HeaderMap, name: axum::http::HeaderName) -> Option<&str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn api_trusted(state: &HostState, headers: &HeaderMap) -> bool {
    is_trusted_api_request(
        header_text(headers, header::HOST),
        header_text(headers, header::ORIGIN),
        header_text(
            headers,
            axum::http::HeaderName::from_static("sec-fetch-site"),
        ),
        &state.trusted_hosts,
    )
}

async fn post_respond(State(state): State<HostState>, headers: HeaderMap, body: Bytes) -> Response {
    if !api_trusted(&state, &headers) {
        return forbidden_response();
    }
    if !is_json_content_type(header_text(&headers, header::CONTENT_TYPE)) {
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "content type must be application/json",
        )
            .into_response();
    }
    let receipt = match serde_json::from_slice::<dsh_rpc::RpcMessage>(&body) {
        Ok(message) => state.respond.apply(&message),
        Err(_) => dsh_rpc::RpcReceipt::rejected(dsh_rpc::ReceiptReject::BadResponse),
    };
    (StatusCode::OK, axum::Json(receipt)).into_response()
}

async fn upgrade_required(State(state): State<HostState>, headers: HeaderMap) -> Response {
    if !api_trusted(&state, &headers) {
        return forbidden_response();
    }
    upgrade_required_response()
}

fn upgrade_required_response() -> Response {
    let mut response = (StatusCode::UPGRADE_REQUIRED, "upgrade required").into_response();
    response
        .headers_mut()
        .insert(header::UPGRADE, HeaderValue::from_static("websocket"));
    response
        .headers_mut()
        .insert(header::CONNECTION, HeaderValue::from_static("Upgrade"));
    response
}

async fn events_mux(
    State(state): State<HostState>,
    headers: HeaderMap,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    accept_downlink(api_trusted(&state, &headers), upgrade, || {
        state.hub.subscribe_mux()
    })
}

async fn events_host(
    State(state): State<HostState>,
    headers: HeaderMap,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    accept_downlink(api_trusted(&state, &headers), upgrade, || {
        state.hub.subscribe_host()
    })
}

fn accept_downlink<F>(
    trusted: bool,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
    subscribe: F,
) -> Response
where
    F: FnOnce() -> tokio::sync::broadcast::Receiver<dsh_rpc::RpcMessage>,
{
    if !trusted {
        return forbidden_response();
    }
    match upgrade {
        Ok(ws) => {
            let rx = subscribe();
            ws.on_upgrade(move |socket| run_downlink(socket, rx))
        }
        Err(_) => upgrade_required_response(),
    }
}

async fn post_dotted(
    State(state): State<HostState>,
    PathParam(method): PathParam<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !api_trusted(&state, &headers) {
        return forbidden_response();
    }
    dispatch_dotted(
        state.handler.as_ref(),
        header_text(&headers, header::HOST),
        &method,
        header_text(&headers, header::CONTENT_TYPE),
        &body,
    )
    .await
}

async fn fallback(
    State(state): State<HostState>,
    req: axum::http::Request<axum::body::Body>,
) -> Response {
    match *req.method() {
        Method::GET | Method::HEAD => {
            static_response(&state, req.uri(), *req.method() == Method::HEAD)
        }
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

fn static_response(state: &HostState, uri: &Uri, head: bool) -> Response {
    let path = uri.path();
    if let Some((id, source_map)) = plugin_asset(path) {
        let folder = plugin_folder(state, id);
        let response = match folder {
            Some(folder) => {
                if source_map {
                    serve_plugin_source_map(state.paths.client_packages(), &folder)
                } else {
                    serve_plugin_js(state.paths.client_packages(), &folder)
                }
            }
            None => StaticResponse::not_found(),
        };
        return response_from_static(response, head);
    }
    response_from_static(serve_spa(state.paths.dist(), path, &state.graph), head)
}

fn plugin_asset(path: &str) -> Option<(&str, bool)> {
    let rest = path.strip_prefix("/plugins/")?;
    if let Some(id) = rest.strip_suffix("/client.js.map") {
        if id.is_empty() {
            return None;
        }
        return Some((id, true));
    }
    if let Some(id) = rest.strip_suffix("/client.js") {
        if id.is_empty() {
            return None;
        }
        return Some((id, false));
    }
    None
}

fn plugin_folder(state: &HostState, id: &str) -> Option<String> {
    if let Some(folder) = state.plugin_dirs.get(id) {
        return Some(folder.clone());
    }
    if id.contains('/') {
        None
    } else {
        Some(id.to_string())
    }
}

fn response_from_static(static_response: StaticResponse, head: bool) -> Response {
    let mut builder = Response::builder()
        .status(static_response.status())
        .header(header::CONTENT_TYPE, static_response.content_type());
    if let Some(cache) = static_response.cache_control() {
        builder = builder.header(header::CACHE_CONTROL, cache);
    }
    let body = if head {
        axum::body::Body::empty()
    } else {
        axum::body::Body::from(static_response.body().to_vec())
    };
    builder.body(body).expect("static headers are valid")
}

#[cfg(test)]
pub(crate) async fn spawn_stub_host() -> ListeningHost {
    spawn_stub_host_with_trusted(&[]).await
}

#[cfg(test)]
pub(crate) async fn spawn_stub_host_with_trusted(trusted_hosts: &[String]) -> ListeningHost {
    let dist = test_temp_dir("dsh-host-dist");
    std::fs::write(
        dist.join("index.html"),
        "<html><head></head><body></body></html>",
    )
    .unwrap();
    let client_packages = test_temp_dir("dsh-host-pkgs");
    let bind = HostBind::new("127.0.0.1", 0).expect("loopback bind");
    let paths = HostPaths::new(dist, client_packages);
    let state =
        HostState::new(bind, paths, trusted_hosts.to_vec(), StubHandler).expect("host state");
    serve(state).await.expect("listen")
}

#[cfg(test)]
fn test_temp_dir(prefix: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "{prefix}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

#[cfg(test)]
mod tests {
    use super::{spawn_stub_host, spawn_stub_host_with_trusted};

    #[tokio::test]
    async fn host_describe_returns_server_response_200() {
        let host = spawn_stub_host().await;
        let url = format!("http://{}/api/host.describe", host.local_addr());
        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header("host", host.local_addr().to_string())
            .header("content-type", "application/json")
            .body(r#"{"type":"client-request","rpcId":"r1","method":"host.describe","payload":{}}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let v: serde_json::Value = response.json().await.unwrap();
        assert_eq!(v["type"], "server-response");
        assert_eq!(v["rpcId"], "r1");
        assert_eq!(v["result"]["ok"], true);
        assert_eq!(v["result"]["value"]["canOpenPath"], false);
        assert_eq!(v["result"]["value"]["version"], "0.0.1");
        host.shutdown().await;
    }

    #[tokio::test]
    async fn non_json_is_415() {
        let host = spawn_stub_host().await;
        let url = format!("http://{}/api/host.describe", host.local_addr());
        let response = reqwest::Client::new()
            .post(&url)
            .header("host", host.local_addr().to_string())
            .header("content-type", "text/plain")
            .body("nope")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 415);
        host.shutdown().await;
    }

    #[tokio::test]
    async fn get_events_mux_is_426() {
        let host = spawn_stub_host().await;
        let url = format!("http://{}/api/events.mux", host.local_addr());
        let response = reqwest::Client::new()
            .get(&url)
            .header("host", host.local_addr().to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 426);
        let upgrade = response.headers().get("upgrade").unwrap().to_str().unwrap();
        assert!(upgrade.eq_ignore_ascii_case("websocket"), "{upgrade}");
        host.shutdown().await;
    }

    #[tokio::test]
    async fn missing_host_header_on_api_is_403() {
        let host = spawn_stub_host().await;
        // reqwest always sends Host; use a raw tcp write or hyper client with empty host if needed.
        // Pin 403 by sending Host: evil.example against empty trustedHosts.
        let url = format!("http://{}/api/host.describe", host.local_addr());
        let response = reqwest::Client::new()
            .post(&url)
            .header("host", "evil.example")
            .header("content-type", "application/json")
            .body(r#"{"type":"client-request","rpcId":"r1","method":"host.describe","payload":{}}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 403);
        host.shutdown().await;
    }

    #[tokio::test]
    async fn settings_describe_from_non_loopback_host_is_403_even_if_trusted() {
        let host = spawn_stub_host_with_trusted(&["evil.example".into()]).await;
        let url = format!("http://{}/api/settings.describe", host.local_addr());
        let response = reqwest::Client::new()
            .post(&url)
            .header("host", "evil.example")
            .header("content-type", "application/json")
            .body(r#"{"type":"client-request","rpcId":"r1","method":"settings.describe","payload":{}}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 403);
        host.shutdown().await;
    }
}
