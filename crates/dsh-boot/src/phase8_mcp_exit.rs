//! Phase 8 MCP exit checklist for dsh-boot.

#[cfg(test)]
mod tests {
    #[test]
    fn phase8_mcp_required_test_names_exist_in_this_crate() {
        let src = include_str!("lib.rs");
        assert!(src.contains("PLUGIN_MCP_CLIENT"));
        assert!(src.contains("@deepseek-ai/dsh-mcp-client"));
    }
}
