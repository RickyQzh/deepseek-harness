//! Phase 8 PTY exit checklist for dsh-subprocess.

#[cfg(test)]
mod tests {
    #[test]
    fn phase8_pty_required_test_names_exist_in_this_crate() {
        let src = include_str!("terminal.rs");
        for name in ["spawn_terminal", "spawn_terminal_rejects_empty_argv"] {
            assert!(src.contains(name), "missing ported test {name}");
        }
    }
}
