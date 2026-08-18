//! Durable per-namespace JSON settings.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::EXPOSED_NAMESPACES;
use crate::error::SettingsError;

const UI_ONBOARDING: &str = "ui-onboarding";
const APPLIES_LIVE: &str = "live";

/// Wire view of one exposed settings namespace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NamespaceView {
    ns: String,
    value: Value,
    revision: u64,
    schema: Value,
    secrets: Vec<Value>,
    applies: &'static str,
}

impl NamespaceView {
    /// Namespace key.
    #[must_use]
    pub fn ns(&self) -> &str {
        &self.ns
    }

    /// User section JSON object.
    #[must_use]
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Monotonic revision of the stored user section.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Schema JSON. This crate does not port schemastery; the object is empty.
    #[must_use]
    pub fn schema(&self) -> &Value {
        &self.schema
    }

    /// Secret-slot list. `ui-onboarding` has none.
    #[must_use]
    pub fn secrets(&self) -> &[Value] {
        &self.secrets
    }

    /// When changes apply. Exposed namespaces in this crate are `"live"`.
    #[must_use]
    pub fn applies(&self) -> &str {
        self.applies
    }
}

/// `describe_all` snapshot of every exposed namespace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DescribeAll {
    namespaces: Vec<NamespaceView>,
}

impl DescribeAll {
    /// Exposed namespace views in [`EXPOSED_NAMESPACES`] order.
    #[must_use]
    pub fn namespaces(&self) -> &[NamespaceView] {
        &self.namespaces
    }
}

mod mutate_op {
    use serde_json::{Map, Value};

    use super::{SettingsError, default_document};

    /// One path-addressed edit applied by [`super::SettingsService::mutate`].
    #[non_exhaustive]
    pub enum MutateOp {
        /// Write `value` at `path`, creating intermediate objects.
        Set {
            /// Path from the section root; empty replaces the root.
            path: Vec<String>,
            /// JSON value to write.
            value: Value,
        },
        /// Remove the value at `path`.
        Unset {
            /// Path from the section root; empty resets to the default document.
            path: Vec<String>,
        },
    }

    impl MutateOp {
        /// Write `value` at `path`. An empty path replaces the section root and `value` must be an object.
        #[must_use]
        pub fn set(path: Vec<String>, value: Value) -> Self {
            Self::Set { path, value }
        }

        /// Remove the value at `path`. An empty path resets the section to the namespace default document.
        #[must_use]
        pub fn unset(path: Vec<String>) -> Self {
            Self::Unset { path }
        }

        pub(super) fn apply(
            &self,
            ns: &str,
            section: Map<String, Value>,
        ) -> Result<Map<String, Value>, SettingsError> {
            match self {
                Self::Set { path, value } => {
                    if path.is_empty() {
                        let Value::Object(map) = value.clone() else {
                            return Err(SettingsError::rejected(
                                "settings mutate: setting the section root requires a JSON object",
                            ));
                        };
                        return Ok(map);
                    }
                    Ok(set_at(section, path, value.clone()))
                }
                Self::Unset { path } => {
                    if path.is_empty() {
                        return Ok(default_document(ns));
                    }
                    Ok(unset_at(section, path))
                }
            }
        }
    }

    fn set_at(
        mut section: Map<String, Value>,
        path: &[String],
        value: Value,
    ) -> Map<String, Value> {
        let Some((head, rest)) = path.split_first() else {
            return section;
        };
        if rest.is_empty() {
            section.insert(head.clone(), value);
            return section;
        }
        let child = match section.remove(head) {
            Some(Value::Object(map)) => map,
            _ => Map::new(),
        };
        section.insert(head.clone(), Value::Object(set_at(child, rest, value)));
        section
    }

    fn unset_at(mut section: Map<String, Value>, path: &[String]) -> Map<String, Value> {
        let Some((head, rest)) = path.split_first() else {
            return section;
        };
        if rest.is_empty() {
            section.remove(head);
            return section;
        }
        match section.remove(head) {
            Some(Value::Object(child)) => {
                section.insert(head.clone(), Value::Object(unset_at(child, rest)));
                section
            }
            Some(other) => {
                section.insert(head.clone(), other);
                section
            }
            None => section,
        }
    }
}

/// One path-addressed edit applied by [`SettingsService::mutate`].
pub use mutate_op::MutateOp;

#[derive(Serialize, Deserialize)]
struct PersistDocument {
    revision: u64,
    value: Value,
}

struct Loaded {
    revision: u64,
    value: Map<String, Value>,
}

/// File-backed settings service. Mutations take an in-process [`Mutex`] and write JSON atomically.
pub struct SettingsService {
    dir: PathBuf,
    lock: Mutex<()>,
}

impl std::fmt::Debug for SettingsService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsService")
            .field("dir", &self.dir)
            .finish()
    }
}

impl SettingsService {
    /// Persist namespace files under `dir` as `{ns}.json`.
    #[must_use]
    pub fn with_dir(dir: PathBuf) -> Self {
        Self {
            dir,
            lock: Mutex::new(()),
        }
    }

    /// Describe one exposed namespace. A missing file is the default document at revision 0.
    ///
    /// # Errors
    ///
    /// [`SettingsError::NotExposed`] when `ns` is not in [`EXPOSED_NAMESPACES`].
    /// Persist read failures.
    pub fn describe(&self, ns: &str) -> Result<NamespaceView, SettingsError> {
        ensure_exposed(ns)?;
        let _guard = self.lock.lock().expect("settings lock");
        Ok(view_from_loaded(ns, self.load_unlocked(ns)?))
    }

    /// Describe every exposed namespace.
    ///
    /// # Errors
    ///
    /// Persist read failures.
    pub fn describe_all(&self) -> Result<DescribeAll, SettingsError> {
        let _guard = self.lock.lock().expect("settings lock");
        let mut namespaces = Vec::with_capacity(EXPOSED_NAMESPACES.len());
        for ns in EXPOSED_NAMESPACES {
            namespaces.push(view_from_loaded(ns, self.load_unlocked(ns)?));
        }
        Ok(DescribeAll { namespaces })
    }

    /// Merge `patch` into the user section (object keys merge; non-objects overwrite).
    ///
    /// # Errors
    ///
    /// [`SettingsError::NotExposed`], [`SettingsError::Conflict`], [`SettingsError::Rejected`]
    /// when `patch` is not an object, or persist failures.
    pub fn update(
        &self,
        ns: &str,
        patch: Value,
        expected: Option<u64>,
    ) -> Result<NamespaceView, SettingsError> {
        let Value::Object(patch) = patch else {
            ensure_exposed(ns)?;
            return Err(SettingsError::rejected(
                "settings update patch must be a JSON object",
            ));
        };
        self.write(ns, expected, |current| Ok(merge_objects(current, patch)))
    }

    /// Replace the user section wholesale. `section` must be a JSON object.
    ///
    /// # Errors
    ///
    /// [`SettingsError::NotExposed`], [`SettingsError::Conflict`], [`SettingsError::Rejected`]
    /// when `section` is not an object, or persist failures.
    pub fn replace(
        &self,
        ns: &str,
        section: Value,
        expected: Option<u64>,
    ) -> Result<NamespaceView, SettingsError> {
        let Value::Object(section) = section else {
            ensure_exposed(ns)?;
            return Err(SettingsError::rejected(
                "settings replace section must be a JSON object",
            ));
        };
        self.write(ns, expected, |_| Ok(section))
    }

    /// Apply ordered path ops to the user section.
    ///
    /// # Errors
    ///
    /// [`SettingsError::NotExposed`], [`SettingsError::Conflict`], [`SettingsError::Rejected`]
    /// when an empty-path set is not an object, or persist failures.
    pub fn mutate(
        &self,
        ns: &str,
        ops: Vec<MutateOp>,
        expected: Option<u64>,
    ) -> Result<NamespaceView, SettingsError> {
        self.write(ns, expected, |mut current| {
            for op in &ops {
                current = op.apply(ns, current)?;
            }
            Ok(current)
        })
    }

    fn write(
        &self,
        ns: &str,
        expected: Option<u64>,
        mutate: impl FnOnce(Map<String, Value>) -> Result<Map<String, Value>, SettingsError>,
    ) -> Result<NamespaceView, SettingsError> {
        ensure_exposed(ns)?;
        let _guard = self.lock.lock().expect("settings lock");
        let loaded = self.load_unlocked(ns)?;
        if let Some(expected) = expected {
            if expected != loaded.revision {
                return Err(SettingsError::conflict(ns, expected, loaded.revision));
            }
        }
        let value = mutate(loaded.value)?;
        let revision = loaded
            .revision
            .checked_add(1)
            .ok_or_else(|| SettingsError::io("settings revision overflow"))?;
        self.save_unlocked(ns, revision, &value)?;
        Ok(view_from_loaded(ns, Loaded { revision, value }))
    }

    fn load_unlocked(&self, ns: &str) -> Result<Loaded, SettingsError> {
        let path = self.file_path(ns);
        match fs::read(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Loaded {
                revision: 0,
                value: default_document(ns),
            }),
            Err(error) => Err(SettingsError::io(error.to_string())),
            Ok(bytes) => parse_document(&bytes),
        }
    }

    fn save_unlocked(
        &self,
        ns: &str,
        revision: u64,
        value: &Map<String, Value>,
    ) -> Result<(), SettingsError> {
        let document = PersistDocument {
            revision,
            value: Value::Object(value.clone()),
        };
        let bytes = serde_json::to_vec(&document)
            .map_err(|error| SettingsError::corrupt(error.to_string()))?;
        write_atomic(&self.file_path(ns), &bytes)
    }

    fn file_path(&self, ns: &str) -> PathBuf {
        self.dir.join(format!("{ns}.json"))
    }
}

fn ensure_exposed(ns: &str) -> Result<(), SettingsError> {
    if EXPOSED_NAMESPACES.contains(&ns) {
        Ok(())
    } else {
        Err(SettingsError::not_exposed(ns))
    }
}

fn default_document(ns: &str) -> Map<String, Value> {
    match ns {
        UI_ONBOARDING => {
            let mut map = Map::new();
            map.insert("welcomeNoticeVersion".into(), Value::String(String::new()));
            map
        }
        _ => Map::new(),
    }
}

fn view_from_loaded(ns: &str, loaded: Loaded) -> NamespaceView {
    NamespaceView {
        ns: ns.to_string(),
        value: Value::Object(loaded.value),
        revision: loaded.revision,
        schema: Value::Object(Map::new()),
        secrets: Vec::new(),
        applies: APPLIES_LIVE,
    }
}

fn parse_document(bytes: &[u8]) -> Result<Loaded, SettingsError> {
    let document: PersistDocument =
        serde_json::from_slice(bytes).map_err(|error| SettingsError::corrupt(error.to_string()))?;
    let Value::Object(value) = document.value else {
        return Err(SettingsError::corrupt(
            "settings persist value must be a JSON object",
        ));
    };
    Ok(Loaded {
        revision: document.revision,
        value,
    })
}

fn merge_objects(mut under: Map<String, Value>, over: Map<String, Value>) -> Map<String, Value> {
    for (key, over_value) in over {
        match under.remove(&key) {
            Some(under_value) => {
                under.insert(key, merge_json(under_value, over_value));
            }
            None => {
                under.insert(key, over_value);
            }
        }
    }
    under
}

fn merge_json(under: Value, over: Value) -> Value {
    match (under, over) {
        (Value::Object(under), Value::Object(over)) => Value::Object(merge_objects(under, over)),
        (_, over) => over,
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), SettingsError> {
    ensure_parent(path)?;
    let tmp = tmp_path(path)?;
    if let Err(error) = fs::write(&tmp, bytes) {
        let _ = fs::remove_file(&tmp);
        return Err(SettingsError::io(error.to_string()));
    }
    if let Err(error) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(SettingsError::io(error.to_string()));
    }
    Ok(())
}

fn ensure_parent(path: &Path) -> Result<(), SettingsError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }
    fs::create_dir_all(parent).map_err(|error| SettingsError::io(error.to_string()))
}

fn tmp_path(path: &Path) -> Result<PathBuf, SettingsError> {
    let Some(name) = path.file_name() else {
        return Err(SettingsError::io("settings persist path has no file name"));
    };
    let mut tmp_name = name.to_os_string();
    tmp_name.push(format!(".{}.tmp", std::process::id()));
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => Ok(parent.join(tmp_name)),
        _ => Ok(PathBuf::from(tmp_name)),
    }
}

/// Create `dir`, failing when it cannot be created.
pub(crate) fn ensure_settings_dir(dir: &Path) -> Result<(), SettingsError> {
    fs::create_dir_all(dir).map_err(|error| SettingsError::io(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::{MutateOp, SettingsService};
    use crate::EXPOSED_NAMESPACES;
    use serde_json::json;

    fn test_temp_dir(prefix: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn missing_file_describes_default_at_revision_zero() {
        let service = SettingsService::with_dir(test_temp_dir("settings-default"));
        let view = service.describe("ui-onboarding").unwrap();
        assert_eq!(view.ns(), "ui-onboarding");
        assert_eq!(view.revision(), 0);
        assert_eq!(view.value(), &json!({ "welcomeNoticeVersion": "" }));
        assert_eq!(view.schema(), &json!({}));
        assert!(view.secrets().is_empty());
        assert_eq!(view.applies(), "live");
        let all = service.describe_all().unwrap();
        assert_eq!(all.namespaces().len(), 1);
        assert_eq!(all.namespaces()[0].ns(), "ui-onboarding");
    }

    #[test]
    fn update_merges_objects_and_persists_revision() {
        let dir = test_temp_dir("settings-merge");
        let service = SettingsService::with_dir(dir.clone());
        let ns = EXPOSED_NAMESPACES[0];
        service
            .update(
                ns,
                json!({ "welcomeNoticeVersion": "v1", "nested": { "a": 1 } }),
                None,
            )
            .unwrap();
        let view = service
            .update(ns, json!({ "nested": { "b": 2 } }), None)
            .unwrap();
        assert_eq!(view.revision(), 2);
        assert_eq!(
            view.value(),
            &json!({
                "welcomeNoticeVersion": "v1",
                "nested": { "a": 1, "b": 2 }
            })
        );
        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("ui-onboarding.json")).unwrap())
                .unwrap();
        assert_eq!(raw["revision"], json!(2));
        assert_eq!(raw["value"]["welcomeNoticeVersion"], json!("v1"));
    }

    #[test]
    fn replace_and_mutate_empty_path() {
        let service = SettingsService::with_dir(test_temp_dir("settings-mutate"));
        let ns = EXPOSED_NAMESPACES[0];
        let replaced = service
            .replace(ns, json!({ "welcomeNoticeVersion": "kept" }), None)
            .unwrap();
        assert_eq!(replaced.value(), &json!({ "welcomeNoticeVersion": "kept" }));
        let set_root = service
            .mutate(
                ns,
                vec![MutateOp::set(
                    Vec::new(),
                    json!({ "welcomeNoticeVersion": "root" }),
                )],
                None,
            )
            .unwrap();
        assert_eq!(set_root.value(), &json!({ "welcomeNoticeVersion": "root" }));
        let reset = service
            .mutate(ns, vec![MutateOp::unset(Vec::new())], None)
            .unwrap();
        assert_eq!(reset.value(), &json!({ "welcomeNoticeVersion": "" }));
        let nested = service
            .mutate(
                ns,
                vec![MutateOp::set(
                    vec!["extra".into(), "flag".into()],
                    json!(true),
                )],
                None,
            )
            .unwrap();
        assert_eq!(nested.value()["extra"]["flag"], json!(true));
    }

    #[test]
    fn non_object_patch_is_settings_rejected() {
        let service = SettingsService::with_dir(test_temp_dir("settings-rejected"));
        let error = service
            .update("ui-onboarding", json!("nope"), None)
            .expect_err("non-object");
        assert_eq!(error.code(), "settings-rejected");
        let error = service
            .replace("ui-onboarding", json!(["x"]), None)
            .expect_err("array");
        assert_eq!(error.code(), "settings-rejected");
        let error = service
            .mutate(
                "ui-onboarding",
                vec![MutateOp::set(Vec::new(), json!("root"))],
                None,
            )
            .expect_err("root");
        assert_eq!(error.code(), "settings-rejected");
    }

    #[test]
    fn expected_none_skips_revision_check() {
        let service = SettingsService::with_dir(test_temp_dir("settings-nocheck"));
        service
            .update(
                "ui-onboarding",
                json!({ "welcomeNoticeVersion": "a" }),
                None,
            )
            .unwrap();
        let view = service
            .update(
                "ui-onboarding",
                json!({ "welcomeNoticeVersion": "b" }),
                None,
            )
            .unwrap();
        assert_eq!(view.revision(), 2);
        assert_eq!(view.value()["welcomeNoticeVersion"], json!("b"));
    }
}
