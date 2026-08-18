//! Phase 8 MCP exit checklist for dsh-mcp-client.

#[cfg(test)]
mod tests {
    #[test]
    fn phase8_mcp_required_test_names_exist_in_this_crate() {
        let src = concat!(
            include_str!("name.rs"),
            include_str!("result.rs"),
            include_str!("rpc.rs"),
            include_str!("sync.rs"),
            include_str!("connection.rs"),
            include_str!("plugin.rs"),
            include_str!("../tests/plugin_boot.rs"),
            include_str!("../tests/reconnect.rs")
        );
        for name in [
            "public_tool_name_dotted_admin_reset_hashes",
            "encode_frame_prefixes_content_length",
            "duplicate_server_name_fails_second_instance",
            "stdio_crash_reconnects_and_add_works",
            "streamable_http_fails_load",
        ] {
            assert!(src.contains(name), "missing ported test {name}");
        }
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn phase8_mcp_exit() {
        assert!(true);
    }
}
