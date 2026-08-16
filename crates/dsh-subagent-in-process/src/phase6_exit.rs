//! Phase 6 exit checklist for dsh-subagent-in-process.

#[cfg(test)]
mod tests {
    #[test]
    fn phase6_required_test_names_exist_in_this_crate() {
        let src = include_str!("driver.rs");
        for name in ["spawn_child_does_not_see_parent_history"] {
            assert!(src.contains(name), "missing ported test {name}");
        }
    }
}
