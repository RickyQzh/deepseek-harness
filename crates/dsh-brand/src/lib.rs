//! Compile-time branded wrappers around owned strings.
//!
//! Owning crates define an empty tag type and alias a product id:
//! `pub struct SessionIdTag; pub type SessionId = Branded<SessionIdTag>;`.
//! This crate does not name product ids.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A string carrying a compile-time brand `B`.
///
/// Two `Branded` values are interchangeable only when their tag types match.
/// Comparison, hashing, display, and `as_str` use the inner string.
/// `Clone` and `Debug` do not require `B` to implement those traits: callers
/// define empty tag structs.
pub struct Branded<B> {
    value: String,
    _brand: PhantomData<fn() -> B>,
}

impl<B> Branded<B> {
    /// Brand `value` as `B`.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            _brand: PhantomData,
        }
    }

    /// Borrow the inner string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// Unwrap the inner string.
    #[must_use]
    pub fn into_inner(self) -> String {
        self.value
    }
}

impl<B> Clone for Branded<B> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
            _brand: PhantomData,
        }
    }
}

impl<B> fmt::Debug for Branded<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Branded")
            .field("value", &self.value)
            .finish()
    }
}

impl<B> AsRef<str> for Branded<B> {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl<B> fmt::Display for Branded<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<B> PartialEq for Branded<B> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<B> Eq for Branded<B> {}

impl<B> PartialOrd for Branded<B> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<B> Ord for Branded<B> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.value.cmp(&other.value)
    }
}

impl<B> Hash for Branded<B> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.value.hash(state);
    }
}

impl<B> Serialize for Branded<B> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de, B> Deserialize<'de> for Branded<B> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::new)
    }
}

#[cfg(test)]
mod tests {
    use super::Branded;

    struct SessionTag;
    struct CallTag;

    #[test]
    fn new_round_trips_the_inner_string() {
        let id = Branded::<SessionTag>::new("sess-1");
        assert_eq!(id.as_str(), "sess-1");
        assert_eq!(id.into_inner(), "sess-1");
    }

    #[test]
    fn same_brand_compares_and_hashes_on_inner_value() {
        let a = Branded::<SessionTag>::new("x");
        let b = Branded::<SessionTag>::new("x");
        let c = Branded::<SessionTag>::new("y");
        assert_eq!(a, b);
        assert_ne!(a, c);
        let mut set = std::collections::HashSet::new();
        set.insert(a.clone());
        assert!(set.contains(&b));
        assert!(!set.contains(&c));
    }

    #[test]
    fn display_and_as_ref_expose_the_inner_string() {
        let id = Branded::<CallTag>::new("call-9");
        assert_eq!(id.to_string(), "call-9");
        assert_eq!(AsRef::<str>::as_ref(&id), "call-9");
    }

    #[test]
    fn serializes_as_a_json_string() {
        let id = Branded::<SessionTag>::new("sess-1");
        let value = serde_json::to_value(&id).expect("serialize");
        assert_eq!(value, serde_json::json!("sess-1"));
        let back: Branded<SessionTag> = serde_json::from_value(value).expect("deserialize");
        assert_eq!(back.as_str(), "sess-1");
    }

    #[test]
    fn rejects_a_json_number() {
        let err = serde_json::from_value::<Branded<SessionTag>>(serde_json::Value::from(123))
            .unwrap_err();
        assert!(err.to_string().contains("string") || err.to_string().contains("invalid type"));
    }

    #[test]
    fn distinct_tag_types_can_hold_the_same_text() {
        let session = Branded::<SessionTag>::new("same");
        let call = Branded::<CallTag>::new("same");
        assert_eq!(session.as_str(), call.as_str());
        fn takes_session(_: &Branded<SessionTag>) {}
        takes_session(&session);
        // takes_session(&call); // must not compile: CallTag is not SessionTag
    }
}
