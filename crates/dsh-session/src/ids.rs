//! Product ids branded in this crate.
//!
//! Each id is a local newtype around [`Branded`] so string serde lives here.
//! `dsh-brand` does not implement serde.

use dsh_brand::Branded;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Tag for [`SessionId`].
pub struct SessionIdTag;

/// Session identity in the store and on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionId(Branded<SessionIdTag>);

/// Tag for [`MessageId`].
pub struct MessageIdTag;

/// Stable identity of one model-visible message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageId(Branded<MessageIdTag>);

/// Tag for [`CallId`].
pub struct CallIdTag;

/// Provider-issued tool-call identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallId(Branded<CallIdTag>);

macro_rules! impl_product_id {
    ($id:ident) => {
        impl $id {
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

        impl Serialize for $id {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $id {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                String::deserialize(deserializer).map(Self::new)
            }
        }
    };
}

impl_product_id!(SessionId);
impl_product_id!(MessageId);
impl_product_id!(CallId);

#[cfg(test)]
mod tests {
    use super::{CallId, MessageId, SessionId};
    use serde_json::{Value, json};

    #[test]
    fn session_id_serializes_as_a_json_string() {
        let id = SessionId::new("sess-1");
        assert_eq!(id.as_str(), "sess-1");
        assert_eq!(id.clone().into_inner(), "sess-1");
        let value = serde_json::to_value(&id).expect("serialize");
        assert_eq!(value, json!("sess-1"));
        let back: SessionId = serde_json::from_value(value).expect("deserialize");
        assert_eq!(back.as_str(), "sess-1");
    }

    #[test]
    fn message_id_and_call_id_are_distinct_brands() {
        let message = MessageId::new("same");
        let call = CallId::new("same");
        assert_eq!(message.as_str(), call.as_str());
        fn takes_message(_: &MessageId) {}
        takes_message(&message);
        // takes_message(&call); // must not compile
    }

    #[test]
    fn session_id_rejects_a_json_number() {
        let err = serde_json::from_value::<SessionId>(Value::from(123)).unwrap_err();
        assert!(err.to_string().contains("string") || err.to_string().contains("invalid type"));
    }
}
