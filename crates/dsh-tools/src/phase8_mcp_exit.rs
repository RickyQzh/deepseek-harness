//! Phase 8 MCP exit checklist for dsh-tools.

#[cfg(test)]
mod tests {
    #[test]
    fn phase8_mcp_required_test_names_exist_in_this_crate() {
        let src = include_str!("pipeline.rs");
        for name in [
            "try_register_rejects_duplicate_name",
            "unregister_removes_name",
            "register_still_overwrites_duplicate_name",
        ] {
            assert!(src.contains(name), "missing ported test {name}");
        }
    }
}
