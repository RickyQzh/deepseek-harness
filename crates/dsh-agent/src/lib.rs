//! Live `LoopAgent` registry: create, resume, followup, and run_until_idle behind a mutex.

mod error;
pub mod plugin;
mod registry;
mod spine;

#[cfg(test)]
mod phase5_exit;

#[cfg(test)]
mod phase6_exit;

pub use error::AgentError;
pub use registry::{AgentHandle, AgentRegistry, CreateAgentOptions};
pub use spine::{register_execution_plugins, register_spine_plugins};
