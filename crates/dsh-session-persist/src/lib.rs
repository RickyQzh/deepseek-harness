//! JSONL and zstd session-log codec and live JSONL store for the DeepSeek Harness Rust host.

mod error;
mod header_line;
mod jsonl;
pub mod plugin;
mod store;
mod zstd;

#[cfg(test)]
mod phase5_exit;

#[cfg(test)]
mod phase6_exit;

pub use error::PersistError;
pub use header_line::{
    HeaderLine, HeaderLineType, from_header_line, parse_header_record, to_header_line,
};
pub use jsonl::{decode_session_log, encode_session_log};
pub use store::JsonlSessionStore;
pub use zstd::{compress_zstd_frame, decompress_zstd_frames};
