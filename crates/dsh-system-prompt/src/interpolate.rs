//! Strict `{{var}}` interpolation.

use std::collections::BTreeMap;

use crate::PromptError;

/// Pattern text embedded in interpolation diagnostics (TypeScript `String(VARIABLE_NAME)`).
const VARIABLE_NAME_PATTERN: &str = "/^[a-z][a-z0-9_]*$/";

/// Whether `name` matches `^[a-z][a-z0-9_]*$`.
pub(crate) fn is_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_lowercase() => {
            chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        }
        _ => false,
    }
}

/// A complete `{{...}}` group at the start of `text` with no `{` or `}` in the inner text.
fn group_at(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("{{")?;
    let close = rest.find("}}")?;
    let inner = &rest[..close];
    if inner.contains('{') || inner.contains('}') {
        return None;
    }
    Some(inner)
}

/// Inner text of a simple `{{name}}` group: empty or ASCII alphanumeric / underscore only.
fn is_simple_group_inner(inner: &str) -> bool {
    inner.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Byte length of `text[start..]` capped at `max_bytes` on a char boundary.
fn slice_end(text: &str, start: usize, max_bytes: usize) -> usize {
    let mut end = (start + max_bytes).min(text.len());
    while end > start && !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}

/// Interpolate strict `{{variable}}` references in one section or context.
///
/// A match of `{{` plus inner text plus `}}` with no `{` or `}` inside is a group.
/// Groups whose inner text is empty or `[A-Za-z0-9_]*` but not a valid variable name
/// fail as malformed `"{{name}}"`. `{{` with a later `}}` that is not a simple group
/// fails with the `at "…" ` diagnostic. A lone `{{` with no later `}}` is literal
/// prose. Substituted values are not scanned again.
///
/// Presence of a key with `None` is registered-but-undefined; absence is unknown.
///
/// # Errors
///
/// Returns [`PromptError::Invalid`] for malformed, unknown, or undefined references.
pub fn interpolate(
    name: &str,
    text: &str,
    variables: &BTreeMap<String, Option<String>>,
    kind: &str,
) -> Result<String, PromptError> {
    let mut result = String::new();
    let mut last = 0;
    let mut search_from = 0;
    while let Some(rel) = text[search_from..].find("{{") {
        let open = search_from + rel;
        let group = group_at(&text[open..]).filter(|inner| is_simple_group_inner(inner));
        if let Some(inner) = group {
            if !is_variable_name(inner) {
                return Err(PromptError::Invalid(format!(
                    "malformed prompt variable reference \"{{{{{inner}}}}}\" in {kind} \"{name}\" (variable names match {VARIABLE_NAME_PATTERN})"
                )));
            }
            match variables.get(inner) {
                None => {
                    let known = if variables.is_empty() {
                        "(none)".to_string()
                    } else {
                        variables.keys().cloned().collect::<Vec<_>>().join(", ")
                    };
                    return Err(PromptError::Invalid(format!(
                        "unknown prompt variable \"{{{{{inner}}}}}\" in {kind} \"{name}\"; registered variables: {known}"
                    )));
                }
                Some(None) => {
                    return Err(PromptError::Invalid(format!(
                        "prompt variable \"{{{{{inner}}}}}\" has no value for this assembly ({kind} \"{name}\")"
                    )));
                }
                Some(Some(value)) => {
                    result.push_str(&text[last..open]);
                    result.push_str(value);
                    last = open + 2 + inner.len() + 2;
                    search_from = last;
                }
            }
        } else if text[open + 2..].contains("}}") {
            let end = slice_end(text, open, 16);
            let slice = &text[open..end];
            return Err(PromptError::Invalid(format!(
                "malformed prompt variable reference at \"{slice}\"… in {kind} \"{name}\" (references are complete simple {{{{name}}}} groups)"
            )));
        } else {
            result.push_str(&text[last..open + 2]);
            last = open + 2;
            search_from = last;
        }
    }
    result.push_str(&text[last..]);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::interpolate;
    use std::collections::BTreeMap;

    fn vars(pairs: &[(&str, Option<&str>)]) -> BTreeMap<String, Option<String>> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.map(str::to_string)))
            .collect()
    }

    #[test]
    fn substitutes_a_registered_value() {
        let text = interpolate(
            "persona",
            "hello {{name}}",
            &vars(&[("name", Some("Ada"))]),
            "section",
        )
        .expect("ok");
        assert_eq!(text, "hello Ada");
    }

    #[test]
    fn unknown_variable_lists_registered_keys() {
        let error = interpolate(
            "persona",
            "x {{missing}}",
            &vars(&[("name", Some("Ada"))]),
            "section",
        )
        .expect_err("unknown");
        assert_eq!(
            error.to_string(),
            "unknown prompt variable \"{{missing}}\" in section \"persona\"; registered variables: name"
        );
    }

    #[test]
    fn undefined_registered_variable_fails() {
        let error = interpolate("persona", "x {{name}}", &vars(&[("name", None)]), "section")
            .expect_err("undef");
        assert_eq!(
            error.to_string(),
            "prompt variable \"{{name}}\" has no value for this assembly (section \"persona\")"
        );
    }

    #[test]
    fn malformed_group_fails() {
        let error =
            interpolate("persona", "x {{Name}}", &BTreeMap::new(), "section").expect_err("case");
        assert!(
            error
                .to_string()
                .contains("malformed prompt variable reference \"{{Name}}\"")
        );
        assert!(error.to_string().contains("section \"persona\""));
    }

    #[test]
    fn empty_group_is_malformed() {
        let error = interpolate("s", "x {{}} y", &BTreeMap::new(), "section").expect_err("empty");
        assert!(
            error
                .to_string()
                .contains("malformed prompt variable reference \"{{}}\"")
        );
    }

    #[test]
    fn lone_open_braces_are_literal() {
        let text = interpolate("s", "keep {{ this", &BTreeMap::new(), "section").expect("literal");
        assert_eq!(text, "keep {{ this");
    }

    #[test]
    fn later_close_without_a_simple_group_is_malformed() {
        let error =
            interpolate("s", "bad {{x y}}", &BTreeMap::new(), "section").expect_err("space");
        assert!(
            error
                .to_string()
                .contains("malformed prompt variable reference at \"{{x y}}\"")
        );
        assert!(error.to_string().contains("section \"s\""));
    }

    #[test]
    fn substituted_value_is_not_rescanned() {
        let text = interpolate(
            "s",
            "see {{token}}",
            &vars(&[("token", Some("{{name}}"))]),
            "section",
        )
        .expect("ok");
        assert_eq!(text, "see {{name}}");
    }
}
