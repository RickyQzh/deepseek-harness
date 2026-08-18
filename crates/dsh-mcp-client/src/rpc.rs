//! LSP `Content-Length` JSON-RPC 2.0 framing for MCP stdio.
//!
//! Frames are `Content-Length: N\r\n\r\n` plus `N` body bytes. Header names are
//! matched case-insensitively; extra headers such as `Content-Type` are ignored.
//! A bare JSON line without that header block is not a frame.

use std::io::ErrorKind;

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt};

/// MCP stdio JSON-RPC framing failure.
#[derive(Debug, thiserror::Error)]
pub enum McpRpcError {
    /// The stream ended before a `\r\n\r\n` header terminator.
    #[error("MCP stdio frame ended before a Content-Length header block")]
    IncompleteHeaders,
    /// The header block has no `Content-Length` field.
    #[error("MCP stdio frame is missing Content-Length")]
    MissingContentLength,
    /// `Content-Length` is not a `usize` byte count.
    #[error("MCP stdio Content-Length is invalid: {0}")]
    InvalidContentLength(String),
    /// The stream ended before the declared body length.
    #[error("MCP stdio frame body ended before Content-Length bytes")]
    IncompleteBody,
    /// A read from the byte stream failed.
    #[error("MCP stdio read failed: {0}")]
    Io(String),
    /// A write to the byte stream failed.
    #[error("MCP stdio write failed: {0}")]
    Write(String),
    /// Frame body bytes are not a JSON value.
    #[error("MCP JSON-RPC body is not JSON: {0}")]
    InvalidJson(String),
    /// JSON-RPC response `error` object.
    #[error("MCP JSON-RPC error: {0}")]
    Rpc(String),
    /// JSON-RPC response or listed tool is missing required fields.
    #[error("MCP JSON-RPC response is invalid: {0}")]
    InvalidResponse(String),
}

impl McpRpcError {
    /// Failure text (same as [`std::fmt::Display`]).
    ///
    /// # Returns
    ///
    /// The diagnostic string for this error.
    #[must_use]
    pub fn message(&self) -> String {
        self.to_string()
    }
}

/// Encode `body` as one MCP stdio frame.
///
/// Writes `Content-Length: N\r\n\r\n` then `body`. `N` is the byte length of
/// `body`, not a character count. The header name is the TypeScript MCP SDK
/// spelling `Content-Length`.
///
/// # Parameters
///
/// * `body` - JSON-RPC message bytes, not yet framed.
///
/// # Returns
///
/// Framed bytes ready to write to the server's stdin.
#[must_use]
pub fn encode_frame(body: &[u8]) -> Vec<u8> {
    let mut framed = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    framed.extend_from_slice(body);
    framed
}

/// Read one MCP stdio frame from `reader`.
///
/// Reads headers through `\r\n\r\n`, parses `Content-Length` (header name matched
/// case-insensitively), ignores other headers, then reads exactly that many body
/// bytes. End of stream before the terminator is an error; a bare JSON line is
/// not a frame.
///
/// # Parameters
///
/// * `reader` - Buffered MCP stdio byte stream.
///
/// # Returns
///
/// The body bytes, excluding headers.
///
/// # Errors
///
/// [`McpRpcError`] when the stream ends before `\r\n\r\n`, `Content-Length` is
/// missing or not a `usize`, the body is truncated, or a read fails.
pub async fn read_frame<R: AsyncBufRead + Unpin>(reader: &mut R) -> Result<Vec<u8>, McpRpcError> {
    let mut headers = Vec::new();
    loop {
        let mut line = Vec::new();
        let n = reader
            .read_until(b'\n', &mut line)
            .await
            .map_err(|err| McpRpcError::Io(err.to_string()))?;
        if n == 0 {
            return Err(McpRpcError::IncompleteHeaders);
        }
        headers.extend_from_slice(&line);
        if headers.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let len = parse_content_length(&headers)?;
    let mut body = vec![0_u8; len];
    match reader.read_exact(&mut body).await {
        Ok(_) => Ok(body),
        Err(err) if err.kind() == ErrorKind::UnexpectedEof => Err(McpRpcError::IncompleteBody),
        Err(err) => Err(McpRpcError::Io(err.to_string())),
    }
}

fn parse_content_length(header_block: &[u8]) -> Result<usize, McpRpcError> {
    let Some(headers) = header_block.strip_suffix(b"\r\n\r\n") else {
        return Err(McpRpcError::IncompleteHeaders);
    };
    let Ok(headers) = std::str::from_utf8(headers) else {
        return Err(McpRpcError::InvalidContentLength(
            "header block is not UTF-8".to_string(),
        ));
    };
    for line in headers.split("\r\n") {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if !name.trim().eq_ignore_ascii_case("content-length") {
            continue;
        }
        let value = value.trim();
        match value.parse::<usize>() {
            Ok(len) => return Ok(len),
            Err(_) => return Err(McpRpcError::InvalidContentLength(value.to_string())),
        }
    }
    Err(McpRpcError::MissingContentLength)
}

#[cfg(test)]
mod tests {
    use super::{encode_frame, read_frame};
    use std::io::Cursor;

    #[tokio::test]
    async fn encode_frame_prefixes_content_length() {
        let body = br#"{"a":1}"#;
        let framed = encode_frame(body);
        assert!(
            framed.starts_with(b"Content-Length: "),
            "frame must start with Content-Length"
        );
        let after_name = &framed[b"Content-Length: ".len()..];
        let sep = after_name
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("header block must end at \\r\\n\\r\\n");
        let declared = std::str::from_utf8(&after_name[..sep]).expect("declared length is UTF-8");
        assert_eq!(
            declared.parse::<usize>().expect("declared length is usize"),
            body.len()
        );
        assert_eq!(&after_name[sep + 4..], body);
    }

    #[tokio::test]
    async fn read_frame_round_trips_json_object() {
        let body = serde_json::to_vec(&serde_json::json!({"jsonrpc": "2.0", "id": 1}))
            .expect("json object");
        let framed = encode_frame(&body);
        let mut reader = Cursor::new(framed);
        let got = read_frame(&mut reader).await.expect("round-trip frame");
        assert_eq!(got, body);
    }

    #[tokio::test]
    async fn read_frame_rejects_ndjson_without_headers() {
        let mut reader = Cursor::new(*b"{\"jsonrpc\":\"2.0\"}\n");
        let result = read_frame(&mut reader).await;
        assert!(result.is_err(), "bare JSON line without headers must fail");
    }
}
