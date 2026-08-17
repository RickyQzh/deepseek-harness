//! Streaming terminal-control sanitizer for line-oriented PTY output.

/// OSC marker emitted by the controlled bash before each prompt.
pub(crate) const PROMPT_MARKER_PREFIX: &str = "133;D;";

/// Exact printable prompt emitted after the private marker.
pub(crate) const CONTROLLED_PROMPT: &str = "dsh> ";

#[derive(Clone, Copy)]
enum DiscardMode {
    Osc,
    Csi,
}

/// One sanitized chunk plus whether it contained the owned prompt marker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SanitizedChunk {
    pub(crate) text: String,
    pub(crate) prompt: bool,
    pub(crate) prompt_tail: Option<String>,
}

/// Remove CSI/OSC/short escape sequences while preserving split-sequence carry.
pub(crate) struct TerminalSanitizer {
    pending: String,
    discard_mode: Option<DiscardMode>,
    discard_osc_escape: bool,
    trailing_carriage_return: bool,
    tracking_prompt_tail: bool,
    max_pending_bytes: usize,
}

impl TerminalSanitizer {
    pub(crate) fn new(max_pending_bytes: usize) -> Self {
        Self {
            pending: String::new(),
            discard_mode: None,
            discard_osc_escape: false,
            trailing_carriage_return: false,
            tracking_prompt_tail: false,
            max_pending_bytes,
        }
    }

    /// Consume one decoded terminal data chunk.
    pub(crate) fn push(&mut self, chunk: &str) -> SanitizedChunk {
        let prefix = self.discard_prefix(chunk);
        self.pending.push_str(&prefix);
        let mut text = String::new();
        let mut prompt = false;
        let mut include_prompt_tail = self.tracking_prompt_tail;
        let mut prompt_tail = String::new();
        let mut index = 0;
        let pending = self.pending.clone();
        let append_text = |dest: &mut String, tail: &mut String, tracking: bool, value: &str| {
            dest.push_str(value);
            if tracking {
                tail.push_str(value);
            }
        };
        while index < pending.len() {
            let relative = pending[index..].find('\x1b');
            let Some(rel) = relative else {
                append_text(
                    &mut text,
                    &mut prompt_tail,
                    self.tracking_prompt_tail,
                    &pending[index..],
                );
                index = pending.len();
                break;
            };
            let escape = index + rel;
            append_text(
                &mut text,
                &mut prompt_tail,
                self.tracking_prompt_tail,
                &pending[index..escape],
            );
            if escape + 1 >= pending.len() {
                index = escape;
                break;
            }
            let kind = pending.as_bytes()[escape + 1];
            if kind == b']' {
                let rest = &pending[escape + 2..];
                let bel = rest.find('\x07').map(|i| escape + 2 + i);
                let string_terminator = rest.find("\x1b\\").map(|i| escape + 2 + i);
                let end = match (bel, string_terminator) {
                    (Some(b), Some(s)) => Some(b.min(s) + if b <= s { 1 } else { 2 }),
                    (Some(b), None) => Some(b + 1),
                    (None, Some(s)) => Some(s + 2),
                    (None, None) => None,
                };
                let Some(end) = end else {
                    index = escape;
                    break;
                };
                let terminator_bytes = if pending.as_bytes()[end - 1] == 0x07 {
                    1
                } else {
                    2
                };
                let content = &pending[escape + 2..end - terminator_bytes];
                if content.starts_with(PROMPT_MARKER_PREFIX) {
                    prompt = true;
                    self.tracking_prompt_tail = true;
                    include_prompt_tail = true;
                    prompt_tail.clear();
                }
                index = end;
                continue;
            }
            if kind == b'[' {
                let bytes = pending.as_bytes();
                let mut end = escape + 2;
                while end < bytes.len() {
                    let code = bytes[end];
                    if (0x40..=0x7e).contains(&code) {
                        break;
                    }
                    end += 1;
                }
                if end >= bytes.len() {
                    index = escape;
                    break;
                }
                index = end + 1;
                continue;
            }
            index = escape + 2;
        }
        self.pending = pending[index..].to_string();
        self.enforce_pending_bound();
        SanitizedChunk {
            text: self.normalize_text(&text),
            prompt,
            prompt_tail: if include_prompt_tail {
                Some(prompt_tail)
            } else {
                None
            },
        }
    }

    /// Flush a trailing printable fragment when the PTY exits.
    pub(crate) fn flush(&mut self) -> String {
        let text = if self.pending.starts_with('\x1b') {
            String::new()
        } else {
            self.pending.clone()
        };
        self.pending.clear();
        self.discard_mode = None;
        self.discard_osc_escape = false;
        self.tracking_prompt_tail = false;
        let normalized = self.normalize_text(&text);
        if !self.trailing_carriage_return {
            return normalized;
        }
        self.trailing_carriage_return = false;
        format!("{normalized}\n")
    }

    fn normalize_text(&mut self, text: &str) -> String {
        let mut complete = if self.trailing_carriage_return {
            format!("\r{text}")
        } else {
            text.to_string()
        };
        self.trailing_carriage_return = false;
        if complete.ends_with('\r') {
            complete.pop();
            self.trailing_carriage_return = true;
        }
        normalize_terminal_text(&complete)
    }

    fn enforce_pending_bound(&mut self) {
        if self.pending.len() <= self.max_pending_bytes {
            return;
        }
        let second = self.pending.as_bytes().get(1).copied();
        self.discard_mode = if second == Some(b']') {
            Some(DiscardMode::Osc)
        } else {
            Some(DiscardMode::Csi)
        };
        self.pending.clear();
    }

    fn discard_prefix(&mut self, chunk: &str) -> String {
        let Some(mode) = self.discard_mode else {
            return chunk.to_string();
        };
        match mode {
            DiscardMode::Csi => {
                for (index, byte) in chunk.bytes().enumerate() {
                    if (0x40..=0x7e).contains(&byte) {
                        self.discard_mode = None;
                        return chunk[index + 1..].to_string();
                    }
                }
                String::new()
            }
            DiscardMode::Osc => {
                let bytes = chunk.as_bytes();
                let mut index = 0;
                if self.discard_osc_escape {
                    self.discard_osc_escape = false;
                    if let Some(rest) = chunk.strip_prefix('\\') {
                        self.discard_mode = None;
                        return rest.to_string();
                    }
                }
                while index < bytes.len() {
                    if bytes[index] == 0x07 {
                        self.discard_mode = None;
                        return chunk[index + 1..].to_string();
                    }
                    if bytes[index] == 0x1b {
                        if bytes.get(index + 1) == Some(&b'\\') {
                            self.discard_mode = None;
                            return chunk[index + 2..].to_string();
                        }
                        if index + 1 == bytes.len() {
                            self.discard_osc_escape = true;
                        }
                    }
                    index += 1;
                }
                String::new()
            }
        }
    }
}

fn normalize_terminal_text(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\u{0007}', "")
}

#[cfg(test)]
mod tests {
    use super::{CONTROLLED_PROMPT, TerminalSanitizer};

    #[test]
    fn removes_split_csi_and_owned_osc_prompt_markers() {
        let mut sanitizer = TerminalSanitizer::new(64);
        let first = sanitizer.push("red\x1b[3");
        assert_eq!(first.text, "red");
        assert!(!first.prompt);
        let second = sanitizer.push("1m text\x1b[0m\r\n");
        assert_eq!(second.text, " text\n");
        assert!(!second.prompt);
        let third = sanitizer.push("\x1b]133;");
        assert_eq!(third.text, "");
        assert!(!third.prompt);
        let fourth = sanitizer.push("D;0\x07dsh> ");
        assert_eq!(fourth.text, CONTROLLED_PROMPT);
        assert!(fourth.prompt);
        assert_eq!(fourth.prompt_tail.as_deref(), Some(CONTROLLED_PROMPT));
    }
}
