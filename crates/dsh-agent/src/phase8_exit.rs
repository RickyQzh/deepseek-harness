//! Phase 8 exit checklist for dsh-agent.

#[cfg(test)]
mod tests {
    #[test]
    fn phase8_required_test_names_exist_in_this_crate() {
        let src = include_str!("registry.rs");
        let name = "unregister_drops_live_handle";
        assert!(src.contains(name), "missing ported test {name}");
    }
}
