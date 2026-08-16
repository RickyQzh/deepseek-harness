//! Phase 6 exit checklist for dsh-permission-presets.

#[cfg(test)]
mod tests {
    #[test]
    fn phase6_required_test_names_exist_in_this_crate() {
        let src = include_str!("service.rs");
        let name = "pin_fresh_session_writes_three_knob_events";
        assert!(src.contains(name), "missing ported test {name}");
    }
}
