//! JSONL and zstd session-log codec for the DeepSeek Harness Rust host.

mod error;
mod header_line;
mod jsonl;
mod zstd;

pub use error::PersistError;
pub use header_line::{
    HeaderLine, HeaderLineType, from_header_line, parse_header_record, to_header_line,
};
pub use jsonl::{decode_session_log, encode_session_log};
pub use zstd::{compress_zstd_frame, decompress_zstd_frames};
