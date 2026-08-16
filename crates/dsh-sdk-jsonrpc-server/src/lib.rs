//! SDK JSON-RPC server plugin and in-process server type.

mod plugin;
mod server;

/// Bundled Phase 5 composition: mock adapter claims `deepseek-official` so initialize needs no API key.
pub const MINIMAL_YAML: &str = include_str!("../minimal.cordis.yml");

pub use plugin::register;
pub use server::HarnessSdkJsonRpcServer;
