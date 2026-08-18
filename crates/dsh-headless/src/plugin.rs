//! Register headless YAML names including auto-approve.

use dsh_boot::PluginRegistry;

/// `headless-auto-approve`, then `headless-startup`, then `headless-runner`.
pub fn register_headless_plugins(registry: &mut PluginRegistry) {
    dsh_user_approval::plugin::register_auto_approve(registry);
    crate::startup::register(registry);
    crate::runner::register(registry);
}
