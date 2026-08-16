//! In-process spawn and fork one-shot subagent providers and the shared child driver.

mod driver;
mod fork;
pub mod plugin;
mod spawn;

pub use driver::start_in_process_run;
pub use plugin::{register_fork, register_spawn};
