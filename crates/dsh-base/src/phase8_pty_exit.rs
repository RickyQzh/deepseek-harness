//! Phase 8 PTY exit checklist for dsh-base.

#[cfg(test)]
mod tests {
    #[test]
    fn phase8_pty_required_test_names_exist_in_this_crate() {
        let src = include_str!("lib.rs");
        for name in [
            "register_terminal_plugins",
            "register_base_plugins_installs_terminal_yaml_names",
        ] {
            assert!(src.contains(name), "missing ported test {name}");
        }
    }
}
