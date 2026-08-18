//! Phase 7 exit checklist for dsh-cli.

#[cfg(test)]
mod tests {
    #[test]
    fn phase7_required_test_names_exist_in_this_crate() {
        let src = include_str!("parse.rs");
        for name in [
            "web_as_argv1_parses_web_launch",
            "profile_web_parses_web_launch",
            "web_host_all_interfaces_is_usage",
        ] {
            assert!(src.contains(name), "missing ported test {name}");
        }
    }
}
