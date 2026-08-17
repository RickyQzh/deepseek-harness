//! GUI host loopback HTTP listener, `/api` trust fence, WebSocket downlinks, static SPA files, plugin bundles, and `__DSH_BOOT__` injection.

mod boot;
mod dispatch;
mod lookup;
mod methods;
mod plugins;
mod respond;
mod server;
mod static_files;
mod trust;
mod ws;

pub use boot::{WebBootEntry, WebBootGraph, inject_boot_manifest};
pub use dispatch::{RpcHandler, StubHandler};
pub use lookup::{AgentLookup, DEFAULT_MODEL, DEFAULT_PROVIDER, LookupError};
pub use methods::SessionHandler;
pub use plugins::{HostError, scan_client_packages, serve_plugin_js, serve_plugin_source_map};
pub use respond::RespondTable;
pub use server::{HostBind, HostPaths, HostState, ListeningHost, serve};
pub use static_files::{StaticResponse, serve_spa};
pub use trust::{
    TrustError, assert_trusted_authority, is_loopback_hostname, is_privileged_method,
    is_trusted_api_request, privileged_requires_loopback,
};
pub use ws::DownlinkHub;
