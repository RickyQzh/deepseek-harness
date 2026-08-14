//! Closed `disabled` predicates.

use serde::{Deserialize, Serialize};

use crate::interpolate::InterpolateEnv;

/// Entry disable flag or closed predicate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Disabled {
    /// Literal enable/disable.
    Flag(bool),
    /// Closed predicate.
    Predicate(DisabledPredicate),
}

impl Default for Disabled {
    fn default() -> Self {
        Self::Flag(false)
    }
}

/// Closed disable predicate. Replaces `!!js process.platform === 'win32'`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DisabledPredicate {
    /// Disabled when `env.platform` equals `platform`.
    Platform {
        /// `linux`, `macos`, or `windows`.
        platform: String,
    },
    /// Disabled when `env.platform` does not equal `not_platform`.
    NotPlatform {
        /// `linux`, `macos`, or `windows`.
        #[serde(rename = "not_platform")]
        not_platform: String,
    },
}

/// Whether this predicate disables the row under `env`.
#[must_use]
pub fn is_disabled(disabled: &Disabled, env: &InterpolateEnv) -> bool {
    match disabled {
        Disabled::Flag(flag) => *flag,
        Disabled::Predicate(DisabledPredicate::Platform { platform }) => env.platform == *platform,
        Disabled::Predicate(DisabledPredicate::NotPlatform { not_platform }) => {
            env.platform != *not_platform
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Disabled, DisabledPredicate, is_disabled};
    use crate::interpolate::InterpolateEnv;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    fn flag_and_platform_predicates() {
        let linux = InterpolateEnv {
            env: BTreeMap::new(),
            cwd: PathBuf::from("/"),
            dsh_home: PathBuf::from("/dsh"),
            platform: "linux".into(),
        };
        let windows = InterpolateEnv {
            platform: "windows".into(),
            ..linux.clone()
        };
        assert!(is_disabled(&Disabled::Flag(true), &linux));
        assert!(!is_disabled(&Disabled::Flag(false), &linux));
        assert!(is_disabled(
            &Disabled::Predicate(DisabledPredicate::Platform {
                platform: "windows".into()
            }),
            &windows
        ));
        assert!(!is_disabled(
            &Disabled::Predicate(DisabledPredicate::Platform {
                platform: "windows".into()
            }),
            &linux
        ));
        assert!(is_disabled(
            &Disabled::Predicate(DisabledPredicate::NotPlatform {
                not_platform: "windows".into()
            }),
            &linux
        ));
    }
}
