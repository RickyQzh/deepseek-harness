//! Phase 8 PTY exit checklist for dsh-permission-presets.

#[cfg(test)]
mod tests {
    #[test]
    fn phase8_pty_required_test_names_exist_in_this_crate() {
        let src = include_str!("service.rs");
        for name in [
            "set_sandbox_mode_with_terminals",
            "sandbox_mode_change_rejected_while_pty_open",
        ] {
            assert!(src.contains(name), "missing ported test {name}");
        }
    }
}
