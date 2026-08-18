//! Phase 6 exit checklist for dsh-subagent.

#[cfg(test)]
mod tests {
    #[test]
    fn phase6_required_test_names_exist_in_this_crate() {
        let src = concat!(include_str!("continuation.rs"), include_str!("runtime.rs"));
        for name in [
            "continuable_settlement_injects_subagent_settled_notice",
            "spawn_child_does_not_see_parent_history",
        ] {
            assert!(src.contains(name), "missing ported test {name}");
        }
    }
}
