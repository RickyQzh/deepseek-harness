//! Phase 7 exit checklist for dsh-commands.

#[cfg(test)]
mod tests {
    #[test]
    fn phase7_required_test_names_exist_in_this_crate() {
        let src = include_str!("registry.rs");
        let name = "parse_foo_bar_keeps_leading_space";
        assert!(src.contains(name), "missing ported test {name}");
    }
}
