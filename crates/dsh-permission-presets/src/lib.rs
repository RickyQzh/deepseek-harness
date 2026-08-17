//! Pin permission presets, sandbox mode, and approval policy at session creation.

pub mod plugin;
mod service;

#[cfg(test)]
mod phase6_exit;

pub use service::{
    CUSTOM_PRESET, PermissionError, PermissionPresetConfig, PermissionPresetService, PresetSpec,
    effective_permission_preset, effective_sandbox_mode, set_sandbox_mode,
    set_sandbox_mode_in_world, set_sandbox_mode_with_terminals,
};
