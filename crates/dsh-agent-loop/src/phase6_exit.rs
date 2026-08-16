//! Phase 6 exit checklist for dsh-agent-loop.

#[cfg(test)]
mod tests {
    #[test]
    fn phase6_required_test_names_exist_in_this_crate() {
        let src = include_str!("agent.rs");
        for name in ["pre_step_waterfall_must_call_next_to_keep_claimed_messages"] {
            assert!(src.contains(name), "missing ported test {name}");
        }
    }
}
