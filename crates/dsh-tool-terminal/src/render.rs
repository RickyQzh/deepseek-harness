//! Model and UI rendering for persistent terminal tool results.

use serde_json::Value;

const TRUNCATED: &str = "\n[output truncated]";

enum RetainKind {
    Head,
    Tail,
}

/// Running or exited top-level session status used by render helpers.
pub enum RenderedSessionStatus {
    /// The top-level process is still running.
    Running,
    /// The top-level process has exited.
    Exited {
        /// Process exit code, or none when the process was signaled without a code.
        exit_code: Option<i32>,
        /// Signal name when the process was terminated by a signal.
        signal: Option<String>,
    },
}

/// Owner-visible session fields used by spawn and list renderers.
pub struct RenderedSessionSnapshot {
    session_id: String,
    name: Option<String>,
    backend_type: String,
    pid: Option<i32>,
    status: RenderedSessionStatus,
}

/// Spawn acknowledgement passed to [`render_spawn`].
pub struct RenderedSpawnResult {
    session_id: String,
    name: Option<String>,
    backend_type: String,
    motd: String,
}

/// Settled foreground send passed to [`render_send`].
pub struct RenderedSendResult {
    viewport: String,
    wait_reason: String,
    session_status: RenderedSessionStatus,
    truncated: bool,
}

/// Incremental background send delta passed to [`render_send_read`].
pub struct RenderedSendRead {
    delta: String,
    truncated: bool,
}

/// Scrollback page passed to [`render_read`].
pub struct RenderedReadResult {
    text: String,
    total_lines: usize,
    line_begin: usize,
    line_end: usize,
    truncated: bool,
}

/// UI presentation intent for one terminal tool call.
pub struct PresentCall {
    card: String,
    title: String,
    kind: Option<String>,
    description: Option<String>,
}

impl PresentCall {
    /// Presentation card (`generic` or `terminal`).
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The card name.
    #[must_use]
    pub fn card(&self) -> &str {
        &self.card
    }

    /// Presentation title.
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The title text.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Optional presentation kind (`execute`, `read`, or `delete`).
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The kind when this intent is a generic card.
    #[must_use]
    pub fn kind(&self) -> Option<&str> {
        self.kind.as_deref()
    }

    /// Optional description (foreground send only).
    ///
    /// # Parameters
    ///
    /// None.
    ///
    /// # Returns
    ///
    /// The description when present.
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }
}

impl RenderedSpawnResult {
    /// Build one spawn render input.
    ///
    /// # Parameters
    ///
    /// * `session_id` - Registry-issued id.
    /// * `name` - Optional owner-local display name.
    /// * `backend_type` - Backend type such as `shell`.
    /// * `motd` - Initial output.
    ///
    /// # Returns
    ///
    /// The render input.
    #[must_use]
    pub fn new(
        session_id: impl Into<String>,
        name: Option<String>,
        backend_type: impl Into<String>,
        motd: impl Into<String>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            name,
            backend_type: backend_type.into(),
            motd: motd.into(),
        }
    }
}

impl RenderedSessionSnapshot {
    /// Build one list-row render input.
    ///
    /// # Parameters
    ///
    /// * `session_id` - Registry-issued id.
    /// * `name` - Optional owner-local display name.
    /// * `backend_type` - Backend type such as `shell`.
    /// * `pid` - Optional top-level pid.
    /// * `status` - Top-level status.
    ///
    /// # Returns
    ///
    /// The render input.
    #[must_use]
    pub fn new(
        session_id: impl Into<String>,
        name: Option<String>,
        backend_type: impl Into<String>,
        pid: Option<i32>,
        status: RenderedSessionStatus,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            name,
            backend_type: backend_type.into(),
            pid,
            status,
        }
    }
}

impl RenderedSendResult {
    /// Build one send render input.
    ///
    /// # Parameters
    ///
    /// * `viewport` - Bounded rendered terminal delta.
    /// * `wait_reason` - `stdin_read` / `inferred_idle` / `timeout` / `session_exit`.
    /// * `session_status` - Top-level status at settlement.
    /// * `truncated` - Whether output was dropped.
    ///
    /// # Returns
    ///
    /// The render input.
    #[must_use]
    pub fn new(
        viewport: impl Into<String>,
        wait_reason: impl Into<String>,
        session_status: RenderedSessionStatus,
        truncated: bool,
    ) -> Self {
        Self {
            viewport: viewport.into(),
            wait_reason: wait_reason.into(),
            session_status,
            truncated,
        }
    }
}

impl RenderedSendRead {
    /// Build one incremental send-read render input.
    ///
    /// # Parameters
    ///
    /// * `delta` - Output produced since the previous operation read.
    /// * `truncated` - Whether unread operation output was dropped.
    ///
    /// # Returns
    ///
    /// The render input.
    #[must_use]
    pub fn new(delta: impl Into<String>, truncated: bool) -> Self {
        Self {
            delta: delta.into(),
            truncated,
        }
    }
}

impl RenderedReadResult {
    /// Build one history-page render input.
    ///
    /// # Parameters
    ///
    /// * `text` - Retained text in chronological order.
    /// * `total_lines` - Number of lines currently retained.
    /// * `line_begin` - Inclusive newest-relative offset of the first returned line.
    /// * `line_end` - Exclusive newest-relative offset after the returned page.
    /// * `truncated` - Whether a bound dropped output.
    ///
    /// # Returns
    ///
    /// The render input.
    #[must_use]
    pub fn new(
        text: impl Into<String>,
        total_lines: usize,
        line_begin: usize,
        line_end: usize,
        truncated: bool,
    ) -> Self {
        Self {
            text: text.into(),
            total_lines,
            line_begin,
            line_end,
            truncated,
        }
    }
}

fn byte_length(text: &str) -> usize {
    text.len()
}

fn retain(text: &str, max_bytes: usize, kind: RetainKind) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    match kind {
        RetainKind::Head => {
            let mut end = max_bytes.min(text.len());
            while end > 0 && !text.is_char_boundary(end) {
                end -= 1;
            }
            text[..end].to_string()
        }
        RetainKind::Tail => {
            let mut start = text.len().saturating_sub(max_bytes);
            while start < text.len() && !text.is_char_boundary(start) {
                start += 1;
            }
            text[start..].to_string()
        }
    }
}

fn fit_with_suffix(content: &str, suffix: &str, max_bytes: usize) -> String {
    let fixed_bytes = byte_length(suffix);
    if fixed_bytes >= max_bytes {
        return retain(suffix, max_bytes, RetainKind::Tail);
    }
    format!(
        "{}{suffix}",
        retain(content, max_bytes - fixed_bytes, RetainKind::Tail)
    )
}

fn fit_with_prefix(prefix: &str, content: &str, max_bytes: usize) -> String {
    let fixed = format!("{prefix}{TRUNCATED}");
    let fixed_bytes = byte_length(&fixed);
    if fixed_bytes >= max_bytes {
        return retain(&fixed, max_bytes, RetainKind::Head);
    }
    format!(
        "{prefix}{}{TRUNCATED}",
        retain(content, max_bytes - fixed_bytes, RetainKind::Tail)
    )
}

fn bound_body_with_suffix(
    content: &str,
    metadata: &str,
    upstream_truncated: bool,
    max_bytes: usize,
) -> String {
    let suffix = if upstream_truncated {
        format!("{metadata}{TRUNCATED}")
    } else {
        metadata.to_string()
    };
    let complete = format!("{content}{suffix}");
    if byte_length(&complete) <= max_bytes {
        return complete;
    }
    fit_with_suffix(content, &format!("{metadata}{TRUNCATED}"), max_bytes)
}

fn status_label(status: &RenderedSessionStatus) -> String {
    match status {
        RenderedSessionStatus::Running => "running".into(),
        RenderedSessionStatus::Exited { exit_code, signal } => {
            let code = exit_code
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".into());
            let signal = signal.clone().unwrap_or_else(|| "null".into());
            format!("exited code={code} signal={signal}")
        }
    }
}

/// Bound one complete terminal acknowledgement while preserving UTF-8 cuts.
///
/// # Parameters
///
/// * `text` - Complete acknowledgement text.
/// * `max_bytes` - Positive final result cap.
///
/// # Returns
///
/// Bounded text with a truncation marker when it fits.
#[must_use]
pub fn bound_terminal_text(text: &str, max_bytes: usize) -> String {
    if byte_length(text) <= max_bytes {
        return text.to_string();
    }
    let marker_bytes = byte_length(TRUNCATED);
    if marker_bytes >= max_bytes {
        return retain(TRUNCATED, max_bytes, RetainKind::Tail);
    }
    format!(
        "{}{TRUNCATED}",
        retain(text, max_bytes - marker_bytes, RetainKind::Head)
    )
}

/// Render one created session and its bounded MOTD.
///
/// # Parameters
///
/// * `result` - Published spawn result.
/// * `max_bytes` - Complete UTF-8 result cap.
///
/// # Returns
///
/// Model-facing session acknowledgement.
#[must_use]
pub fn render_spawn(result: &RenderedSpawnResult, max_bytes: usize) -> String {
    let label = match &result.name {
        None => result.session_id.clone(),
        Some(name) => format!("{} ({name})", result.session_id),
    };
    let prefix = format!(
        "started terminal session {label} [type: {}]\n",
        result.backend_type
    );
    let motd = if result.motd.is_empty() {
        "(no startup output)"
    } else {
        result.motd.as_str()
    };
    let complete = format!("{prefix}{motd}");
    if byte_length(&complete) <= max_bytes {
        complete
    } else {
        fit_with_prefix(&prefix, motd, max_bytes)
    }
}

/// Render one settled interactive send.
///
/// # Parameters
///
/// * `result` - Settled send outcome.
/// * `max_bytes` - Complete UTF-8 result cap.
///
/// # Returns
///
/// Terminal output plus wait/session markers.
#[must_use]
pub fn render_send(result: &RenderedSendResult, max_bytes: usize) -> String {
    let output = if result.viewport.is_empty() {
        "(no new output)"
    } else {
        result.viewport.as_str()
    };
    let status = status_label(&result.session_status);
    bound_body_with_suffix(
        output,
        &format!("\n[wait: {}]\n[session: {status}]", result.wait_reason),
        result.truncated,
        max_bytes,
    )
}

/// Render one incremental background operation read.
///
/// # Parameters
///
/// * `read` - Consuming operation delta.
///
/// # Returns
///
/// Delta plus its upstream truncation marker.
#[must_use]
pub fn render_send_read(read: &RenderedSendRead) -> String {
    let separator = if read.delta.ends_with('\n') || read.delta.is_empty() {
        ""
    } else {
        "\n"
    };
    if read.truncated {
        format!("{}{separator}[output truncated]", read.delta)
    } else {
        read.delta.clone()
    }
}

/// Render one bounded historical page.
///
/// # Parameters
///
/// * `result` - Retained scrollback page.
/// * `max_bytes` - Complete UTF-8 result cap.
///
/// # Returns
///
/// Page text plus pagination and truncation markers.
#[must_use]
pub fn render_read(result: &RenderedReadResult, max_bytes: usize) -> String {
    let output = if result.text.is_empty() {
        "(no retained output)"
    } else {
        result.text.as_str()
    };
    bound_body_with_suffix(
        output,
        &format!(
            "\n[lines: {}-{} of {}]",
            result.line_begin, result.line_end, result.total_lines
        ),
        result.truncated,
        max_bytes,
    )
}

/// Render owner-visible live sessions.
///
/// # Parameters
///
/// * `sessions` - Fresh owner-scoped snapshots.
/// * `max_bytes` - Complete UTF-8 result cap.
///
/// # Returns
///
/// One line per session or the empty marker.
#[must_use]
pub fn render_list(sessions: &[RenderedSessionSnapshot], max_bytes: usize) -> String {
    if sessions.is_empty() {
        return "(no terminal sessions)".into();
    }
    let text = sessions
        .iter()
        .map(|session| {
            let name = session
                .name
                .as_ref()
                .map(|name| format!(" ({name})"))
                .unwrap_or_default();
            let pid = session
                .pid
                .map(|pid| format!(" pid={pid}"))
                .unwrap_or_default();
            let status = status_label(&session.status);
            format!(
                "{}{name} [{}] {status}{pid}",
                session.session_id, session.backend_type
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    bound_body_with_suffix(&text, "", false, max_bytes)
}

fn arg_str(args: &Value, key: &str) -> String {
    args.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// Presentation intent for `terminal_open`.
///
/// # Parameters
///
/// * `args` - Tool arguments.
///
/// # Returns
///
/// Generic execute card titled `Open terminal {name ?? type}`.
#[must_use]
pub fn present_open(args: &Value) -> PresentCall {
    let name = args.get("name").and_then(Value::as_str);
    let backend_type = arg_str(args, "type");
    let label = name.unwrap_or(backend_type.as_str());
    PresentCall {
        card: "generic".into(),
        title: format!("Open terminal {label}"),
        kind: Some("execute".into()),
        description: None,
    }
}

/// Presentation intent for `terminal_send`.
///
/// # Parameters
///
/// * `args` - Tool arguments.
///
/// # Returns
///
/// Terminal card for foreground send, or generic execute for background send.
#[must_use]
pub fn present_send(args: &Value) -> PresentCall {
    let session_id = arg_str(args, "sessionId");
    if args.get("run_in_background") == Some(&Value::Bool(true)) {
        return PresentCall {
            card: "generic".into(),
            title: format!("Send to terminal {session_id} in background"),
            kind: Some("execute".into()),
            description: None,
        };
    }
    let text = arg_str(args, "text");
    let title = if text.is_empty() {
        "(send input)".into()
    } else {
        text
    };
    PresentCall {
        card: "terminal".into(),
        title,
        kind: None,
        description: Some(format!("Terminal {session_id}")),
    }
}

/// Presentation intent for `terminal_read`.
///
/// # Parameters
///
/// * `args` - Tool arguments.
///
/// # Returns
///
/// Generic read card titled `Read terminal {sessionId}`.
#[must_use]
pub fn present_read(args: &Value) -> PresentCall {
    PresentCall {
        card: "generic".into(),
        title: format!("Read terminal {}", arg_str(args, "sessionId")),
        kind: Some("read".into()),
        description: None,
    }
}

/// Presentation intent for `terminal_signal`.
///
/// # Parameters
///
/// * `args` - Tool arguments.
///
/// # Returns
///
/// Generic execute card titled `Signal terminal {sessionId}`.
#[must_use]
pub fn present_signal(args: &Value) -> PresentCall {
    PresentCall {
        card: "generic".into(),
        title: format!("Signal terminal {}", arg_str(args, "sessionId")),
        kind: Some("execute".into()),
        description: None,
    }
}

/// Presentation intent for `terminal_close`.
///
/// # Parameters
///
/// * `args` - Tool arguments.
///
/// # Returns
///
/// Generic delete card titled `Close terminal {sessionId}`.
#[must_use]
pub fn present_close(args: &Value) -> PresentCall {
    PresentCall {
        card: "generic".into(),
        title: format!("Close terminal {}", arg_str(args, "sessionId")),
        kind: Some("delete".into()),
        description: None,
    }
}

/// Presentation intent for `terminal_list`.
///
/// # Parameters
///
/// * `args` - Tool arguments.
///
/// # Returns
///
/// Generic read card titled `List terminal sessions`.
#[must_use]
pub fn present_list(_args: &Value) -> PresentCall {
    PresentCall {
        card: "generic".into(),
        title: "List terminal sessions".into(),
        kind: Some("read".into()),
        description: None,
    }
}

pub(crate) fn spawn_from_json(value: &Value) -> RenderedSpawnResult {
    RenderedSpawnResult::new(
        value
            .get("sessionId")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        value
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string),
        value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        value
            .get("motd")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    )
}

pub(crate) fn send_from_json(value: &Value) -> RenderedSendResult {
    RenderedSendResult::new(
        value
            .get("viewport")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        value
            .get("waitReason")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        status_from_json(value.get("sessionStatus")),
        value
            .get("truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    )
}

pub(crate) fn read_from_json(value: &Value) -> RenderedReadResult {
    RenderedReadResult::new(
        value
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        value.get("totalLines").and_then(Value::as_u64).unwrap_or(0) as usize,
        value.get("lineBegin").and_then(Value::as_u64).unwrap_or(0) as usize,
        value.get("lineEnd").and_then(Value::as_u64).unwrap_or(0) as usize,
        value
            .get("truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    )
}

pub(crate) fn list_from_json(value: &Value) -> Vec<RenderedSessionSnapshot> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    RenderedSessionSnapshot::new(
                        item.get("sessionId")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                        item.get("name").and_then(Value::as_str).map(str::to_string),
                        item.get("type").and_then(Value::as_str).unwrap_or_default(),
                        item.get("pid")
                            .and_then(Value::as_i64)
                            .and_then(|pid| i32::try_from(pid).ok()),
                        status_from_json(item.get("status")),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn status_from_json(value: Option<&Value>) -> RenderedSessionStatus {
    let Some(value) = value else {
        return RenderedSessionStatus::Running;
    };
    if value.get("kind").and_then(Value::as_str) != Some("exited") {
        return RenderedSessionStatus::Running;
    }
    let exit_code = match value.get("exitCode") {
        Some(Value::Null) | None => None,
        Some(number) => number.as_i64().and_then(|code| i32::try_from(code).ok()),
    };
    let signal = match value.get("signal") {
        Some(Value::Null) | None => None,
        Some(Value::String(name)) => Some(name.clone()),
        Some(_) => None,
    };
    RenderedSessionStatus::Exited { exit_code, signal }
}

#[cfg(test)]
mod tests {
    use super::{
        RenderedReadResult, RenderedSendRead, RenderedSendResult, RenderedSessionSnapshot,
        RenderedSessionStatus, RenderedSpawnResult, bound_terminal_text, present_close,
        present_list, present_open, present_read, present_send, present_signal, render_list,
        render_read, render_send, render_send_read, render_spawn,
    };
    use serde_json::json;

    #[test]
    fn render_spawn_matches_typescript() {
        let unnamed = render_spawn(&RenderedSpawnResult::new("pty-1", None, "shell", ""), 1024);
        assert_eq!(
            unnamed,
            "started terminal session pty-1 [type: shell]\n(no startup output)"
        );
        let named = render_spawn(
            &RenderedSpawnResult::new("pty-2", Some("main".into()), "shell", "ready"),
            1024,
        );
        assert!(named.contains("pty-2 (main)"));
    }

    #[test]
    fn render_send_matches_typescript_vectors() {
        assert_eq!(
            render_send(
                &RenderedSendResult::new("", "timeout", RenderedSessionStatus::Running, true,),
                1024,
            ),
            "(no new output)\n[wait: timeout]\n[session: running]\n[output truncated]"
        );
        assert!(
            render_send(
                &RenderedSendResult::new(
                    "bye",
                    "session_exit",
                    RenderedSessionStatus::Exited {
                        exit_code: None,
                        signal: Some("SIGTERM".into()),
                    },
                    false,
                ),
                1024,
            )
            .contains("exited code=null signal=SIGTERM")
        );
        assert!(
            render_send(
                &RenderedSendResult::new(
                    "bye",
                    "session_exit",
                    RenderedSessionStatus::Exited {
                        exit_code: Some(2),
                        signal: None,
                    },
                    false,
                ),
                1024,
            )
            .contains("exited code=2 signal=null")
        );
        assert!(
            render_send(
                &RenderedSendResult::new(
                    "bye",
                    "session_exit",
                    RenderedSessionStatus::Exited {
                        exit_code: None,
                        signal: None,
                    },
                    false,
                ),
                1024,
            )
            .contains("exited code=null signal=null")
        );
        assert_eq!(
            render_send_read(&RenderedSendRead::new("", true)),
            "[output truncated]"
        );
        assert_eq!(
            render_send_read(&RenderedSendRead::new("x", true)),
            "x\n[output truncated]"
        );
        assert_eq!(
            render_send_read(&RenderedSendRead::new("x\n", true)),
            "x\n[output truncated]"
        );
        assert_eq!(render_send_read(&RenderedSendRead::new("x", false)), "x");
    }

    #[test]
    fn render_read_and_list_match_typescript() {
        assert_eq!(
            render_read(&RenderedReadResult::new("", 0, 0, 0, true), 1024),
            "(no retained output)\n[lines: 0-0 of 0]\n[output truncated]"
        );
        assert_eq!(render_list(&[], 1024), "(no terminal sessions)");
        assert_eq!(
            render_list(
                &[
                    RenderedSessionSnapshot::new(
                        "pty-1",
                        None,
                        "shell",
                        None,
                        RenderedSessionStatus::Running,
                    ),
                    RenderedSessionSnapshot::new(
                        "pty-2",
                        Some("done".into()),
                        "shell",
                        Some(9),
                        RenderedSessionStatus::Exited {
                            exit_code: Some(2),
                            signal: None,
                        },
                    ),
                    RenderedSessionSnapshot::new(
                        "pty-3",
                        None,
                        "shell",
                        None,
                        RenderedSessionStatus::Exited {
                            exit_code: None,
                            signal: Some("SIGTERM".into()),
                        },
                    ),
                    RenderedSessionSnapshot::new(
                        "pty-4",
                        None,
                        "shell",
                        None,
                        RenderedSessionStatus::Exited {
                            exit_code: None,
                            signal: None,
                        },
                    ),
                ],
                1024,
            ),
            "pty-1 [shell] running\npty-2 (done) [shell] exited code=2 signal=null pid=9\npty-3 [shell] exited code=null signal=SIGTERM\npty-4 [shell] exited code=null signal=null"
        );
    }

    #[test]
    fn bounds_complete_utf8_results_while_retaining_metadata() {
        let send = render_send(
            &RenderedSendResult::new(
                format!("prefix-{}", "界".repeat(40)),
                "stdin_read",
                RenderedSessionStatus::Running,
                false,
            ),
            64,
        );
        assert!(send.len() <= 64);
        assert!(send.contains("[wait: stdin_read]"));
        assert!(send.contains("[output truncated]"));

        let read = render_read(
            &RenderedReadResult::new("x".repeat(200), 20, 0, 10, false),
            48,
        );
        assert!(read.len() <= 48);
        assert!(read.contains("[lines: 0-10 of 20]"));

        assert!(
            render_spawn(
                &RenderedSpawnResult::new("pty-1", None, "shell", "x".repeat(200)),
                32,
            )
            .len()
                <= 32
        );

        let bounded_spawn = render_spawn(
            &RenderedSpawnResult::new("pty-1", None, "shell", "x".repeat(200)),
            96,
        );
        assert!(bounded_spawn.contains("started terminal session pty-1"));
        assert!(bounded_spawn.contains("[output truncated]"));

        assert!(
            render_send(
                &RenderedSendResult::new(
                    "x".repeat(200),
                    "stdin_read",
                    RenderedSessionStatus::Running,
                    false,
                ),
                8,
            )
            .len()
                <= 8
        );
        assert_eq!(bound_terminal_text(&"x".repeat(200), 8).len(), 8);
        assert!(bound_terminal_text(&"x".repeat(200), 32).ends_with("[output truncated]"));
    }

    #[test]
    fn present_intents_match_typescript_table() {
        let open_type = present_open(&json!({ "type": "stub" }));
        assert_eq!(open_type.card(), "generic");
        assert_eq!(open_type.kind(), Some("execute"));
        assert_eq!(open_type.title(), "Open terminal stub");
        assert_eq!(
            present_open(&json!({ "type": "stub", "name": "main" })).title(),
            "Open terminal main"
        );

        let foreground = present_send(&json!({ "sessionId": "pty-1", "text": "python3" }));
        assert_eq!(foreground.card(), "terminal");
        assert_eq!(foreground.title(), "python3");
        assert_eq!(foreground.description(), Some("Terminal pty-1"));
        assert!(foreground.kind().is_none());
        assert_eq!(
            present_send(&json!({ "sessionId": "pty-1", "text": "" })).title(),
            "(send input)"
        );
        let background = present_send(&json!({
            "sessionId": "pty-1",
            "text": "make",
            "run_in_background": true,
        }));
        assert_eq!(background.card(), "generic");
        assert_eq!(background.kind(), Some("execute"));
        assert_eq!(background.title(), "Send to terminal pty-1 in background");

        let read = present_read(&json!({ "sessionId": "pty-1" }));
        assert_eq!(read.card(), "generic");
        assert_eq!(read.kind(), Some("read"));
        assert_eq!(read.title(), "Read terminal pty-1");

        let signal = present_signal(&json!({ "sessionId": "pty-1", "signal": "SIGINT" }));
        assert_eq!(signal.card(), "generic");
        assert_eq!(signal.kind(), Some("execute"));
        assert_eq!(signal.title(), "Signal terminal pty-1");

        let close = present_close(&json!({ "sessionId": "pty-1" }));
        assert_eq!(close.card(), "generic");
        assert_eq!(close.kind(), Some("delete"));
        assert_eq!(close.title(), "Close terminal pty-1");

        let list = present_list(&json!({}));
        assert_eq!(list.card(), "generic");
        assert_eq!(list.kind(), Some("read"));
        assert_eq!(list.title(), "List terminal sessions");
    }
}
