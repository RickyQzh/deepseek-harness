//! Phase 8 PTY exit checklist for dsh-jobs.

#[cfg(test)]
mod tests {
    #[test]
    fn phase8_pty_required_test_names_exist_in_this_crate() {
        let src = include_str!("types.rs");
        for name in ["JobKind::PtySend", "job_kind_pty_send_prefix"] {
            assert!(src.contains(name), "missing ported test {name}");
        }
    }
}
