//! Live `LoopAgent` registry: create, followup, and run_until_idle behind a mutex.

mod error;
mod registry;

pub use error::AgentError;
pub use registry::{AgentHandle, AgentRegistry, CreateAgentOptions};
