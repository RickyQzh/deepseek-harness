//! Phase 7 exit checklist for dsh-rpc.

#[cfg(test)]
mod tests {
    #[test]
    fn phase7_required_test_names_exist_in_this_crate() {
        let src = include_str!("message.rs");
        for name in [
            "client_request_round_trips_camel_case",
            "receipt_is_not_an_rpc_message",
        ] {
            assert!(src.contains(name), "missing ported test {name}");
        }
    }
}
