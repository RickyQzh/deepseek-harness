//! User-settings namespaces for the DeepSeek Harness Rust host.

mod error;
pub mod plugin;
mod service;

#[cfg(test)]
mod phase7_exit;

pub use error::SettingsError;
pub use service::{DescribeAll, MutateOp, NamespaceView, SettingsService};

/// Namespace keys this crate exposes to callers.
pub const EXPOSED_NAMESPACES: &[&str] = &["ui-onboarding"];

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{EXPOSED_NAMESPACES, SettingsService};

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
    fn unexposed_namespace_is_settings_not_exposed() {
        let service = SettingsService::with_dir(test_temp_dir("settings-nope"));
        let error = service.describe("nope").expect_err("unexposed");
        assert_eq!(error.code(), "settings-not-exposed");
        assert_eq!(error.details(), json!({ "ns": "nope" }));
    }

    #[test]
    fn stale_expected_revision_is_settings_conflict() {
        let service = SettingsService::with_dir(test_temp_dir("settings-conflict"));
        let ns = EXPOSED_NAMESPACES[0];
        let first = service
            .update(ns, json!({ "welcomeNoticeVersion": "one" }), Some(0))
            .expect("first write");
        assert_eq!(first.revision(), 1);
        let error = service
            .update(ns, json!({ "welcomeNoticeVersion": "two" }), Some(0))
            .expect_err("stale expected");
        assert_eq!(error.code(), "settings-conflict");
        assert_eq!(
            error.details(),
            json!({ "ns": ns, "expected": 0, "actual": 1 })
        );
    }
}
