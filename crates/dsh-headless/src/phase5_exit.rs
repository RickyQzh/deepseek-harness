//! Phase 5 exit checklist for dsh-headless.

#[cfg(test)]
mod tests {
    #[test]
    fn phase5_required_test_names_exist_in_this_crate() {
        let runner = include_str!("runner.rs");
        let name = "mock_llm_prints_last_text_and_exits_0_on_completed";
        assert!(runner.contains(name), "missing ported test {name}");
    }
}
