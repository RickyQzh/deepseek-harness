//! Phase 6 exit checklist for dsh-skill.

#[cfg(test)]
mod tests {
    #[test]
    fn phase6_required_test_names_exist_in_this_crate() {
        let src = include_str!("registry.rs");
        for name in ["list_picks_lower_rank_then_sorts_by_name"] {
            assert!(src.contains(name), "missing ported test {name}");
        }
    }
}
