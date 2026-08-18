//! DeepSeek `POST /chat/completions` SSE adapter.

mod adapter;
pub mod plugin;
mod serialize;
mod sse;
mod translate;
mod types;

pub use adapter::{
    DEFAULT_CONTEXT_WINDOW, DEFAULT_MAX_TOKENS, DEFAULT_STREAM_IDLE_TIMEOUT_MS, DeepSeekAdapter,
    DeepSeekConnectionOptions, http_error_code,
};
pub use serialize::{RequestDefaults, serialize_messages, serialize_request};
pub use sse::{DONE, parse_sse};
pub use translate::{map_finish_reason, map_usage, translate};
pub use types::{
    StreamOptions, ThinkingField, ThinkingMode, WireMessage, WireRequest, WireTool, WireToolCall,
    WireUsage,
};
