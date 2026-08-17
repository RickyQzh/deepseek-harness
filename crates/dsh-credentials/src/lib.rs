//! Per-request credential references. Consumers re-resolve on every operation.

pub mod plugin;

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use dsh_brand::Branded;
use dsh_rpc::RpcErrorCode;

/// Compile-time brand for a POSIX credential reference.
pub struct CredentialRefTag;

/// POSIX-named credential reference. Local newtype; `dsh-brand` does not name it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CredentialRef(Branded<CredentialRefTag>);

impl CredentialRef {
    /// Brand `value` as a credential reference without POSIX validation.
    #[must_use]
    pub(crate) fn new(value: impl Into<String>) -> Self {
        Self(Branded::new(value))
    }

    /// Borrow the reference name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Unwrap the inner string.
    #[must_use]
    pub fn into_inner(self) -> String {
        self.0.into_inner()
    }
}

fn is_posix_ref(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Brand `value` as a credential reference.
///
/// # Errors
///
/// Returns [`CredentialError::InvalidRef`] when `value` is not a POSIX identifier
/// matching `/^[A-Za-z_][A-Za-z0-9_]*$/`.
pub fn credential_ref(value: &str) -> Result<CredentialRef, CredentialError> {
    if !is_posix_ref(value) {
        return Err(CredentialError::InvalidRef(value.to_string()));
    }
    Ok(CredentialRef::new(value))
}

/// One resolved secret and the layer that supplied it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedCredential {
    /// Non-empty secret value.
    pub value: String,
    /// Layer id: `"env"`, `"file"`, or `"memory"`.
    pub source: String,
}

/// Source and writability for one reference, without the secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialInfo {
    /// Whether [`CredentialProvider::resolve`] would currently return a value.
    pub configured: bool,
    /// Layer currently supplying the value; absent while unconfigured.
    pub source: Option<String>,
    /// Whether [`CredentialProvider::set`] would currently succeed for this reference.
    pub writable: bool,
}

/// Wire view of one credential reference. Never includes the secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialView {
    configured: bool,
    source: Option<String>,
    writable: bool,
}

impl CredentialView {
    /// Whether any layer currently supplies a non-empty value.
    #[must_use]
    pub fn configured(&self) -> bool {
        self.configured
    }

    /// Winning layer when configured (`env`, `file`, `memory`); `None` while unconfigured.
    #[must_use]
    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }

    /// Whether [`LayeredCredentials::set_ref`] can affect this reference.
    #[must_use]
    pub fn writable(&self) -> bool {
        self.writable
    }
}

/// Durable credential store. The provided service type is [`LayeredCredentials`].
pub type CredentialStore = LayeredCredentials;

/// Resolve, describe, and mutate credential references.
pub trait CredentialProvider: Send + Sync {
    /// Resolve one reference to its current value. Resolution is per call.
    ///
    /// # Errors
    ///
    /// Returns a [`CredentialError`] when the provider cannot read a layer.
    fn resolve(
        &self,
        credential: &CredentialRef,
    ) -> Result<Option<ResolvedCredential>, CredentialError>;

    /// Describe one reference without exposing the value.
    ///
    /// # Errors
    ///
    /// Returns a [`CredentialError`] when the provider cannot read a layer.
    fn describe(&self, credential: &CredentialRef) -> Result<CredentialInfo, CredentialError>;

    /// Store one non-empty value in the writable (memory) layer.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialError::EmptyValue`] for an empty `value`, or
    /// [`CredentialError::Shadowed`] when a read-only env layer currently
    /// shadows the reference.
    fn set(&mut self, credential: &CredentialRef, value: String) -> Result<(), CredentialError>;

    /// Remove one reference from the writable (memory) layer.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialError::Shadowed`] when a read-only env layer
    /// currently shadows the reference.
    fn unset(&mut self, credential: &CredentialRef) -> Result<(), CredentialError>;
}

/// Memory + optional YAML file map + live process environment.
pub struct LayeredCredentials {
    layers: Mutex<Layers>,
    persist_path: Option<PathBuf>,
}

struct Layers {
    memory: HashMap<String, String>,
    file: HashMap<String, String>,
}

/// Prints layer key names only; secret values stay out of the debug string.
impl fmt::Debug for LayeredCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let layers = self.lock();
        f.debug_struct("LayeredCredentials")
            .field("memory_keys", &Keys(&layers.memory))
            .field("file_keys", &Keys(&layers.file))
            .finish()
    }
}

struct Keys<'a>(&'a HashMap<String, String>);

impl fmt::Debug for Keys<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.0.keys()).finish()
    }
}

impl LayeredCredentials {
    /// Empty memory, no file map.
    #[must_use]
    pub fn new() -> Self {
        Self {
            layers: Mutex::new(Layers {
                memory: HashMap::new(),
                file: HashMap::new(),
            }),
            persist_path: None,
        }
    }

    /// Use `map` as the file layer with empty memory.
    #[must_use]
    pub fn with_file_map(map: BTreeMap<String, String>) -> Self {
        Self {
            layers: Mutex::new(Layers {
                memory: HashMap::new(),
                file: map.into_iter().collect(),
            }),
            persist_path: None,
        }
    }

    /// Parse a YAML object of string keys to string values as the file layer.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialError::InvalidFile`] when `text` is not a YAML mapping
    /// of strings to strings.
    pub fn from_yaml(text: &str) -> Result<Self, CredentialError> {
        let file: BTreeMap<String, String> = serde_yaml::from_str(text)
            .map_err(|error| CredentialError::InvalidFile(error.to_string()))?;
        Ok(Self::with_file_map(file))
    }

    /// Load `{path}` as the file layer when present. Creates the parent directory.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialError::Io`] when the parent cannot be created or the file
    /// cannot be read, or [`CredentialError::InvalidFile`] when the file is not a
    /// YAML string map.
    pub fn with_persist_path(path: PathBuf) -> Result<Self, CredentialError> {
        ensure_parent(&path)?;
        let file = load_file_map(&path)?;
        Ok(Self {
            layers: Mutex::new(Layers {
                memory: HashMap::new(),
                file,
            }),
            persist_path: Some(path),
        })
    }

    /// Describe named references without exposing values.
    ///
    /// # Errors
    ///
    /// [`CredentialError::InvalidRef`] when any name is not a POSIX identifier.
    pub fn describe_refs(
        &self,
        refs: &[String],
    ) -> Result<BTreeMap<String, CredentialView>, CredentialError> {
        let mut out = BTreeMap::new();
        for name in refs {
            let credential = credential_ref(name)?;
            let info = self.describe(&credential)?;
            out.insert(
                name.clone(),
                CredentialView {
                    configured: info.configured,
                    source: info.source,
                    writable: info.writable,
                },
            );
        }
        Ok(out)
    }

    /// Store `value` in the file layer. An empty `value` is [`Self::unset_ref`].
    ///
    /// # Errors
    ///
    /// [`CredentialError::InvalidRef`], [`CredentialError::Shadowed`] when env
    /// currently supplies the reference, or persist I/O failures.
    pub fn set_ref(&self, r#ref: &str, value: &str) -> Result<(), CredentialError> {
        if value.is_empty() {
            return self.unset_ref(r#ref);
        }
        let credential = credential_ref(r#ref)?;
        if env_nonempty(credential.as_str()).is_some() {
            return Err(CredentialError::Shadowed(credential.as_str().to_string()));
        }
        let mut layers = self.lock();
        let key = credential.as_str().to_string();
        let previous = layers.file.insert(key.clone(), value.to_string());
        if let Err(error) = self.persist_file_map(&layers.file) {
            match previous {
                Some(old) => {
                    layers.file.insert(key, old);
                }
                None => {
                    layers.file.remove(&key);
                }
            }
            return Err(error);
        }
        Ok(())
    }

    /// Remove `ref` from the file layer. Absent keys succeed.
    ///
    /// # Errors
    ///
    /// [`CredentialError::InvalidRef`], [`CredentialError::Shadowed`] when env
    /// currently supplies the reference, or persist I/O failures.
    pub fn unset_ref(&self, r#ref: &str) -> Result<(), CredentialError> {
        let credential = credential_ref(r#ref)?;
        if env_nonempty(credential.as_str()).is_some() {
            return Err(CredentialError::Shadowed(credential.as_str().to_string()));
        }
        let mut layers = self.lock();
        let key = credential.as_str().to_string();
        let Some(previous) = layers.file.remove(&key) else {
            return Ok(());
        };
        if let Err(error) = self.persist_file_map(&layers.file) {
            layers.file.insert(key, previous);
            return Err(error);
        }
        Ok(())
    }

    fn persist_file_map(&self, file: &HashMap<String, String>) -> Result<(), CredentialError> {
        let Some(path) = &self.persist_path else {
            return Ok(());
        };
        let map: BTreeMap<&str, &str> = file
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        let text =
            serde_yaml::to_string(&map).map_err(|error| CredentialError::Io(error.to_string()))?;
        write_atomic(path, text.as_bytes())
    }

    fn lock(&self) -> MutexGuard<'_, Layers> {
        self.layers.lock().expect("credentials lock")
    }
}

impl Default for LayeredCredentials {
    fn default() -> Self {
        Self::new()
    }
}

fn nonempty(value: &str) -> Option<&str> {
    if value.is_empty() { None } else { Some(value) }
}

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .and_then(|value| nonempty(&value).map(str::to_string))
}

fn resolve_layers(layers: &Layers, name: &str) -> Option<ResolvedCredential> {
    if let Some(value) = env_nonempty(name) {
        return Some(ResolvedCredential {
            value,
            source: "env".into(),
        });
    }
    if let Some(value) = layers.file.get(name).and_then(|value| nonempty(value)) {
        return Some(ResolvedCredential {
            value: value.to_string(),
            source: "file".into(),
        });
    }
    if let Some(value) = layers.memory.get(name).and_then(|value| nonempty(value)) {
        return Some(ResolvedCredential {
            value: value.to_string(),
            source: "memory".into(),
        });
    }
    None
}

impl CredentialProvider for LayeredCredentials {
    fn resolve(
        &self,
        credential: &CredentialRef,
    ) -> Result<Option<ResolvedCredential>, CredentialError> {
        let layers = self.lock();
        Ok(resolve_layers(&layers, credential.as_str()))
    }

    fn describe(&self, credential: &CredentialRef) -> Result<CredentialInfo, CredentialError> {
        let layers = self.lock();
        let resolved = resolve_layers(&layers, credential.as_str());
        Ok(CredentialInfo {
            configured: resolved.is_some(),
            source: resolved.map(|resolved| resolved.source),
            writable: env_nonempty(credential.as_str()).is_none(),
        })
    }

    fn set(&mut self, credential: &CredentialRef, value: String) -> Result<(), CredentialError> {
        if value.is_empty() {
            return Err(CredentialError::EmptyValue);
        }
        if env_nonempty(credential.as_str()).is_some() {
            return Err(CredentialError::Shadowed(credential.as_str().to_string()));
        }
        self.lock()
            .memory
            .insert(credential.as_str().to_string(), value);
        Ok(())
    }

    fn unset(&mut self, credential: &CredentialRef) -> Result<(), CredentialError> {
        if env_nonempty(credential.as_str()).is_some() {
            return Err(CredentialError::Shadowed(credential.as_str().to_string()));
        }
        self.lock().memory.remove(credential.as_str());
        Ok(())
    }
}

/// Failures from branding a reference, writing a value, or parsing a file map.
#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    /// `value` is not a POSIX identifier.
    #[error("credential ref \"{0}\" must match /^[A-Za-z_][A-Za-z0-9_]*$/")]
    InvalidRef(String),
    /// `set` rejected an empty secret.
    #[error("credentials reject an empty value")]
    EmptyValue,
    /// YAML was not a string-to-string mapping.
    #[error("{0}")]
    InvalidFile(String),
    /// A read-only env layer currently supplies this reference.
    #[error("read-only source shadows \"{0}\"")]
    Shadowed(String),
    /// Filesystem failure while creating the parent directory or writing YAML.
    #[error("credential persist failed: {0}")]
    Io(String),
}

impl CredentialError {
    /// Kebab-case wire code for this failure.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidRef(_) => "bad-request",
            Self::EmptyValue | Self::Shadowed(_) => "credential-rejected",
            Self::InvalidFile(_) | Self::Io(_) => "internal",
        }
    }

    /// Closed RPC error code for this failure.
    #[must_use]
    pub fn rpc_code(&self) -> RpcErrorCode {
        match self {
            Self::InvalidRef(_) => RpcErrorCode::BadRequest,
            Self::EmptyValue | Self::Shadowed(_) => RpcErrorCode::CredentialRejected,
            Self::InvalidFile(_) | Self::Io(_) => RpcErrorCode::Internal,
        }
    }
}

fn load_file_map(path: &Path) -> Result<HashMap<String, String>, CredentialError> {
    match fs::read_to_string(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
        Err(error) => Err(CredentialError::Io(error.to_string())),
        Ok(text) => {
            if text.trim().is_empty() {
                return Ok(HashMap::new());
            }
            let file: BTreeMap<String, String> = serde_yaml::from_str(&text)
                .map_err(|error| CredentialError::InvalidFile(error.to_string()))?;
            Ok(file.into_iter().collect())
        }
    }
}

fn ensure_parent(path: &Path) -> Result<(), CredentialError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }
    fs::create_dir_all(parent).map_err(|error| CredentialError::Io(error.to_string()))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), CredentialError> {
    ensure_parent(path)?;
    let tmp = tmp_path(path)?;
    if let Err(error) = fs::write(&tmp, bytes) {
        let _ = fs::remove_file(&tmp);
        return Err(CredentialError::Io(error.to_string()));
    }
    if let Err(error) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(CredentialError::Io(error.to_string()));
    }
    Ok(())
}

fn tmp_path(path: &Path) -> Result<PathBuf, CredentialError> {
    let Some(name) = path.file_name() else {
        return Err(CredentialError::Io(
            "credential persist path has no file name".into(),
        ));
    };
    let mut tmp_name = name.to_os_string();
    tmp_name.push(format!(".{}.tmp", std::process::id()));
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => Ok(parent.join(tmp_name)),
        _ => Ok(PathBuf::from(tmp_name)),
    }
}

/// Build credentials from `DSH_HOME` when set, otherwise memory-only.
pub(crate) fn credentials_from_home(
    home: Option<&str>,
) -> Result<LayeredCredentials, CredentialError> {
    match home {
        Some(home) if !home.is_empty() => {
            LayeredCredentials::with_persist_path(PathBuf::from(home).join("credentials.yaml"))
        }
        _ => Ok(LayeredCredentials::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::{CredentialError, CredentialProvider, LayeredCredentials, credential_ref};

    #[test]
    fn brands_posix_identifiers() {
        assert_eq!(
            credential_ref("DEEPSEEK_API_KEY").unwrap().as_str(),
            "DEEPSEEK_API_KEY"
        );
        assert_eq!(credential_ref("_private").unwrap().as_str(), "_private");
        assert_eq!(
            credential_ref("lower_case9").unwrap().as_str(),
            "lower_case9"
        );
    }

    #[test]
    fn rejects_non_posix_refs() {
        for invalid in ["", "9LEADING", "WITH-DASH", "WITH SPACE", "ns:key"] {
            let error = credential_ref(invalid).expect_err(invalid);
            assert!(error.to_string().contains("must match"));
        }
    }

    #[test]
    fn brands_via_credential_ref_and_into_inner() {
        let r = credential_ref("DEEPSEEK_API_KEY").unwrap();
        assert_eq!(r.as_str(), "DEEPSEEK_API_KEY");
        assert_eq!(r.into_inner(), "DEEPSEEK_API_KEY");
    }

    #[test]
    fn debug_omits_secret_values() {
        let mut creds = LayeredCredentials::new();
        let r = credential_ref("PHASE3_CRED_DEBUG").unwrap();
        creds.set(&r, "sk-must-not-appear".into()).unwrap();
        let debug = format!("{creds:?}");
        assert!(!debug.contains("sk-must-not-appear"));
        assert!(debug.contains("PHASE3_CRED_DEBUG"));
    }

    #[test]
    fn empty_stored_value_is_absent() {
        let mut creds = LayeredCredentials::new();
        let r = credential_ref("PHASE3_CRED_ABSENT").unwrap();
        creds.set(&r, "sk-live".into()).unwrap();
        let error = creds.set(&r, String::new()).expect_err("empty");
        assert!(error.to_string().contains("empty value"));
        creds.unset(&r).unwrap();
        assert!(creds.resolve(&r).unwrap().is_none());
        let info = creds.describe(&r).unwrap();
        assert!(!info.configured);
        assert!(info.writable);
    }

    #[test]
    fn memory_resolve_and_describe() {
        let mut creds = LayeredCredentials::new();
        let r = credential_ref("PHASE3_CRED_MEMORY").unwrap();
        creds.set(&r, "sk-seeded".into()).unwrap();
        let resolved = creds.resolve(&r).unwrap().expect("present");
        assert_eq!(resolved.value, "sk-seeded");
        assert_eq!(resolved.source, "memory");
        let info = creds.describe(&r).unwrap();
        assert!(info.configured);
        assert_eq!(info.source.as_deref(), Some("memory"));
        assert!(info.writable);
    }

    #[test]
    fn file_wins_over_memory() {
        let mut creds = LayeredCredentials::from_yaml("PHASE3_CRED_FILE: sk-file\n").unwrap();
        let r = credential_ref("PHASE3_CRED_FILE").unwrap();
        creds.set(&r, "sk-memory".into()).unwrap();
        let resolved = creds.resolve(&r).unwrap().expect("file");
        assert_eq!(resolved.value, "sk-file");
        assert_eq!(resolved.source, "file");
    }

    #[test]
    fn with_file_map_resolves_file_layer() {
        let mut map = std::collections::BTreeMap::new();
        map.insert("PHASE3_CRED_MAP".into(), "sk-map".into());
        let creds = LayeredCredentials::with_file_map(map);
        let r = credential_ref("PHASE3_CRED_MAP").unwrap();
        let resolved = creds.resolve(&r).unwrap().expect("file map");
        assert_eq!(resolved.value, "sk-map");
        assert_eq!(resolved.source, "file");
    }

    #[test]
    fn from_yaml_rejects_non_mapping() {
        let error = LayeredCredentials::from_yaml("- not\n- a\n- map\n").expect_err("sequence");
        assert!(matches!(error, CredentialError::InvalidFile(_)));
        let error = LayeredCredentials::from_yaml("just-a-string\n").expect_err("scalar");
        assert!(matches!(error, CredentialError::InvalidFile(_)));
    }

    #[test]
    fn env_wins_over_file_and_memory() {
        let mut creds = LayeredCredentials::from_yaml("PHASE3_CRED_TEST: sk-file\n").unwrap();
        let r = credential_ref("PHASE3_CRED_TEST").unwrap();
        creds.set(&r, "sk-memory".into()).unwrap();
        // SAFETY: test process key used only in this function; restored in the same scope.
        let previous = std::env::var("PHASE3_CRED_TEST").ok();
        unsafe {
            std::env::set_var("PHASE3_CRED_TEST", "sk-env");
        }
        let resolved = creds.resolve(&r).unwrap().expect("env");
        assert_eq!(resolved.value, "sk-env");
        assert_eq!(resolved.source, "env");
        match previous {
            Some(value) => unsafe {
                std::env::set_var("PHASE3_CRED_TEST", value);
            },
            None => unsafe {
                std::env::remove_var("PHASE3_CRED_TEST");
            },
        }
    }

    #[test]
    fn empty_env_is_absent_so_file_shows() {
        let creds = LayeredCredentials::from_yaml("PHASE3_CRED_EMPTY_ENV: sk-file\n").unwrap();
        let r = credential_ref("PHASE3_CRED_EMPTY_ENV").unwrap();
        let previous = std::env::var("PHASE3_CRED_EMPTY_ENV").ok();
        // SAFETY: unique test-only variable; restored before return.
        unsafe {
            std::env::set_var("PHASE3_CRED_EMPTY_ENV", "");
        }
        let resolved = creds.resolve(&r).unwrap().expect("file under empty env");
        assert_eq!(resolved.value, "sk-file");
        assert_eq!(resolved.source, "file");
        match previous {
            Some(value) => unsafe {
                std::env::set_var("PHASE3_CRED_EMPTY_ENV", value);
            },
            None => unsafe {
                std::env::remove_var("PHASE3_CRED_EMPTY_ENV");
            },
        }
    }

    #[test]
    fn env_shadowed_write_is_rejected() {
        let mut creds = LayeredCredentials::new();
        let r = credential_ref("PHASE3_CRED_SHADOW").unwrap();
        let previous = std::env::var("PHASE3_CRED_SHADOW").ok();
        // SAFETY: unique test-only variable; restored before return.
        unsafe {
            std::env::set_var("PHASE3_CRED_SHADOW", "sk-env");
        }
        let set_err = creds.set(&r, "sk-memory".into()).expect_err("shadowed set");
        assert!(set_err.to_string().contains("read-only source shadows"));
        let unset_err = creds.unset(&r).expect_err("shadowed unset");
        assert!(unset_err.to_string().contains("read-only source shadows"));
        let info = creds.describe(&r).unwrap();
        assert!(info.configured);
        assert_eq!(info.source.as_deref(), Some("env"));
        assert!(!info.writable);
        match previous {
            Some(value) => unsafe {
                std::env::set_var("PHASE3_CRED_SHADOW", value);
            },
            None => unsafe {
                std::env::remove_var("PHASE3_CRED_SHADOW");
            },
        }
    }

    #[test]
    fn second_resolve_sees_updated_memory() {
        let mut creds = LayeredCredentials::new();
        let r = credential_ref("PHASE3_CRED_UPDATE").unwrap();
        assert!(creds.resolve(&r).unwrap().is_none());
        creds.set(&r, "sk-one".into()).unwrap();
        assert_eq!(creds.resolve(&r).unwrap().unwrap().value, "sk-one");
        creds.set(&r, "sk-two".into()).unwrap();
        assert_eq!(creds.resolve(&r).unwrap().unwrap().value, "sk-two");
    }

    #[test]
    fn empty_file_entry_is_absent_so_memory_shows() {
        let mut creds = LayeredCredentials::from_yaml("PHASE3_CRED_EMPTY_FILE: \"\"\n").unwrap();
        let r = credential_ref("PHASE3_CRED_EMPTY_FILE").unwrap();
        creds.set(&r, "sk-memory".into()).unwrap();
        let resolved = creds.resolve(&r).unwrap().expect("memory under empty file");
        assert_eq!(resolved.value, "sk-memory");
        assert_eq!(resolved.source, "memory");
    }

    #[test]
    fn persist_set_describe_omits_secret() {
        let dir = std::env::temp_dir().join(format!(
            "cred-persist-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("credentials.yaml");
        let creds = LayeredCredentials::with_persist_path(path.clone()).expect("persist path");
        let secret = "task85-omit-from-describe";
        creds
            .set_ref("TASK85_PERSIST_OMIT", secret)
            .expect("set_ref");
        let map = creds
            .describe_refs(&["TASK85_PERSIST_OMIT".into()])
            .expect("describe_refs");
        let view = map.get("TASK85_PERSIST_OMIT").expect("named ref");
        assert!(view.configured());
        assert_eq!(view.source(), Some("file"));
        assert!(view.writable());
        let view_debug = format!("{view:?}");
        let creds_debug = format!("{creds:?}");
        let map_debug = format!("{map:?}");
        let json = serde_json::json!({
            "configured": view.configured(),
            "source": view.source(),
            "writable": view.writable(),
        })
        .to_string();
        assert!(!view_debug.contains(secret), "CredentialView debug");
        assert!(!creds_debug.contains(secret), "LayeredCredentials debug");
        assert!(!map_debug.contains(secret), "describe_refs debug");
        assert!(!json.contains(secret), "describe_refs json");
        let stored = std::fs::read_to_string(&path).expect("yaml");
        assert!(stored.contains("TASK85_PERSIST_OMIT"));
        assert!(!format!("{creds:?}").contains(secret));
    }

    #[test]
    fn describe_refs_rejects_invalid_posix_name() {
        let creds = LayeredCredentials::new();
        let error = creds
            .describe_refs(&["not-posix".into()])
            .expect_err("invalid");
        assert_eq!(error.code(), "bad-request");
        assert_eq!(error.rpc_code(), dsh_rpc::RpcErrorCode::BadRequest);
    }

    #[test]
    fn set_ref_empty_unsets_file_layer() {
        let dir = std::env::temp_dir().join(format!(
            "cred-unset-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let creds = LayeredCredentials::with_persist_path(dir.join("credentials.yaml")).unwrap();
        creds.set_ref("TASK85_EMPTY_UNSET", "present").unwrap();
        creds.set_ref("TASK85_EMPTY_UNSET", "").unwrap();
        let map = creds.describe_refs(&["TASK85_EMPTY_UNSET".into()]).unwrap();
        assert!(!map["TASK85_EMPTY_UNSET"].configured());
        creds.unset_ref("TASK85_EMPTY_UNSET").unwrap();
    }

    #[test]
    fn unknown_valid_ref_describes_unconfigured() {
        let creds = LayeredCredentials::new();
        let map = creds
            .describe_refs(&["TASK85_UNKNOWN_VALID".into()])
            .unwrap();
        let view = &map["TASK85_UNKNOWN_VALID"];
        assert!(!view.configured());
        assert_eq!(view.source(), None);
        assert!(view.writable());
    }
}
