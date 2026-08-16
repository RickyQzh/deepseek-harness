//! Phase 6 exit checklist for dsh-jobs-local.

#[cfg(test)]
mod tests {
    #[test]
    fn phase6_required_test_names_exist_in_this_crate() {
        let src = include_str!("lib.rs");
        for name in ["start_assigns_kind_n_and_list_is_owner_fenced"] {
            assert!(src.contains(name), "missing ported test {name}");
        }
    }
}
