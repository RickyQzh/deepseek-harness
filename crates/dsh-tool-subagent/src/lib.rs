//! Model-facing subagent delegation, control, list, and report tools.

pub mod control;
pub mod delegate;
pub mod list;
pub mod plugin;
pub mod report;
mod util;

pub use plugin::register;
