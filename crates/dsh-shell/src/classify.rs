//! Sandbox stderr classification: denial dialect and runner-failure evidence.

use dsh_sandbox::RunnerFailureRule;

/// Fatal runner evidence retained for an infrastructure-error detail.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunnerFailureMatch {
    /// The original stderr line that matched a fatal signature.
    pub detail: String,
}

/// Match a nonzero exit against case-insensitive stderr signatures.
///
/// Returns `false` when `exit_code` is [`None`] or `0`. Otherwise true when any signature is a
/// case-insensitive substring of `stderr`.
#[must_use]
pub fn matches_signature(exit_code: Option<i32>, stderr: &str, signatures: &[String]) -> bool {
    match exit_code {
        None | Some(0) => false,
        Some(_) => {
            let lowered = stderr.to_lowercase();
            signatures
                .iter()
                .any(|signature| lowered.contains(&signature.to_lowercase()))
        }
    }
}

/// Classify a failed run against the selected backend's denial dialect.
///
/// Same as [`matches_signature`] with that backend's denial signatures.
#[must_use]
pub fn classify_denial(exit_code: Option<i32>, stderr: &str, signatures: &[String]) -> bool {
    matches_signature(exit_code, stderr, signatures)
}

/// Classify one settled process against structured runner-failure rules.
///
/// Returns [`None`] when `exit_code` is [`None`] or `0`. A rule whose `allowed_exit_codes` is
/// [`Some`] and does not contain `exit_code` is skipped. Empty or whitespace-only fatal
/// signatures are ignored. A stderr line whose whole lowercased text equals an informational
/// line is skipped. The first remaining line that contains a fatal signature (case-insensitive
/// substring) yields that original line as [`RunnerFailureMatch::detail`].
///
/// Landlock launcher failure is therefore exit 125 plus a fatal `landlock-run: ` line; 125
/// alone, or that exit with only the partial-enforcement notice, is not a runner failure.
#[must_use]
pub fn classify_runner_failure(
    exit_code: Option<i32>,
    stderr: &str,
    rules: &[RunnerFailureRule],
) -> Option<RunnerFailureMatch> {
    let exit_code = match exit_code {
        None | Some(0) => return None,
        Some(code) => code,
    };
    for rule in rules {
        if rule
            .allowed_exit_codes
            .as_ref()
            .is_some_and(|allowed| !allowed.contains(&exit_code))
        {
            continue;
        }
        let informational: std::collections::HashSet<String> = rule
            .informational_lines
            .iter()
            .map(|line| line.to_lowercase())
            .collect();
        let fatal_signatures: Vec<String> = rule
            .fatal_signatures
            .iter()
            .filter(|signature| !signature.trim().is_empty())
            .map(|signature| signature.to_lowercase())
            .collect();
        for line in stderr_lines(stderr) {
            let lowered = line.to_lowercase();
            if informational.contains(&lowered) {
                continue;
            }
            if fatal_signatures
                .iter()
                .any(|signature| lowered.contains(signature))
            {
                return Some(RunnerFailureMatch {
                    detail: line.to_owned(),
                });
            }
        }
    }
    None
}

fn stderr_lines(stderr: &str) -> impl Iterator<Item = &str> {
    stderr
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
}

#[cfg(test)]
mod tests {
    use super::classify_runner_failure;
    use dsh_sandbox::RunnerFailureRule;

    fn landlock_rule() -> RunnerFailureRule {
        RunnerFailureRule {
            allowed_exit_codes: Some(vec![125]),
            fatal_signatures: vec!["landlock-run: ".into()],
            informational_lines: vec![
                "landlock-run: partial enforcement (older Landlock ABI)".into(),
            ],
        }
    }

    #[test]
    fn landlock_125_with_fatal_line_is_launcher_failure() {
        let notice = "landlock-run: partial enforcement (older Landlock ABI)";
        let fatal = "landlock-run: exec failed: No such file or directory";
        let matched =
            classify_runner_failure(Some(125), &format!("{notice}\n{fatal}"), &[landlock_rule()]);
        assert_eq!(matched.unwrap().detail, fatal);
    }

    #[test]
    fn landlock_125_with_only_partial_notice_is_not_launcher_failure() {
        let notice = "landlock-run: partial enforcement (older Landlock ABI)";
        assert!(classify_runner_failure(Some(125), notice, &[landlock_rule()]).is_none());
    }

    #[test]
    fn landlock_exit_1_with_fatal_prefix_is_not_launcher_failure() {
        assert!(
            classify_runner_failure(
                Some(1),
                "landlock-run: exec failed: No such file or directory",
                &[landlock_rule()]
            )
            .is_none()
        );
    }

    #[test]
    fn exit_0_is_never_runner_failure() {
        assert!(
            classify_runner_failure(Some(0), "landlock-run: boom", &[landlock_rule()]).is_none()
        );
    }
}
