//! Phase 4 exit checklist for dsh-tool-bash.

#[cfg(test)]
mod tests {
    #[test]
    fn phase4_required_test_names_exist_in_this_crate() {
        let lib = include_str!("lib.rs");
        for name in [
            "loop_smoke_runs_real_bash_echo",
            "loop_smoke_fs_edit_without_read_is_not_observed",
        ] {
            assert!(lib.contains(name), "missing ported test {name}");
        }
    }
}
