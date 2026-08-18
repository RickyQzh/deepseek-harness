//! Phase 8 MCP exit checklist for dsh-base.

#[cfg(test)]
mod tests {
    #[test]
    fn phase8_mcp_required_test_names_exist_in_this_crate() {
        let src = include_str!("lib.rs");
        let name = "register_mcp_plugins";
        assert!(src.contains(name), "missing ported test {name}");
    }
}
