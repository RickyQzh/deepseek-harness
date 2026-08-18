//! Phase 4 exit checklist for dsh-shell.

#[cfg(test)]
mod tests {
    #[test]
    fn phase4_required_test_names_exist_in_this_crate() {
        let classify = include_str!("classify.rs");
        let bash_sandbox = include_str!("bash_sandbox.rs");
        for name in [
            "landlock_125_with_fatal_line_is_launcher_failure",
            "landlock_125_fatal_line_becomes_sandbox_unavailable",
            "unavailable_confine_does_not_run_the_command",
        ] {
            assert!(
                classify.contains(name) || bash_sandbox.contains(name),
                "missing ported test {name}"
            );
        }
    }
}
