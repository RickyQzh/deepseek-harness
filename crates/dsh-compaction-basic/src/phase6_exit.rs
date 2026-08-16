//! Phase 6 exit checklist for dsh-compaction-basic.

#[cfg(test)]
mod tests {
    #[test]
    fn phase6_required_test_names_exist_in_this_crate() {
        let src = include_str!("engine.rs");
        for name in [
            "compact_region_writes_lock_summary_replace_end",
            "overflow_retries_only_when_replace_generation_advances",
        ] {
            assert!(src.contains(name), "missing ported test {name}");
        }
    }
}
