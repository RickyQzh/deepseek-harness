//! Phase 8 exit checklist for dsh-acp.

#[cfg(test)]
mod tests {
    #[test]
    fn phase8_required_test_names_exist_in_this_crate() {
        let src = concat!(
            include_str!("error.rs"),
            include_str!("rpc.rs"),
            include_str!("codec.rs"),
            include_str!("bridge.rs"),
            include_str!("plugin.rs")
        );
        for name in [
            "method_not_found_message_matches_typescript",
            "invalid_params_message_matches_typescript",
            "turn_end_completed_is_end_turn",
            "prompt_stop_reason_max_tokens_is_end_turn",
            "initialize_advertises_automation_only_agent",
            "session_new_rejects_additional_directories",
            "prompt_emits_committed_text_and_end_turn",
            "cancel_settles_inflight_as_cancelled",
            "maps_allow_once_to_allowed_once",
            "acp_yaml_rejects_js_tag_substring",
        ] {
            assert!(src.contains(name), "missing ported test {name}");
        }
    }
}
