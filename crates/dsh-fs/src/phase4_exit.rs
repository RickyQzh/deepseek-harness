//! Phase 4 exit checklist for dsh-fs.

#[cfg(test)]
mod tests {
    #[test]
    fn phase4_required_test_names_exist_in_this_crate() {
        let observation = include_str!("observation.rs");
        let name = "edit_without_observation_is_not_observed";
        assert!(observation.contains(name), "missing ported test {name}");
    }
}
