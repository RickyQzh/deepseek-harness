//! Closed YAML compose dialect.

mod disabled;
mod error;
mod interpolate;
mod patch;

pub use disabled::{Disabled, DisabledPredicate, is_disabled};
pub use error::ComposeError;
pub use interpolate::{InterpolateEnv, interpolate, interpolate_value, reject_js_tags};
pub use patch::{
    Entry, Layer, Patch, apply_entry_patches, compose_layers, compose_named_layers, dump_config,
    parse_yaml_entries, parse_yaml_patches, validate_configs,
};
