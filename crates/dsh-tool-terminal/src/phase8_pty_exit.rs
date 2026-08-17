//! Phase 8 PTY exit checklist for dsh-tool-terminal.

#[cfg(test)]
mod tests {
    #[test]
    fn phase8_pty_required_test_names_exist_in_this_crate() {
        let src = include_str!("plugin.rs");
        for name in [
            "PLUGIN_TOOL_TERMINAL",
            "terminal_open",
            "plugin_yaml_name_is_typescript_package_name",
        ] {
            assert!(src.contains(name), "missing ported test {name}");
        }
    }
}
