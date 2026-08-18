//! Phase 8 exit checklist for dsh-cli.

#[cfg(test)]
mod tests {
    #[test]
    fn phase8_required_test_names_exist_in_this_crate() {
        let src = include_str!("parse.rs");
        for name in [
            "profile_acp_parses_acp_launch",
            "acp_as_argv1_parses_acp_launch",
            "unknown_profile_is_not_implemented_exit_2",
        ] {
            assert!(src.contains(name), "missing ported test {name}");
        }
    }
}
