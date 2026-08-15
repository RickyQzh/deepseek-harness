//! Lossless JSON argument freeze.

use serde_json::Value;

use crate::ToolError;

/// Clone one lossless JSON value. Rejects non-finite numbers.
pub fn freeze_args(value: &Value) -> Result<Value, ToolError> {
    if let Value::Number(number) = value {
        if number.as_f64().is_none() && number.as_i64().is_none() && number.as_u64().is_none() {
            return Err(ToolError::ArgsNotJson);
        }
    }
    if let Value::Array(items) = value {
        for item in items {
            freeze_args(item)?;
        }
    }
    if let Value::Object(map) = value {
        for item in map.values() {
            freeze_args(item)?;
        }
    }
    Ok(value.clone())
}

/// Parse model-authored argument JSON. Empty → `{}`. Invalid JSON → the raw string.
pub fn freeze_args_from_raw(raw: &str) -> Result<Value, ToolError> {
    if raw.is_empty() {
        return Ok(serde_json::json!({}));
    }
    match serde_json::from_str::<Value>(raw) {
        Ok(value) => freeze_args(&value),
        Err(_) => Ok(Value::String(raw.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::{freeze_args, freeze_args_from_raw};
    use serde_json::json;

    #[test]
    fn freeze_clones_an_object_independently() {
        let original = json!({"text": "hi", "n": 1});
        let frozen = freeze_args(&original).expect("freeze");
        assert_eq!(frozen, original);
        let mut mutated = frozen.clone();
        mutated["text"] = json!("changed");
        assert_eq!(original["text"], "hi");
        assert_eq!(frozen["text"], "hi");
    }

    #[test]
    fn freeze_from_raw_empty_becomes_empty_object() {
        assert_eq!(freeze_args_from_raw("").expect("empty"), json!({}));
    }

    #[test]
    fn freeze_from_raw_invalid_json_is_kept_as_string() {
        assert_eq!(
            freeze_args_from_raw("not-json").expect("raw"),
            json!("not-json")
        );
    }

    #[test]
    fn freeze_from_raw_parses_object() {
        assert_eq!(
            freeze_args_from_raw("{\"text\":\"ping\"}").expect("json"),
            json!({"text": "ping"})
        );
    }
}
