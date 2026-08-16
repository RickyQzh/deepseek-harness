//! Phase 6 exit checklist for dsh-session-persist.

#[cfg(test)]
mod tests {
    #[test]
    fn phase6_required_test_names_exist_in_this_crate() {
        let src = include_str!("store.rs");
        for name in ["load_round_trips_uncompressed_jsonl"] {
            assert!(src.contains(name), "missing ported test {name}");
        }
    }
}
