//! Phase 5 exit checklist for dsh-sdk-protocol.

#[cfg(test)]
mod tests {
    #[test]
    fn phase5_required_test_names_exist_in_this_crate() {
        let types = include_str!("types.rs");
        let transport = include_str!("transport.rs");
        for name in [
            "server_info_name_is_the_wire_stable_runtime_id",
            "encode_error_uses_minus_32601_and_minus_32603",
            "serve_ignores_malformed_lines_and_answers_unknown_method",
        ] {
            assert!(
                types.contains(name) || transport.contains(name),
                "missing ported test {name}"
            );
        }
    }
}
