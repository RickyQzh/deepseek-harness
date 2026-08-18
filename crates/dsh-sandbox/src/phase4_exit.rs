//! Phase 4 exit checklist for dsh-sandbox.

#[cfg(test)]
mod tests {
    #[test]
    fn phase4_required_test_names_exist_in_this_crate() {
        let provider = include_str!("provider.rs");
        let landlock = include_str!("landlock.rs");
        for name in [
            "empty_chain_fails_closed",
            "both_linux_probes_unusable_fails_closed",
            "landlock_wrap_includes_separator_and_failure_rule",
            "launcher_path_ignores_environment_variables",
        ] {
            assert!(
                provider.contains(name) || landlock.contains(name),
                "missing ported test {name}"
            );
        }
    }
}
