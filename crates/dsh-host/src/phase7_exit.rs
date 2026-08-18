//! Phase 7 exit checklist for dsh-host.

#[cfg(test)]
mod tests {
    #[test]
    fn phase7_required_test_names_exist_in_this_crate() {
        let src = concat!(
            include_str!("trust.rs"),
            include_str!("static_files.rs"),
            include_str!("boot.rs"),
            include_str!("server.rs")
        );
        for name in [
            "loopback_accepts_localhost_v6_and_127_8",
            "privileged_set_matches_typescript",
            "traversal_is_forbidden",
            "boot_escapes_left_angle_in_json",
            "get_events_mux_is_426",
            "host_describe_returns_server_response_200",
        ] {
            assert!(src.contains(name), "missing ported test {name}");
        }
    }
}
