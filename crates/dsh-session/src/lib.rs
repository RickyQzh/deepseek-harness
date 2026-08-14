//! Append-only session log types for the Rust host.

mod catalog;
mod header;
mod ids;

pub use catalog::{KNOWN_SESSION_EVENT_TYPES, is_known_session_event_type};
pub use header::{SESSION_FORMAT_VERSION, SessionHeader, SessionOrigin};
pub use ids::{CallId, CallIdTag, MessageId, MessageIdTag, SessionId, SessionIdTag};
