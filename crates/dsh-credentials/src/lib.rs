//! Per-request credential references. Consumers re-resolve on every operation.

use std::collections::{BTreeMap, HashMap};

use dsh_brand::Branded;

/// Compile-time brand for a POSIX credential reference.
pub struct CredentialRefTag;

/// POSIX-named credential reference. Local newtype; `dsh-brand` does not name it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CredentialRef(Branded<CredentialRefTag>);

impl CredentialRef {
    /// Brand `value` as a credential reference without POSIX validation.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
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
    /// [`CredentialError::InvalidFile`] when a read-only env layer currently
    /// shadows the reference.
    fn set(&mut self, credential: &CredentialRef, value: String) -> Result<(), CredentialError>;

    /// Remove one reference from the writable (memory) layer.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialError::InvalidFile`] when a read-only env layer
    /// currently shadows the reference.
    fn unset(&mut self, credential: &CredentialRef) -> Result<(), CredentialError>;
}

/// Memory + optional YAML file map + live process environment.
#[derive(Debug)]
pub struct LayeredCredentials {
    memory: HashMap<String, String>,
    file: HashMap<String, String>,
}

impl LayeredCredentials {
    /// Empty memory, no file map.
    #[must_use]
    pub fn new() -> Self {
        Self {
            memory: HashMap::new(),
            file: HashMap::new(),
        }
    }

    /// Use `map` as the file layer with empty memory.
    #[must_use]
    pub fn with_file_map(map: BTreeMap<String, String>) -> Self {
        Self {
            memory: HashMap::new(),
            file: map.into_iter().collect(),
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
}

impl Default for LayeredCredentials {
    fn default() -> Self {
        Self::new()
    }
}

fn nonempty(value: &str) -> Option<&str> {
    if value.is_empty() { None } else { Some(value) }
}

impl CredentialProvider for LayeredCredentials {
    fn resolve(
        &self,
        credential: &CredentialRef,
    ) -> Result<Option<ResolvedCredential>, CredentialError> {
        let name = credential.as_str();
        if let Ok(value) = std::env::var(name) {
            if let Some(value) = nonempty(&value) {
                return Ok(Some(ResolvedCredential {
                    value: value.to_string(),
                    source: "env".into(),
                }));
            }
        }
        if let Some(value) = self.file.get(name).and_then(|v| nonempty(v)) {
            return Ok(Some(ResolvedCredential {
                value: value.to_string(),
                source: "file".into(),
            }));
        }
        if let Some(value) = self.memory.get(name).and_then(|v| nonempty(v)) {
            return Ok(Some(ResolvedCredential {
                value: value.to_string(),
                source: "memory".into(),
            }));
        }
        Ok(None)
    }

    fn describe(&self, credential: &CredentialRef) -> Result<CredentialInfo, CredentialError> {
        let resolved = self.resolve(credential)?;
        let shadowed_by_env = std::env::var(credential.as_str())
            .ok()
            .and_then(|v| nonempty(&v).map(str::to_string))
            .is_some();
        Ok(CredentialInfo {
            configured: resolved.is_some(),
            source: resolved.map(|r| r.source),
            writable: !shadowed_by_env,
        })
    }

    fn set(&mut self, credential: &CredentialRef, value: String) -> Result<(), CredentialError> {
        if value.is_empty() {
            return Err(CredentialError::EmptyValue);
        }
        if !self.describe(credential)?.writable {
            return Err(CredentialError::InvalidFile(format!(
                "read-only source shadows \"{}\"",
                credential.as_str()
            )));
        }
        self.memory.insert(credential.as_str().to_string(), value);
        Ok(())
    }

    fn unset(&mut self, credential: &CredentialRef) -> Result<(), CredentialError> {
        if !self.describe(credential)?.writable {
            return Err(CredentialError::InvalidFile(format!(
                "read-only source shadows \"{}\"",
                credential.as_str()
            )));
        }
        self.memory.remove(credential.as_str());
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
    /// YAML was not a string-to-string mapping, or a write is shadowed by env.
    #[error("{0}")]
    InvalidFile(String),
}

#[cfg(test)]
mod tests {
    use super::{
        CredentialError, CredentialProvider, CredentialRef, LayeredCredentials, credential_ref,
    };

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
    fn brands_via_new_and_into_inner() {
        let r = CredentialRef::new("DEEPSEEK_API_KEY");
        assert_eq!(r.as_str(), "DEEPSEEK_API_KEY");
        assert_eq!(r.into_inner(), "DEEPSEEK_API_KEY");
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
}
