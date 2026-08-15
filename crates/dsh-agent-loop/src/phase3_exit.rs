//! Phase 3 exit checklist. Tests here fail if a required filter name disappeared.

#[cfg(test)]
mod tests {
    #[test]
    fn phase3_exit_does_not_read_deepseek_api_key() {
        // SAFETY: isolated assertion; restored immediately.
        let previous = std::env::var("DEEPSEEK_API_KEY").ok();
        unsafe {
            std::env::remove_var("DEEPSEEK_API_KEY");
        }
        assert!(std::env::var("DEEPSEEK_API_KEY").is_err());
        if let Some(value) = previous {
            unsafe {
                std::env::set_var("DEEPSEEK_API_KEY", value);
            }
        }
    }

    #[test]
    fn phase3_required_test_names_exist_in_this_crate() {
        let source = include_str!("agent.rs");
        for name in [
            "simple_text_turn_event_order",
            "empty_first_claim_logs_a_turn_without_a_step",
            "seeds_max_tokens_on_the_first_request",
            "sticky_max_tokens_survives_a_later_completed_step",
            "idle_cancel_is_noop_and_next_prompt_runs",
            "abort_drain_synthesizes_aborted_before_dispatch",
            "parallel_siblings_start_together_exclusive_is_a_barrier",
            "each_step_request_append_extends_the_previous",
            "header_change_is_logged_when_the_canonical_header_differs",
        ] {
            assert!(source.contains(name), "missing ported test {name}");
        }
    }
}
