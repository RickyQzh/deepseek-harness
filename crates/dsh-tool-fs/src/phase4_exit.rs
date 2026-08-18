//! Phase 4 exit checklist for dsh-tool-fs.

#[cfg(test)]
mod tests {
    #[test]
    fn phase4_required_test_names_exist_in_this_crate() {
        let lib = include_str!("lib.rs");
        let search = include_str!("search.rs");
        for name in [
            "edit_without_read_is_fs_not_observed",
            "no_config_is_the_first_argument_after_the_binary",
        ] {
            assert!(
                lib.contains(name) || search.contains(name),
                "missing ported test {name}"
            );
        }
    }
}
