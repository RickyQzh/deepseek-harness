//! Phase 6 exit checklist for dsh-agent-instructions.

#[cfg(test)]
mod tests {
    #[test]
    fn phase6_required_test_names_exist_in_this_crate() {
        let src = include_str!("plugin.rs");
        let name = "baseline_injects_agents_md_before_first_request";
        assert!(src.contains(name), "missing ported test {name}");
    }
}
