//! Closed map from YAML `name` to setup closure.

use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::Arc;

use dsh_kernel::{Context, KernelError};
use serde_json::Value;

/// Async plugin setup: `(ctx, interpolated config) -> Result`.
pub type PluginSetup = Arc<
    dyn Fn(
            Context,
            Value,
        ) -> Pin<Box<dyn std::future::Future<Output = Result<(), KernelError>> + Send>>
        + Send
        + Sync,
>;

/// In-process plugin name registry. Missing names are load errors.
#[derive(Clone, Default)]
pub struct PluginRegistry {
    plugins: BTreeMap<String, PluginSetup>,
}

impl PluginRegistry {
    /// Empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            plugins: BTreeMap::new(),
        }
    }

    /// Register `setup` under YAML `name`. A later registration replaces the earlier one.
    pub fn register(&mut self, name: impl Into<String>, setup: PluginSetup) {
        self.plugins.insert(name.into(), setup);
    }

    /// Lookup by YAML `name`.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&PluginSetup> {
        self.plugins.get(name)
    }
}
