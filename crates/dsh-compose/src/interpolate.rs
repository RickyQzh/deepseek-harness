//! Closed interpolators.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::ComposeError;

/// Values substituted into closed interpolators. Callers construct this; the crate does not read process env itself.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterpolateEnv {
    /// Environment map for `${env:VAR}` and `${env:VAR:-default}`.
    pub env: BTreeMap<String, String>,
    /// `${cwd}`.
    pub cwd: PathBuf,
    /// Root for `${dshHome:rel}`.
    pub dsh_home: PathBuf,
    /// `${platform}` (`linux`, `macos`, or `windows`).
    pub platform: String,
}

/// Fail when `source` contains a `!!js` tag.
///
/// # Errors
///
/// `JsTagNotSupported` when `!!js` appears anywhere in the source.
pub fn reject_js_tags(source: &str) -> Result<(), ComposeError> {
    if source.contains("!!js") {
        return Err(ComposeError::JsTagNotSupported);
    }
    Ok(())
}

/// Replace closed interpolators in `input`.
///
/// # Errors
///
/// `MissingReferent` or `UnknownInterpolator`.
pub fn interpolate(input: &str, env: &InterpolateEnv) -> Result<String, ComposeError> {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            out.push_str("${");
            rest = after;
            continue;
        };
        let expr = &after[..end];
        out.push_str(&interpolate_one(expr, env)?);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn interpolate_one(expr: &str, env: &InterpolateEnv) -> Result<String, ComposeError> {
    if expr == "cwd" {
        return Ok(env.cwd.to_string_lossy().into_owned());
    }
    if expr == "platform" {
        return Ok(env.platform.clone());
    }
    if let Some(rel) = expr.strip_prefix("dshHome:") {
        if rel.is_empty() {
            return Err(ComposeError::MissingReferent {
                referent: "dshHome:".into(),
            });
        }
        return Ok(env.dsh_home.join(rel).to_string_lossy().into_owned());
    }
    if let Some(spec) = expr.strip_prefix("env:") {
        if let Some((var, default)) = spec.split_once(":-") {
            return Ok(env
                .env
                .get(var)
                .cloned()
                .unwrap_or_else(|| default.to_string()));
        }
        return env
            .env
            .get(spec)
            .cloned()
            .ok_or_else(|| ComposeError::MissingReferent {
                referent: format!("env:{spec}"),
            });
    }
    Err(ComposeError::UnknownInterpolator {
        expr: expr.to_string(),
    })
}

/// Walk a JSON value and interpolate every string.
///
/// # Errors
///
/// Same as [`interpolate`].
pub fn interpolate_value(
    value: &serde_json::Value,
    env: &InterpolateEnv,
) -> Result<serde_json::Value, ComposeError> {
    match value {
        serde_json::Value::String(text) => Ok(serde_json::Value::String(interpolate(text, env)?)),
        serde_json::Value::Array(items) => Ok(serde_json::Value::Array(
            items
                .iter()
                .map(|item| interpolate_value(item, env))
                .collect::<Result<Vec<_>, _>>()?,
        )),
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, child) in map {
                out.insert(key.clone(), interpolate_value(child, env)?);
            }
            Ok(serde_json::Value::Object(out))
        }
        other => Ok(other.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::{InterpolateEnv, interpolate, interpolate_value, reject_js_tags};
    use crate::ComposeError;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn env() -> InterpolateEnv {
        InterpolateEnv {
            env: BTreeMap::from([("HOME".into(), "/tmp/home".into())]),
            cwd: PathBuf::from("/work"),
            dsh_home: PathBuf::from("/dsh"),
            platform: "linux".into(),
        }
    }

    #[test]
    fn interpolates_closed_forms() {
        let e = env();
        assert_eq!(interpolate("${cwd}", &e).unwrap(), "/work");
        assert_eq!(interpolate("${platform}", &e).unwrap(), "linux");
        assert_eq!(
            interpolate("${dshHome:sessions}", &e).unwrap(),
            PathBuf::from("/dsh").join("sessions").to_string_lossy()
        );
        assert_eq!(interpolate("${env:HOME}", &e).unwrap(), "/tmp/home");
        assert_eq!(
            interpolate("${env:MISSING:-fallback}", &e).unwrap(),
            "fallback"
        );
    }

    #[test]
    fn missing_env_without_default_is_fail_loud() {
        let err = interpolate("${env:MISSING}", &env()).unwrap_err();
        assert_eq!(
            err,
            ComposeError::MissingReferent {
                referent: "env:MISSING".into()
            }
        );
    }

    #[test]
    fn unknown_interpolator_is_fail_loud() {
        let err = interpolate("${foo}", &env()).unwrap_err();
        assert_eq!(
            err,
            ComposeError::UnknownInterpolator { expr: "foo".into() }
        );
    }

    #[test]
    fn empty_dsh_home_path_is_missing_referent() {
        let err = interpolate("${dshHome:}", &env()).unwrap_err();
        assert_eq!(
            err,
            ComposeError::MissingReferent {
                referent: "dshHome:".into()
            }
        );
    }

    #[test]
    fn js_tag_is_load_error() {
        let err = reject_js_tags("disabled: !!js process.platform === 'win32'").unwrap_err();
        assert_eq!(err, ComposeError::JsTagNotSupported);
        reject_js_tags("disabled: true\n").unwrap();
    }

    #[test]
    fn interpolate_value_walks_objects() {
        let out = interpolate_value(&json!({"p": "${platform}", "n": 1}), &env()).unwrap();
        assert_eq!(out, json!({"p": "linux", "n": 1}));
    }
}
