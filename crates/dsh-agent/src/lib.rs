//! Live `LoopAgent` registry: create, followup, and run_until_idle behind a mutex.

mod error;
pub mod plugin;
mod registry;
mod spine;

pub use error::AgentError;
pub use registry::{AgentHandle, AgentRegistry, CreateAgentOptions};
pub use spine::register_spine_plugins;
