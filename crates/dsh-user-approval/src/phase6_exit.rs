//! Phase 6 exit checklist for dsh-user-approval.

#[cfg(test)]
mod tests {
    #[test]
    fn phase6_required_test_names_exist_in_this_crate() {
        let src = include_str!("service.rs");
        for name in [
            "request_without_answerer_is_unavailable_and_logs_the_pair",
            "never_policy_rejects_before_answerer",
        ] {
            assert!(src.contains(name), "missing ported test {name}");
        }
    }
}
