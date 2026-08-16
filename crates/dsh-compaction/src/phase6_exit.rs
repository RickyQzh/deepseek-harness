//! Phase 6 exit checklist for dsh-compaction.

#[cfg(test)]
mod tests {
    #[test]
    fn phase6_required_test_names_exist_in_this_crate() {
        let src = include_str!("checkpoint.rs");
        for name in ["compact_checkpoint_source_is_plugin_compact"] {
            assert!(src.contains(name), "missing ported test {name}");
        }
    }
}
