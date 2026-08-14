//! Closed YAML compose dialect.

mod error;
mod interpolate;

pub use error::ComposeError;
pub use interpolate::{InterpolateEnv, interpolate, interpolate_value, reject_js_tags};
