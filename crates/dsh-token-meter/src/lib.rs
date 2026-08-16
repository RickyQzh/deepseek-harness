//! Replay-aware token measurement using a fixed four-characters-per-token heuristic.

mod estimate;
mod fold;
pub mod plugin;

pub use estimate::{
    BLOCK_OVERHEAD, CHARS_PER_TOKEN, ROLE_OVERHEAD, estimate_content, estimate_header,
    estimate_message,
};
pub use fold::{TokenMeasurement, TokenMeasurementBaseline, TokenMeter, TokenSurfaceNode};
