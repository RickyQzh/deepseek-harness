//! Local [`JobId`] newtype around [`dsh_brand::Branded`].
//!
//! `dsh-brand` does not implement serde; this id is compared and hashed on the inner string.

use std::fmt;

use dsh_brand::Branded;

/// Tag for [`JobId`].
pub struct JobIdTag;

/// Registry-issued background job id (`<kind>-N`).
///
/// Ids are predictable; owner authorization is the access boundary.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct JobId(Branded<JobIdTag>);

impl JobId {
    /// Brand `value` as a job id. No format check is performed.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(Branded::new(value))
    }

    /// Borrow the inner string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for JobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl AsRef<str> for JobId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::JobId;

    #[test]
    fn new_round_trips_the_inner_string() {
        let id = JobId::new("bash-1");
        assert_eq!(id.as_str(), "bash-1");
        assert_eq!(id.to_string(), "bash-1");
        assert_eq!(AsRef::<str>::as_ref(&id), "bash-1");
    }
}
