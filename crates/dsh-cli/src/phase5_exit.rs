//! Phase 5 exit checklist for dsh-cli.

#[cfg(test)]
mod tests {
    #[test]
    fn phase5_required_test_names_exist_in_this_crate() {
        let parse = include_str!("parse.rs");
        for name in [
            "parse_profile_headless_joins_positional_task",
            "web_as_argv1_parses_web_launch",
            "profile_web_parses_web_launch",
            "web_host_all_interfaces_is_usage",
            "headless_requires_positional_task",
        ] {
            assert!(parse.contains(name), "missing ported test {name}");
        }
    }
}
