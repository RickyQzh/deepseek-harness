//! Shared helpers for subagent tool plugins.

use std::time::{SystemTime, UNIX_EPOCH};

use dsh_session::MessageId;

pub(crate) fn setup_err(message: impl Into<String>) -> dsh_kernel::KernelError {
    dsh_kernel::KernelError::SetupFailed(message.into())
}

pub(crate) fn mint_message_id(prefix: &str) -> MessageId {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    MessageId::new(format!("{prefix}-{nanos}"))
}

pub(crate) fn reject_unknown_keys(
    value: &serde_json::Value,
    allowed: &[&str],
    subject: &str,
) -> Result<(), String> {
    match value {
        serde_json::Value::Null => Ok(()),
        serde_json::Value::Object(map) => {
            for key in map.keys() {
                if !allowed.contains(&key.as_str()) {
                    return Err(format!("{subject}: unknown key \"{key}\""));
                }
            }
            Ok(())
        }
        _ => Err(format!("{subject}: config must be an object")),
    }
}
