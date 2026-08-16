//! Kernel plugin `sdk-jsonrpc-server`.

use std::sync::Arc;

use dsh_agent::AgentRegistry;
use dsh_boot::{PLUGIN_SDK_JSONRPC, PluginRegistry, PluginSetup};
use dsh_sdk_protocol::JsonRpcLineTransport;
use dsh_session_persist::JsonlSessionStore;
use tokio::io::{AsyncWriteExt, BufReader};

use crate::server::HarnessSdkJsonRpcServer;

/// Serve NDJSON JSON-RPC on stdin/stdout. Stdout is frames only.
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, _config| {
        Box::pin(async move {
            let agents = ctx.inject::<AgentRegistry>("agents").await?;
            let sessions = ctx.inject::<JsonlSessionStore>("sessions").await?;
            let stdin = BufReader::new(tokio::io::stdin());
            let stdout = tokio::io::stdout();
            let transport = JsonRpcLineTransport::new(stdin, stdout);
            let server = HarnessSdkJsonRpcServer::new(agents, sessions, transport.clone());
            server.set_exit_hook(Arc::new(|code| std::process::exit(code)));
            server.bind();
            tokio::spawn(async move {
                let _ = transport.serve().await;
                let mut out = tokio::io::stdout();
                let _ = out.flush().await;
                std::process::exit(0);
            });
            Ok(())
        })
    });
    registry.register(PLUGIN_SDK_JSONRPC, setup);
}
