//! Plugin config schema and JSON Schema export.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// JSON-serializable config schema for validation and the Settings UI.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Schema {
    /// UTF-8 string.
    #[serde(rename = "string")]
    String {
        /// Default when the field is omitted.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<String>,
    },
    /// Signed 64-bit integer.
    #[serde(rename = "integer")]
    Integer {
        /// Default when the field is omitted.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<i64>,
        /// Inclusive lower bound.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        minimum: Option<i64>,
        /// Inclusive upper bound.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        maximum: Option<i64>,
    },
    /// Boolean.
    #[serde(rename = "boolean")]
    Boolean {
        /// Default when the field is omitted.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<bool>,
    },
    /// IEEE-754 number.
    #[serde(rename = "number")]
    Number {
        /// Default when the field is omitted.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<f64>,
    },
    /// Object with named properties.
    #[serde(rename = "object")]
    Object {
        /// Property schemas.
        #[serde(default)]
        properties: BTreeMap<String, Schema>,
        /// Names that must be present.
        #[serde(default)]
        required: Vec<String>,
    },
    /// Homogeneous array.
    #[serde(rename = "array")]
    Array {
        /// Schema for each element.
        items: Box<Schema>,
    },
}

/// Validation failure for one value.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SchemaError {
    /// JSON value kind did not match the schema.
    #[error("expected {expected}, got {actual}")]
    TypeMismatch {
        /// Schema type name.
        expected: &'static str,
        /// Actual JSON type name.
        actual: String,
    },
    /// A required object property was absent.
    #[error("missing required property `{0}`")]
    MissingProperty(String),
    /// Integer below `minimum`.
    #[error("integer `{value}` is below minimum {minimum}")]
    BelowMinimum {
        /// Actual value.
        value: i64,
        /// Inclusive minimum.
        minimum: i64,
    },
    /// Integer above `maximum`.
    #[error("integer `{value}` is above maximum {maximum}")]
    AboveMaximum {
        /// Actual value.
        value: i64,
        /// Inclusive maximum.
        maximum: i64,
    },
}

impl Schema {
    /// Reject `value` when it does not match this schema.
    ///
    /// # Errors
    ///
    /// `SchemaError` naming the first mismatch.
    pub fn validate(&self, value: &serde_json::Value) -> Result<(), SchemaError> {
        match self {
            Schema::String { .. } => match value {
                serde_json::Value::String(_) => Ok(()),
                other => Err(SchemaError::TypeMismatch {
                    expected: "string",
                    actual: type_name(other).into(),
                }),
            },
            Schema::Boolean { .. } => match value {
                serde_json::Value::Bool(_) => Ok(()),
                other => Err(SchemaError::TypeMismatch {
                    expected: "boolean",
                    actual: type_name(other).into(),
                }),
            },
            Schema::Number { .. } => match value {
                serde_json::Value::Number(_) => Ok(()),
                other => Err(SchemaError::TypeMismatch {
                    expected: "number",
                    actual: type_name(other).into(),
                }),
            },
            Schema::Integer {
                minimum, maximum, ..
            } => {
                let Some(int) = value.as_i64() else {
                    return Err(SchemaError::TypeMismatch {
                        expected: "integer",
                        actual: type_name(value).into(),
                    });
                };
                if let Some(minimum) = *minimum {
                    if int < minimum {
                        return Err(SchemaError::BelowMinimum {
                            value: int,
                            minimum,
                        });
                    }
                }
                if let Some(maximum) = *maximum {
                    if int > maximum {
                        return Err(SchemaError::AboveMaximum {
                            value: int,
                            maximum,
                        });
                    }
                }
                Ok(())
            }
            Schema::Object {
                properties,
                required,
            } => {
                let Some(obj) = value.as_object() else {
                    return Err(SchemaError::TypeMismatch {
                        expected: "object",
                        actual: type_name(value).into(),
                    });
                };
                for key in required {
                    if !obj.contains_key(key) {
                        return Err(SchemaError::MissingProperty(key.clone()));
                    }
                }
                for (key, schema) in properties {
                    if let Some(child) = obj.get(key) {
                        schema.validate(child)?;
                    }
                }
                Ok(())
            }
            Schema::Array { items } => {
                let Some(arr) = value.as_array() else {
                    return Err(SchemaError::TypeMismatch {
                        expected: "array",
                        actual: type_name(value).into(),
                    });
                };
                for child in arr {
                    items.validate(child)?;
                }
                Ok(())
            }
        }
    }

    /// JSON Schema document for the Settings UI (`{"type":...}`).
    #[must_use]
    pub fn to_json_schema(&self) -> serde_json::Value {
        match self {
            Schema::String { default } => {
                json_type("string", default.as_ref().map(|d| serde_json::json!(d)))
            }
            Schema::Boolean { default } => {
                json_type("boolean", default.map(|d| serde_json::json!(d)))
            }
            Schema::Number { default } => {
                json_type("number", default.map(|d| serde_json::json!(d)))
            }
            Schema::Integer {
                default,
                minimum,
                maximum,
            } => {
                let mut map =
                    serde_json::Map::from_iter([("type".into(), serde_json::json!("integer"))]);
                if let Some(default) = default {
                    map.insert("default".into(), serde_json::json!(default));
                }
                if let Some(minimum) = minimum {
                    map.insert("minimum".into(), serde_json::json!(minimum));
                }
                if let Some(maximum) = maximum {
                    map.insert("maximum".into(), serde_json::json!(maximum));
                }
                serde_json::Value::Object(map)
            }
            Schema::Object {
                properties,
                required,
            } => {
                let props: serde_json::Map<String, serde_json::Value> = properties
                    .iter()
                    .map(|(k, v)| (k.clone(), v.to_json_schema()))
                    .collect();
                serde_json::json!({
                    "type": "object",
                    "properties": props,
                    "required": required,
                })
            }
            Schema::Array { items } => serde_json::json!({
                "type": "array",
                "items": items.to_json_schema(),
            }),
        }
    }
}

fn json_type(ty: &str, default: Option<serde_json::Value>) -> serde_json::Value {
    let mut map = serde_json::Map::from_iter([("type".into(), serde_json::json!(ty))]);
    if let Some(default) = default {
        map.insert("default".into(), default);
    }
    serde_json::Value::Object(map)
}

fn type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::{Schema, SchemaError};
    use serde_json::json;
    use std::collections::BTreeMap;

    fn timeout_schema() -> Schema {
        Schema::Object {
            properties: BTreeMap::from([(
                "timeout".into(),
                Schema::Integer {
                    default: Some(30),
                    minimum: Some(1),
                    maximum: Some(120),
                },
            )]),
            required: vec!["timeout".into()],
        }
    }

    #[test]
    fn validate_accepts_in_range_object() {
        timeout_schema().validate(&json!({"timeout": 30})).unwrap();
    }

    #[test]
    fn validate_rejects_missing_required() {
        let err = timeout_schema().validate(&json!({})).unwrap_err();
        assert_eq!(err, SchemaError::MissingProperty("timeout".into()));
    }

    #[test]
    fn validate_rejects_below_minimum() {
        let err = timeout_schema()
            .validate(&json!({"timeout": 0}))
            .unwrap_err();
        assert_eq!(
            err,
            SchemaError::BelowMinimum {
                value: 0,
                minimum: 1
            }
        );
    }

    #[test]
    fn json_schema_is_json_serializable_object() {
        let doc = timeout_schema().to_json_schema();
        assert_eq!(doc["type"], "object");
        assert_eq!(doc["properties"]["timeout"]["type"], "integer");
        assert_eq!(doc["properties"]["timeout"]["minimum"], 1);
        assert_eq!(doc["required"], json!(["timeout"]));
        let _again: String = serde_json::to_string(&doc).unwrap();
    }

    #[test]
    fn schema_enum_round_trips_serde_json() {
        let raw = serde_json::to_value(timeout_schema()).unwrap();
        let back: Schema = serde_json::from_value(raw).unwrap();
        assert_eq!(back, timeout_schema());
    }
}
