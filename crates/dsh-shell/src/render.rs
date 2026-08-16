//! Inverse of the shell-tool `[exit code: N]` / `[killed by signal: NAME]` markers.

/// Exit status recovered from a rendered shell-tool result, plus the output body that status was split off from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedExitStatus {
    /// Rendered output with the consumed exit or kill marker removed, when one matched.
    pub body: String,
    /// Recovered exit code. `None` when a kill marker matched.
    pub exit_code: Option<i32>,
    /// Terminating signal name from a kill marker. `None` on a normal exit.
    pub signal: Option<String>,
}

const SIGNAL_MARKER_PREFIX: &str = "\n[killed by signal: ";
const EXIT_MARKER_PREFIX: &str = "\n[exit code: ";

/// Split a rendered shell-tool result into its output body and structured exit status.
///
/// If `text` ends with `\n[killed by signal: NAME]`, strips that marker, sets [`ParsedExitStatus::signal`] to `NAME`, and leaves [`ParsedExitStatus::exit_code`] as `None`. Else if it ends with `\n[exit code: N]`, strips that marker, sets `exit_code` to the parsed `i32`, and leaves `signal` as `None`. Otherwise returns `{ body: text unchanged, exit_code: Some(0), signal: None }`.
///
/// `NAME` is the rest of the marker line after the prefix. `N` is a parsed `i32` of ASCII digits. Timeout and sandbox-denial markers stay in `body`.
#[must_use]
pub fn parse_exit_status(text: &str) -> ParsedExitStatus {
    if let Some((body, name)) =
        trailing_marker(text, SIGNAL_MARKER_PREFIX).filter(|(_, name)| is_signal_name(name))
    {
        return ParsedExitStatus {
            body: body.to_owned(),
            exit_code: None,
            signal: Some(name.to_owned()),
        };
    }
    if let Some((body, exit_code)) = trailing_marker(text, EXIT_MARKER_PREFIX)
        .and_then(|(body, digits)| parse_exit_digits(digits).map(|code| (body, code)))
    {
        return ParsedExitStatus {
            body: body.to_owned(),
            exit_code: Some(exit_code),
            signal: None,
        };
    }
    ParsedExitStatus {
        body: text.to_owned(),
        exit_code: Some(0),
        signal: None,
    }
}

fn trailing_marker<'a>(text: &'a str, prefix: &str) -> Option<(&'a str, &'a str)> {
    let stripped = text.strip_suffix(']')?;
    let newline = stripped.rfind('\n')?;
    let payload = stripped[newline..].strip_prefix(prefix)?;
    Some((&text[..newline], payload))
}

fn is_signal_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(']') && !name.contains('\n')
}

fn parse_exit_digits(digits: &str) -> Option<i32> {
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::parse_exit_status;

    #[test]
    fn strips_exit_code_marker() {
        let parsed = parse_exit_status("hello\n[exit code: 2]");
        assert_eq!(parsed.body, "hello");
        assert_eq!(parsed.exit_code, Some(2));
        assert!(parsed.signal.is_none());
    }

    #[test]
    fn strips_signal_marker() {
        let parsed = parse_exit_status("x\n[killed by signal: SIGTERM]");
        assert_eq!(parsed.body, "x");
        assert_eq!(parsed.signal.as_deref(), Some("SIGTERM"));
        assert!(parsed.exit_code.is_none());
    }

    #[test]
    fn clean_exit_has_no_marker() {
        let parsed = parse_exit_status("ok\n");
        assert_eq!(parsed.body, "ok\n");
        assert_eq!(parsed.exit_code, Some(0));
    }

    #[test]
    fn marker_without_leading_newline_stays_in_body() {
        let exit = parse_exit_status("[exit code: 5]");
        assert_eq!(exit.body, "[exit code: 5]");
        assert_eq!(exit.exit_code, Some(0));
        let killed = parse_exit_status("[killed by signal: SIGKILL]");
        assert_eq!(killed.body, "[killed by signal: SIGKILL]");
        assert_eq!(killed.exit_code, Some(0));
        assert!(killed.signal.is_none());
    }

    #[test]
    fn keeps_timeout_marker_in_body() {
        let parsed = parse_exit_status("slow\n[timed out after 100ms]\n[exit code: 143]");
        assert_eq!(parsed.body, "slow\n[timed out after 100ms]");
        assert_eq!(parsed.exit_code, Some(143));
    }
}
