//! Phase 5 exit checklist for dsh-agent.

#[cfg(test)]
mod tests {
    #[test]
    fn phase5_required_test_names_exist_in_this_crate() {
        let registry = include_str!("registry.rs");
        for name in [
            "create_followup_run_until_idle_text_only",
            "session_create_hook_runs_before_loop_agent_is_published",
            "followup_during_tool_await_does_not_need_driver_permit",
        ] {
            assert!(registry.contains(name), "missing ported test {name}");
        }
    }
}
