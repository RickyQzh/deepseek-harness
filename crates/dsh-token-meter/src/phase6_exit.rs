//! Phase 6 exit checklist for dsh-token-meter.

#[cfg(test)]
mod tests {
    #[test]
    fn phase6_required_test_names_exist_in_this_crate() {
        let src = include_str!("estimate.rs");
        for name in ["estimate_text_matches_ts_heuristic"] {
            assert!(src.contains(name), "missing ported test {name}");
        }
    }
}
