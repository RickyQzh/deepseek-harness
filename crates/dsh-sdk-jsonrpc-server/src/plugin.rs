//! Kernel plugin `sdk-jsonrpc-server`.

use std::sync::Arc;

use dsh_agent::AgentRegistry;
use dsh_boot::{PLUGIN_SDK_JSONRPC, PluginRegistry, PluginSetup};
use dsh_kernel::KernelError;
use dsh_sdk_protocol::JsonRpcLineTransport;
use dsh_session_persist::JsonlSessionStore;
use tokio::io::BufReader;

use crate::server::HarnessSdkJsonRpcServer;

/// Kernel service name the bin reads after `boot_yaml`.
pub const SDK_JSONRPC_SERVER_SERVICE: &str = "sdkJsonRpcServer";

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Bind stdio JSON-RPC and provide [`SDK_JSONRPC_SERVER_SERVICE`]. The bin serves after boot so sibling adapter registration is visible to `initialize`.
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, _config| {
        Box::pin(async move {
            let agents = ctx.inject::<AgentRegistry>("agents").await?;
            let sessions = ctx.inject::<JsonlSessionStore>("sessions").await?;
            let stdin = BufReader::new(tokio::io::stdin());
            let stdout = tokio::io::stdout();
            let transport = JsonRpcLineTransport::new(stdin, stdout);
            let server = HarnessSdkJsonRpcServer::new(agents, sessions, transport, ctx.clone());
            server.set_exit_hook(Arc::new(|code| std::process::exit(code)));
            server.bind();
            ctx.provide(SDK_JSONRPC_SERVER_SERVICE, server)
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_SDK_JSONRPC, setup);
}
