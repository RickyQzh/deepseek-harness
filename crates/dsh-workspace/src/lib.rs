//! Durable JSON workspace registry for the DeepSeek Harness Rust host.

mod error;
mod ids;
pub mod plugin;
mod registry;

#[cfg(test)]
mod phase7_exit;

pub use error::WorkspaceError;
pub use ids::{WorkspaceId, WorkspaceIdTag};
pub use registry::{ListResult, WorkspaceRegistry, WorkspaceView, default_persist_path};
