//! Phase 6 exit checklist for dsh-web.

#[cfg(test)]
mod tests {
    #[test]
    fn phase6_required_test_names_exist_in_this_crate() {
        let src = include_str!("runtime.rs");
        let name = "search_caps_sources_and_sets_truncated";
        assert!(src.contains(name), "missing ported test {name}");
    }
}
