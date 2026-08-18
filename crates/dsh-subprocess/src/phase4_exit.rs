//! Phase 4 exit checklist for dsh-subprocess.

#[cfg(test)]
mod tests {
    #[test]
    fn phase4_required_test_names_exist_in_this_crate() {
        let spawn = include_str!("spawn.rs");
        let env = include_str!("env.rs");
        for name in [
            "argv_is_not_a_shell_string",
            "scrubbed_parent_env_drops_credentials_and_dsh_names",
        ] {
            assert!(
                spawn.contains(name) || env.contains(name),
                "missing ported test {name}"
            );
        }
    }
}
