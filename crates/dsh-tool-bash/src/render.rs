//! Model-facing bash result rendering: stdout/stderr body plus exit-status markers.

use dsh_sandbox::{SandboxMode, escalation_hint_marker, sandbox_denial_marker};
use dsh_shell::ShellRunResult;

/// Render one finished run as the text the model sees.
///
/// Stdout is first. Nonempty stderr is appended as `[stderr]\n{stderr}`, with a
/// newline between sections when stdout does not already end with one. Both
/// streams empty becomes `(no output)`. A truncated stream appends
/// `\n[output truncated; full output: {spill_path|'(unavailable)'}]`.
///
/// Markers then follow, each on its own line, in this order: sandbox denial
/// (and the shared escalation hint when `escalation_modes` is nonempty),
/// timeout, killed-by-signal, then nonzero `[exit code: N]`. Exit `0` and a
/// missing exit code without a signal emit no exit marker.
///
/// # Parameters
///
/// * `result` — completed foreground run from a bash executor.
/// * `escalation_modes` — advertised escalation targets; nonempty adds the
///   same-turn hint after a denial marker.
///
/// # Returns
///
/// The model-visible body plus any markers, joined so a trailing exit marker
/// is the last line (`oops\n[exit code: 2]` for a nonzero exit with stdout).
#[must_use]
pub fn render_result(result: &ShellRunResult, escalation_modes: &[SandboxMode]) -> String {
    let out = stream_text(
        &result.stdout.text,
        result.stdout.truncated,
        result.stdout.spill_path.as_deref(),
    );
    let err = stream_text(
        &result.stderr.text,
        result.stderr.truncated,
        result.stderr.spill_path.as_deref(),
    );

    let mut body = out;
    if !err.is_empty() {
        if !body.is_empty() && !body.ends_with('\n') {
            body.push('\n');
        }
        body.push_str("[stderr]\n");
        body.push_str(&err);
    }
    if body.is_empty() {
        body = "(no output)".into();
    }

    let mut markers = Vec::new();
    if let Some(sandbox) = result.sandbox.as_ref() {
        if sandbox.denied {
            markers.push(sandbox_denial_marker(sandbox.mode));
            if !escalation_modes.is_empty() {
                markers.push(escalation_hint_marker("command"));
            }
        }
    }
    if result.timed_out {
        markers.push(format!("[timed out after {}ms]", result.timeout_ms));
    }
    if let Some(signal) = result.signal {
        markers.push(format!("[killed by signal: {}]", signal_name(signal)));
    } else if let Some(code) = result.exit_code {
        if code != 0 {
            markers.push(format!("[exit code: {code}]"));
        }
    }
    if markers.is_empty() {
        return body;
    }
    if !body.ends_with('\n') {
        body.push('\n');
    }
    body.push_str(&markers.join("\n"));
    body
}

fn stream_text(text: &str, truncated: bool, spill_path: Option<&str>) -> String {
    if !truncated {
        return text.to_owned();
    }
    let path = spill_path.unwrap_or("(unavailable)");
    format!("{text}\n[output truncated; full output: {path}]")
}

fn signal_name(number: i32) -> String {
    match number {
        libc::SIGTERM => "SIGTERM".into(),
        libc::SIGKILL => "SIGKILL".into(),
        libc::SIGINT => "SIGINT".into(),
        libc::SIGHUP => "SIGHUP".into(),
        libc::SIGQUIT => "SIGQUIT".into(),
        libc::SIGPIPE => "SIGPIPE".into(),
        libc::SIGABRT => "SIGABRT".into(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::render_result;
    use dsh_sandbox::SandboxMode;
    use dsh_shell::{ShellRunResult, ShellSandboxInfo};
    use dsh_subprocess::CollectedOutput;

    fn collected(text: &str) -> CollectedOutput {
        CollectedOutput {
            text: text.into(),
            truncated: false,
            spill_path: None,
        }
    }

    #[test]
    fn nonzero_exit_appends_marker() {
        let result = ShellRunResult {
            exit_code: Some(2),
            signal: None,
            timed_out: false,
            aborted: false,
            timeout_ms: 1000,
            stdout: collected("oops"),
            stderr: collected(""),
            sandbox: None,
        };
        assert_eq!(render_result(&result, &[]), "oops\n[exit code: 2]");
    }

    #[test]
    fn sandbox_denial_uses_shared_marker() {
        let result = ShellRunResult {
            exit_code: Some(1),
            signal: None,
            timed_out: false,
            aborted: false,
            timeout_ms: 1000,
            stdout: collected(""),
            stderr: collected("permission denied"),
            sandbox: Some(ShellSandboxInfo {
                mode: SandboxMode::ReadOnly,
                denied: true,
                enforcement: None,
                runner_failed: None,
            }),
        };
        let text = render_result(&result, &[SandboxMode::WorkspaceWrite]);
        assert!(text.contains("[sandbox: file access denied under read-only mode]"));
        assert!(text.contains("escalation available"));
        assert!(text.contains("[exit code: 1]"));
    }
}
