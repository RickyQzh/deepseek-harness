//! Append-only session log types for the Rust host.

mod header;
mod ids;

pub use header::{SESSION_FORMAT_VERSION, SessionHeader, SessionOrigin};
pub use ids::{CallId, CallIdTag, MessageId, MessageIdTag, SessionId, SessionIdTag};
