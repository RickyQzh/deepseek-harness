//! Register both headless YAML names.

use dsh_boot::PluginRegistry;

/// `headless-startup` then `headless-runner`.
pub fn register_headless_plugins(registry: &mut PluginRegistry) {
    crate::startup::register(registry);
    crate::runner::register(registry);
}
