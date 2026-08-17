//! In-process slash-command registry for the DeepSeek Harness Rust host.

pub mod plugin;
mod registry;

#[cfg(test)]
mod phase7_exit;

pub use registry::{
    CommandDefinition, CommandDescriptor, CommandExecution, CommandHandler, CommandRegistry,
    CommandResult, Dispose, ParsedCommand, RegisterError,
};
