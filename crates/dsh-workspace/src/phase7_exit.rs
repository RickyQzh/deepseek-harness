//! Phase 7 exit checklist for dsh-workspace.

#[cfg(test)]
mod tests {
    #[test]
    fn phase7_required_test_names_exist_in_this_crate() {
        let src = include_str!("registry.rs");
        for name in [
            "create_is_idempotent_on_realpath",
            "archive_keeps_session_in_account",
        ] {
            assert!(src.contains(name), "missing ported test {name}");
        }
    }
}
