//! Decode a DeepSeek SSE byte stream into `data` payloads.

use dsh_llm::LlmError;
use tokio::io::{AsyncBufRead, AsyncBufReadExt};

/// The terminal payload DeepSeek (and OpenAI) send after the last chunk.
pub const DONE: &str = "[DONE]";

pub(crate) struct SseParser {
    leftover: Vec<u8>,
    data_lines: Vec<String>,
    saw_data: bool,
}

impl SseParser {
    pub(crate) fn new() -> Self {
        Self {
            leftover: Vec::new(),
            data_lines: Vec::new(),
            saw_data: false,
        }
    }

    fn dispatch(&mut self) -> Option<String> {
        if !self.saw_data {
            self.data_lines.clear();
            return None;
        }
        let payload = self.data_lines.join("\n");
        self.data_lines.clear();
        self.saw_data = false;
        Some(payload)
    }

    pub(crate) fn push_line(
        &mut self,
        line: &str,
        on_comment: &mut impl FnMut(&str),
    ) -> Option<String> {
        if line.is_empty() {
            return self.dispatch();
        }
        if let Some(rest) = line.strip_prefix(':') {
            on_comment(rest.strip_prefix(' ').unwrap_or(rest));
            return None;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name == "data" {
                self.data_lines
                    .push(value.strip_prefix(' ').unwrap_or(value).to_string());
                self.saw_data = true;
            }
            return None;
        }
        if line == "data" {
            self.data_lines.push(String::new());
            self.saw_data = true;
        }
        None
    }

    pub(crate) fn push_bytes(
        &mut self,
        bytes: &[u8],
        on_comment: &mut impl FnMut(&str),
    ) -> Result<Vec<String>, LlmError> {
        self.leftover.extend_from_slice(bytes);
        let mut payloads = Vec::new();
        while let Some(pos) = self.leftover.iter().position(|&byte| byte == b'\n') {
            let mut line: Vec<u8> = self.leftover.drain(..=pos).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = std::str::from_utf8(&line)
                .map_err(|_| LlmError::new("SSE line is not valid UTF-8", "MALFORMED_RESPONSE"))?;
            if let Some(payload) = self.push_line(line, on_comment) {
                payloads.push(payload);
            }
        }
        Ok(payloads)
    }
}

/// Parse an SSE byte stream into data payloads, including the `[DONE]` sentinel.
///
/// Lines are `\n`-delimited with a trailing `\r` stripped. `data:` / `data: `
/// fields append to the current event and join with `\n`. Lines starting with
/// `:` are comments: `on_comment` receives the text after `:` and an optional
/// space. A blank line dispatches the joined data. `[DONE]` is collected and
/// then this function returns. EOF without `[DONE]`, including an unterminated
/// tail with no blank-line terminator, is `STREAM_CLOSED`.
///
/// # Errors
///
/// Returns [`LlmError`] with code `STREAM_CLOSED` when the stream ends without
/// a dispatched `[DONE]`, or `MALFORMED_RESPONSE` when a line is not UTF-8.
pub async fn parse_sse<R: AsyncBufRead + Unpin>(
    mut reader: R,
    mut on_comment: impl FnMut(&str),
) -> Result<Vec<String>, LlmError> {
    let mut parser = SseParser::new();
    let mut payloads = Vec::new();
    loop {
        let mut line = Vec::new();
        let n = reader
            .read_until(b'\n', &mut line)
            .await
            .map_err(|error| LlmError::new(format!("SSE read failed: {error}"), "STREAM_CLOSED"))?;
        if n == 0 || !line.ends_with(b"\n") {
            return Err(LlmError::new(
                "SSE stream ended without [DONE]",
                "STREAM_CLOSED",
            ));
        }
        line.pop();
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        let line = std::str::from_utf8(&line)
            .map_err(|_| LlmError::new("SSE line is not valid UTF-8", "MALFORMED_RESPONSE"))?;
        if let Some(payload) = parser.push_line(line, &mut on_comment) {
            payloads.push(payload.clone());
            if payload == DONE {
                return Ok(payloads);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DONE, parse_sse};
    use std::io::Cursor;

    #[tokio::test]
    async fn yields_payloads_and_done() {
        let raw = b"data: {\"ok\":true}\n\ndata: [DONE]\n\n";
        let payloads = parse_sse(Cursor::new(&raw[..]), |_| {}).await.unwrap();
        assert_eq!(
            payloads,
            vec!["{\"ok\":true}".to_string(), DONE.to_string()]
        );
    }

    #[tokio::test]
    async fn eof_without_done_is_stream_closed() {
        let raw = b"data: {\"ok\":true}\n\n";
        let error = parse_sse(Cursor::new(&raw[..]), |_| {})
            .await
            .expect_err("closed");
        assert_eq!(error.code, "STREAM_CLOSED");
    }

    #[tokio::test]
    async fn unterminated_tail_is_not_flushed() {
        let raw = b"data: {\"ok\":true}";
        let error = parse_sse(Cursor::new(&raw[..]), |_| {})
            .await
            .expect_err("tail");
        assert_eq!(error.code, "STREAM_CLOSED");
    }

    #[tokio::test]
    async fn comments_do_not_enter_payloads() {
        let raw = b": keep-alive\n\ndata: [DONE]\n\n";
        let mut comments = Vec::new();
        let payloads = parse_sse(Cursor::new(&raw[..]), |c| comments.push(c.to_string()))
            .await
            .unwrap();
        assert_eq!(payloads, vec![DONE.to_string()]);
        assert_eq!(comments, vec!["keep-alive".to_string()]);
    }
}
