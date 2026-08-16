//! Phase 5 exit checklist for dsh-boot.

#[cfg(test)]
mod tests {
    #[test]
    fn phase5_required_test_names_exist_in_this_crate() {
        let mount = include_str!("mount.rs");
        for name in ["unknown_name_fails_loud", "js_tag_is_still_rejected"] {
            assert!(mount.contains(name), "missing ported test {name}");
        }
    }
}
