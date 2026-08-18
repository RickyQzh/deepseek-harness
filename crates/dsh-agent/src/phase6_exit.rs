//! Phase 6 exit checklist for dsh-agent.

#[cfg(test)]
mod tests {
    #[test]
    fn phase6_required_test_names_exist_in_this_crate() {
        let src = include_str!("registry.rs");
        let name = "followup_during_tool_await_does_not_need_driver_permit";
        assert!(src.contains(name), "missing ported test {name}");
    }
}
