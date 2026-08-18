//! Phase 8 exit checklist for dsh-user-approval.

#[cfg(test)]
mod tests {
    #[test]
    fn phase8_required_test_names_exist_in_this_crate() {
        let src = include_str!("service.rs");
        let name = "waterfall_question_carries_session_and_call_id";
        assert!(src.contains(name), "missing ported test {name}");
    }
}
