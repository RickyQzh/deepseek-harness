//! One-shot headless driver: print last assistant text and exit on `turn/end`.

mod error;
mod io;
mod plugin;
mod runner;
mod startup;
mod summarize;

/// Bundled Phase 5 composition (mock LLM, no `!!js`).
pub const MINIMAL_YAML: &str = include_str!("../minimal.cordis.yml");

pub use error::HeadlessError;
pub use io::{AppExit, CmdlineArgs, HeadlessIo, HeadlessStartup};
pub use plugin::register_headless_plugins;
pub use summarize::{RunOutcome, summarize};
