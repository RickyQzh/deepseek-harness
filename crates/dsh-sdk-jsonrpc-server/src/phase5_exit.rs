//! Phase 5 exit checklist for dsh-sdk-jsonrpc-server.

#[cfg(test)]
mod tests {
    #[test]
    fn phase5_required_test_names_exist_in_this_crate() {
        let server = include_str!("server.rs");
        for name in [
            "initialize_server_info_name_is_stable",
            "prompt_returns_message_id_before_turn_ends",
        ] {
            assert!(server.contains(name), "missing ported test {name}");
        }
    }
}
