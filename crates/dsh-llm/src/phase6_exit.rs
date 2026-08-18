//! Phase 6 exit checklist for dsh-llm.

#[cfg(test)]
mod tests {
    #[test]
    fn phase6_required_test_names_exist_in_this_crate() {
        let src = include_str!("retry.rs");
        let name = "crate_dsh_llm_retry_must_not_exist";
        assert!(src.contains(name), "missing ported test {name}");
    }
}
