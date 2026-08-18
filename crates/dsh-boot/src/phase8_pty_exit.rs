//! Phase 8 PTY exit checklist for dsh-boot.

#[cfg(test)]
mod tests {
    #[test]
    fn phase8_pty_required_test_names_exist_in_this_crate() {
        let src = include_str!("lib.rs");
        for name in [
            "PLUGIN_TERMINAL",
            "@deepseek-ai/dsh-terminal",
            "PLUGIN_PTY_SNAPSHOT_BACKEND",
            "pty-snapshot-backend",
        ] {
            assert!(src.contains(name), "missing ported test {name}");
        }
    }
}
