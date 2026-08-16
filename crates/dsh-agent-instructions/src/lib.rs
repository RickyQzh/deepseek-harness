//! Workspace `AGENTS.md` / `CLAUDE.md` baseline injection for the Rust host.

mod config;
mod files;
mod plugin;
mod render;

pub use config::{AgentInstructionsConfig, ConfigError, resolve_config};
pub use files::{LoadedInstructionFile, load_baseline_files};
pub use plugin::register;
pub use render::{AgentInstructionChange, render_workspace_context, workspace_baseline_identity};
