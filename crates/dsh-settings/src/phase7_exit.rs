//! Phase 7 exit checklist for dsh-settings.

#[cfg(test)]
mod tests {
    #[test]
    fn phase7_required_test_names_exist_in_this_crate() {
        let src = include_str!("lib.rs");
        let name = "stale_expected_revision_is_settings_conflict";
        assert!(src.contains(name), "missing ported test {name}");
    }
}
