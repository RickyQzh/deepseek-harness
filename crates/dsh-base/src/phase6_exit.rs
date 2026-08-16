//! Phase 6 exit checklist for dsh-base.

#[cfg(test)]
mod tests {
    #[test]
    fn phase6_required_test_names_exist_in_this_crate() {
        let src = include_str!("lib.rs");
        for name in [
            "register_base_plugins_provides_approval_compaction_skills_web_jobs_subagents",
            "base_yaml_rejects_js_tag_substring",
        ] {
            assert!(src.contains(name), "missing ported test {name}");
        }
    }
}
