//! Phase 8 PTY exit checklist for dsh-terminal-bash.

#[cfg(test)]
mod tests {
    #[test]
    fn phase8_pty_required_test_names_exist_in_this_crate() {
        let src = include_str!("session.rs");
        for name in ["stdin_read", "tier1_stdin_wait_is_stdin_read"] {
            assert!(src.contains(name), "missing ported test {name}");
        }
    }
}
