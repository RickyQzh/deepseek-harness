//! Entry list, id-targeted patches, layer order, dump, and schema validate.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ComposeError;
use crate::disabled::Disabled;
use dsh_schema::Schema;

/// One composed plugin row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Stable patch target, when present.
    #[serde(default)]
    pub id: Option<String>,
    /// Plugin name (schema key).
    pub name: String,
    /// When true, `config` is a child entry list.
    #[serde(default)]
    pub group: bool,
    /// Disable flag or closed predicate.
    #[serde(default)]
    pub disabled: Disabled,
    /// Plugin config, or a child array lifted into `children` after parse.
    #[serde(default)]
    pub config: serde_json::Value,
    /// Isolation realm map.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub isolate: BTreeMap<String, String>,
    /// Normalized children of a group. Not a YAML field.
    #[serde(default, skip)]
    pub children: Vec<Entry>,
}

/// Id-targeted override or `insert` of new rows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Patch {
    /// Target id, or the group id for `insert`.
    #[serde(default)]
    pub id: Option<String>,
    /// Rows to append at root or under a group.
    #[serde(default)]
    pub insert: Option<Vec<Entry>>,
    /// Must match the target `name` when set.
    #[serde(default)]
    pub name: Option<String>,
    /// Whole-config replacement when set.
    #[serde(default)]
    pub config: Option<serde_json::Value>,
    /// Override for `disabled` when set.
    #[serde(default)]
    pub disabled: Option<Disabled>,
    /// Override for `group` when set.
    #[serde(default)]
    pub group: Option<bool>,
    /// Override for `isolate` when set.
    #[serde(default)]
    pub isolate: Option<BTreeMap<String, String>>,
}

/// Named patch list applied as one compose layer.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    /// Caller-supplied label (bundle id, `user`, or `--patch:N`).
    pub label: String,
    /// Patches in this layer.
    pub patches: Vec<Patch>,
}

impl Entry {
    /// Move a JSON array `config` into `children` when `group` is true.
    pub fn normalize(&mut self) {
        if self.group && self.config.is_array() {
            if let Ok(mut children) = serde_json::from_value::<Vec<Entry>>(self.config.take()) {
                for child in &mut children {
                    child.normalize();
                }
                self.children = children;
            }
        } else {
            for child in &mut self.children {
                child.normalize();
            }
        }
    }

    /// Move `children` into `config` as a JSON array when `group` is true.
    pub fn denormalize(&mut self) {
        if self.group {
            for child in &mut self.children {
                child.denormalize();
            }
            self.config = serde_json::to_value(&self.children).unwrap_or(serde_json::json!([]));
        }
    }
}

fn normalize_entries(entries: &mut [Entry]) {
    for entry in entries {
        entry.normalize();
    }
}

fn normalize_patches(patches: &mut [Patch]) {
    for patch in patches {
        if let Some(insert) = &mut patch.insert {
            normalize_entries(insert);
        }
    }
}

/// Parse an entry list. `!!js` is a load error.
///
/// # Errors
///
/// `JsTagNotSupported` or `InvalidYaml`.
pub fn parse_yaml_entries(source: &str) -> Result<Vec<Entry>, ComposeError> {
    crate::interpolate::reject_js_tags(source)?;
    let mut entries: Vec<Entry> = serde_yaml::from_str(source)
        .map_err(|error| ComposeError::InvalidYaml(error.to_string()))?;
    normalize_entries(&mut entries);
    Ok(entries)
}

/// Parse a patch list. `!!js` is a load error.
///
/// # Errors
///
/// `JsTagNotSupported` or `InvalidYaml`.
pub fn parse_yaml_patches(source: &str) -> Result<Vec<Patch>, ComposeError> {
    crate::interpolate::reject_js_tags(source)?;
    let mut patches: Vec<Patch> = serde_yaml::from_str(source)
        .map_err(|error| ComposeError::InvalidYaml(error.to_string()))?;
    normalize_patches(&mut patches);
    Ok(patches)
}

fn find_mut<'a>(entries: &'a mut [Entry], id: &str) -> Option<&'a mut Entry> {
    for entry in entries.iter_mut() {
        if entry.id.as_deref() == Some(id) {
            return Some(entry);
        }
        if let Some(found) = find_mut(&mut entry.children, id) {
            return Some(found);
        }
    }
    None
}

/// Apply `patches` to a detached clone of `data`. Inserted rows are indexed immediately.
///
/// # Errors
///
/// `MissingReferent`, `NotAGroup`, or `NameMismatch`.
pub fn apply_entry_patches(data: &[Entry], patches: &[Patch]) -> Result<Vec<Entry>, ComposeError> {
    let mut data = data.to_vec();
    for patch in patches {
        if let Some(insert) = &patch.insert {
            if let Some(id) = &patch.id {
                let target =
                    find_mut(&mut data, id).ok_or_else(|| ComposeError::MissingReferent {
                        referent: id.clone(),
                    })?;
                if !target.group {
                    return Err(ComposeError::NotAGroup { id: id.clone() });
                }
                target.children.extend(insert.iter().cloned());
            } else {
                data.extend(insert.iter().cloned());
            }
            continue;
        }
        let id = patch
            .id
            .clone()
            .ok_or_else(|| ComposeError::MissingReferent {
                referent: "<missing patch id>".into(),
            })?;
        let target = find_mut(&mut data, &id).ok_or_else(|| ComposeError::MissingReferent {
            referent: id.clone(),
        })?;
        if let Some(name) = &patch.name {
            if name != &target.name {
                return Err(ComposeError::NameMismatch {
                    id,
                    expected: target.name.clone(),
                    actual: name.clone(),
                });
            }
        }
        if let Some(config) = &patch.config {
            target.config = config.clone();
        }
        if let Some(disabled) = &patch.disabled {
            target.disabled = disabled.clone();
        }
        if let Some(group) = patch.group {
            target.group = group;
        }
        if let Some(isolate) = &patch.isolate {
            target.isolate = isolate.clone();
        }
    }
    Ok(data)
}

/// Compose from an empty root by applying each patch list in order.
///
/// # Errors
///
/// Same as [`apply_entry_patches`].
pub fn compose_layers(layers: &[Vec<Patch>]) -> Result<Vec<Entry>, ComposeError> {
    let mut data = Vec::new();
    for layer in layers {
        data = apply_entry_patches(&data, layer)?;
    }
    Ok(data)
}

/// [`compose_layers`] over [`Layer::patches`].
///
/// # Errors
///
/// Same as [`compose_layers`].
pub fn compose_named_layers(layers: &[Layer]) -> Result<Vec<Entry>, ComposeError> {
    let lists: Vec<Vec<Patch>> = layers.iter().map(|layer| layer.patches.clone()).collect();
    compose_layers(&lists)
}

/// YAML of `entries` with interpolators left as written.
///
/// # Errors
///
/// `InvalidYaml` when serde_yaml cannot serialize the tree.
pub fn dump_config(entries: &[Entry]) -> Result<String, ComposeError> {
    let mut cloned = entries.to_vec();
    for entry in &mut cloned {
        entry.denormalize();
    }
    serde_yaml::to_string(&cloned).map_err(|error| ComposeError::InvalidYaml(error.to_string()))
}

/// Validate each non-group entry whose `name` has a schema. Group children are walked.
///
/// # Errors
///
/// `InvalidConfig` when `Schema::validate` fails.
pub fn validate_configs(
    entries: &[Entry],
    schemas: &BTreeMap<String, Schema>,
) -> Result<(), ComposeError> {
    for entry in entries {
        if entry.group {
            validate_configs(&entry.children, schemas)?;
            continue;
        }
        if let Some(schema) = schemas.get(&entry.name) {
            schema
                .validate(&entry.config)
                .map_err(|source| ComposeError::InvalidConfig {
                    id: entry.id.clone(),
                    name: entry.name.clone(),
                    source,
                })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        Entry, Layer, Patch, apply_entry_patches, compose_named_layers, dump_config,
        parse_yaml_entries, parse_yaml_patches, validate_configs,
    };
    use crate::ComposeError;
    use crate::disabled::Disabled;
    use dsh_schema::Schema;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn row(id: &str, name: &str, config: serde_json::Value) -> Entry {
        Entry {
            id: Some(id.into()),
            name: name.into(),
            group: false,
            disabled: Disabled::Flag(false),
            config,
            isolate: BTreeMap::new(),
            children: Vec::new(),
        }
    }

    #[test]
    fn insert_then_patch_in_the_same_list() {
        let patches = parse_yaml_patches(
            r#"
- insert:
    - id: new-row
      name: dsh-foo
      config: { a: 1 }
- id: new-row
  config: { a: 2 }
"#,
        )
        .unwrap();
        let out = apply_entry_patches(&[], &patches).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id.as_deref(), Some("new-row"));
        assert_eq!(out[0].config, json!({"a": 2}));
    }

    #[test]
    fn missing_patch_id_is_fail_loud() {
        let patches = vec![Patch {
            id: Some("missing".into()),
            insert: None,
            name: None,
            config: Some(json!({"a": 1})),
            disabled: None,
            group: None,
            isolate: None,
        }];
        let err = apply_entry_patches(&[], &patches).unwrap_err();
        assert_eq!(
            err,
            ComposeError::MissingReferent {
                referent: "missing".into()
            }
        );
    }

    #[test]
    fn layer_order_is_empty_root_then_each_layer() {
        let base = parse_yaml_patches(
            r#"
- insert:
    - id: bash
      name: dsh-tool-bash
      config: { timeout: 10 }
"#,
        )
        .unwrap();
        let user = parse_yaml_patches(
            r#"
- id: bash
  config: { timeout: 99 }
"#,
        )
        .unwrap();
        let overlay = parse_yaml_patches(
            r#"
- insert:
    - id: extra
      name: dsh-extra
      config: {}
"#,
        )
        .unwrap();
        let tree = compose_named_layers(&[
            Layer {
                label: "dsh-base".into(),
                patches: base,
            },
            Layer {
                label: "user".into(),
                patches: user,
            },
            Layer {
                label: "--patch:0".into(),
                patches: overlay,
            },
        ])
        .unwrap();
        assert_eq!(tree[0].config, json!({"timeout": 99}));
        assert_eq!(tree[1].id.as_deref(), Some("extra"));
    }

    #[test]
    fn dump_config_leaves_interpolators_unevaluated() {
        let dump = dump_config(&[row(
            "home",
            "dsh-sessions",
            json!({"path": "${dshHome:sessions}"}),
        )])
        .unwrap();
        assert!(dump.contains("${dshHome:sessions}"));
        assert!(!dump.contains("/dsh/sessions"));
    }

    #[test]
    fn validate_configs_uses_schema_by_plugin_name() {
        let schema = Schema::Object {
            properties: BTreeMap::from([(
                "timeout".into(),
                Schema::Integer {
                    default: None,
                    minimum: Some(1),
                    maximum: None,
                },
            )]),
            required: vec!["timeout".into()],
        };
        let schemas = BTreeMap::from([("dsh-tool-bash".into(), schema)]);
        validate_configs(
            &[row("bash", "dsh-tool-bash", json!({"timeout": 5}))],
            &schemas,
        )
        .unwrap();
        let err =
            validate_configs(&[row("bash", "dsh-tool-bash", json!({}))], &schemas).unwrap_err();
        assert!(matches!(err, ComposeError::InvalidConfig { name, .. } if name == "dsh-tool-bash"));
    }

    #[test]
    fn parse_yaml_entries_rejects_js_tag() {
        let err = parse_yaml_entries("- name: x\n  disabled: !!js true\n").unwrap_err();
        assert_eq!(err, ComposeError::JsTagNotSupported);
    }
}
