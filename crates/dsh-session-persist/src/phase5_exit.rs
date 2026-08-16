//! Phase 5 exit checklist for dsh-session-persist.

#[cfg(test)]
mod tests {
    #[test]
    fn phase5_required_test_names_exist_in_this_crate() {
        let store = include_str!("store.rs");
        let name = "flush_writes_uncompressed_jsonl_under_session_id_dir";
        assert!(store.contains(name), "missing ported test {name}");
    }
}
