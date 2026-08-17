//! Product ids branded in this crate.
//!
//! Each id is a local newtype around [`Branded`] so string serde lives here.
//! `dsh-brand` does not implement serde.

use dsh_brand::Branded;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Tag for [`WorkspaceId`].
pub struct WorkspaceIdTag;

/// Workspace identity in the registry and on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceId(Branded<WorkspaceIdTag>);

impl WorkspaceId {
    /// Brand `value` as this product id.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(Branded::new(value))
    }

    /// Borrow the inner string.
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

impl Serialize for WorkspaceId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for WorkspaceId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::new)
    }
}

#[cfg(test)]
mod tests {
    use super::WorkspaceId;
    use serde_json::{Value, json};

    #[test]
    fn workspace_id_serializes_as_a_json_string() {
        let id = WorkspaceId::new("wk-1-2");
        assert_eq!(id.as_str(), "wk-1-2");
        assert_eq!(id.clone().into_inner(), "wk-1-2");
        let value = serde_json::to_value(&id).expect("serialize");
        assert_eq!(value, json!("wk-1-2"));
        let back: WorkspaceId = serde_json::from_value(value).expect("deserialize");
        assert_eq!(back.as_str(), "wk-1-2");
    }

    #[test]
    fn workspace_id_rejects_a_json_number() {
        let err = serde_json::from_value::<WorkspaceId>(Value::from(123)).unwrap_err();
        assert!(err.to_string().contains("string") || err.to_string().contains("invalid type"));
    }
}
