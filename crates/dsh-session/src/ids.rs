//! Product ids branded in this crate.
//!
//! JSON string serde is implemented on [`dsh_brand::Branded`].
//! `impl Serialize for Branded<LocalTag>` is orphan-illegal.

use dsh_brand::Branded;

/// Tag for [`SessionId`].
pub struct SessionIdTag;
/// Session identity in the store and on disk.
pub type SessionId = Branded<SessionIdTag>;
/// Tag for [`MessageId`].
pub struct MessageIdTag;
/// Stable identity of one model-visible message.
pub type MessageId = Branded<MessageIdTag>;
/// Tag for [`CallId`].
pub struct CallIdTag;
/// Provider-issued tool-call identity.
pub type CallId = Branded<CallIdTag>;

#[cfg(test)]
mod tests {
    use super::{CallId, MessageId, SessionId};
    use serde_json::{Value, json};

    #[test]
    fn session_id_serializes_as_a_json_string() {
        let id = SessionId::new("sess-1");
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
