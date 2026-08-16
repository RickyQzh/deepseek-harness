//! Phase 5 exit checklist for dsh-agent.

#[cfg(test)]
mod tests {
    #[test]
    fn phase5_required_test_names_exist_in_this_crate() {
        let registry = include_str!("registry.rs");
        let name = "create_followup_run_until_idle_text_only";
        assert!(registry.contains(name), "missing ported test {name}");
    }
}
