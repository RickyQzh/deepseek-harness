//! Per-owner presence/absence observations that derive write and edit intents.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::{FsError, FsErrorCode, FsObservation, FsTarget, FsVersion, FsWriteIntent};

/// Owner id for [`ObservationGate`]. Distinct ids do not share observations.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct ObservationOwner(pub u64);

/// Per-owner map of the last authoritative observation of each target.
///
/// The map key is `(owner id, target_key)`. [`None`] owner never looks up and
/// never records.
pub struct ObservationGate {
    observed: Mutex<HashMap<(u64, String), FsObservation>>,
}

impl Default for ObservationGate {
    fn default() -> Self {
        Self::new()
    }
}

impl ObservationGate {
    /// Empty gate: every target is unobserved for every owner.
    #[must_use]
    pub fn new() -> Self {
        Self {
            observed: Mutex::new(HashMap::new()),
        }
    }

    fn lock_map(&self) -> std::sync::MutexGuard<'_, HashMap<(u64, String), FsObservation>> {
        self.observed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Record `observation` for `owner` and `target.target_key`.
    ///
    /// `owner: None` records nothing.
    pub fn observe(
        &self,
        target: &FsTarget,
        observation: FsObservation,
        owner: Option<ObservationOwner>,
    ) {
        let Some(owner) = owner else {
            return;
        };
        self.lock_map().insert(
            (owner.0, target.target_key.as_str().to_string()),
            observation,
        );
    }

    /// Write intent from the owner's last observation of `target`.
    ///
    /// No owner, no prior observation, or [`FsObservation::Absent`] yields
    /// [`FsWriteIntent::CreateIfAbsent`]. [`FsObservation::Present`] yields
    /// [`FsWriteIntent::ReplaceIfVersion`] at that version.
    #[must_use]
    pub fn write_intent(
        &self,
        target: &FsTarget,
        owner: Option<ObservationOwner>,
    ) -> FsWriteIntent {
        let Some(owner) = owner else {
            return FsWriteIntent::CreateIfAbsent;
        };
        match self
            .lock_map()
            .get(&(owner.0, target.target_key.as_str().to_string()))
        {
            Some(FsObservation::Present { version }) => FsWriteIntent::ReplaceIfVersion {
                version: version.clone(),
            },
            Some(FsObservation::Absent) | None => FsWriteIntent::CreateIfAbsent,
        }
    }

    /// Edit version from the owner's last observation of `target`.
    ///
    /// # Errors
    ///
    /// [`FsErrorCode::NotObserved`] when `owner` is [`None`] or the target was
    /// never observed: `edit requires reading "{display}" first`.
    /// [`FsErrorCode::NotFound`] when the observation is [`FsObservation::Absent`]:
    /// `cannot edit "{display}": not found`.
    pub fn edit_intent(
        &self,
        target: &FsTarget,
        owner: Option<ObservationOwner>,
    ) -> Result<FsVersion, FsError> {
        let display = &target.display_path;
        let Some(owner) = owner else {
            return Err(not_observed(display));
        };
        match self
            .lock_map()
            .get(&(owner.0, target.target_key.as_str().to_string()))
        {
            None => Err(not_observed(display)),
            Some(FsObservation::Absent) => Err(FsError::new(
                format!("cannot edit \"{display}\": not found"),
                FsErrorCode::NotFound,
            )),
            Some(FsObservation::Present { version }) => Ok(version.clone()),
        }
    }
}

fn not_observed(display: &str) -> FsError {
    FsError::new(
        format!("edit requires reading \"{display}\" first"),
        FsErrorCode::NotObserved,
    )
}

#[cfg(test)]
mod tests {
    use super::{ObservationGate, ObservationOwner};
    use crate::{FsErrorCode, FsObservation, FsTarget, FsTargetKey, FsVersion, FsWriteIntent};

    fn target(path: &str) -> FsTarget {
        FsTarget {
            target_key: FsTargetKey::new(path),
            display_path: path.into(),
        }
    }

    #[test]
    fn edit_without_observation_is_not_observed() {
        let gate = ObservationGate::new();
        let owner = ObservationOwner(1);
        let err = gate
            .edit_intent(&target("/ws/a.txt"), Some(owner))
            .unwrap_err();
        assert_eq!(err.code, FsErrorCode::NotObserved);
        assert_eq!(err.message, "edit requires reading \"/ws/a.txt\" first");
    }

    #[test]
    fn write_unobserved_is_create_if_absent() {
        let gate = ObservationGate::new();
        let owner = ObservationOwner(1);
        assert_eq!(
            gate.write_intent(&target("/ws/a.txt"), Some(owner)),
            FsWriteIntent::CreateIfAbsent
        );
        gate.observe(
            &target("/ws/a.txt"),
            FsObservation::Present {
                version: FsVersion::new("v7"),
            },
            Some(owner),
        );
        assert_eq!(
            gate.write_intent(&target("/ws/a.txt"), Some(owner)),
            FsWriteIntent::ReplaceIfVersion {
                version: FsVersion::new("v7")
            }
        );
    }

    #[test]
    fn owners_do_not_share_observations() {
        let gate = ObservationGate::new();
        gate.observe(
            &target("/ws/a.txt"),
            FsObservation::Present {
                version: FsVersion::new("v1"),
            },
            Some(ObservationOwner(1)),
        );
        let err = gate
            .edit_intent(&target("/ws/a.txt"), Some(ObservationOwner(2)))
            .unwrap_err();
        assert_eq!(err.code, FsErrorCode::NotObserved);
    }

    #[test]
    fn absent_observation_creates_and_rejects_edit() {
        let gate = ObservationGate::new();
        let owner = ObservationOwner(1);
        gate.observe(&target("/ws/a.txt"), FsObservation::Absent, Some(owner));
        assert_eq!(
            gate.write_intent(&target("/ws/a.txt"), Some(owner)),
            FsWriteIntent::CreateIfAbsent
        );
        let err = gate
            .edit_intent(&target("/ws/a.txt"), Some(owner))
            .unwrap_err();
        assert_eq!(err.code, FsErrorCode::NotFound);
        assert_eq!(err.message, "cannot edit \"/ws/a.txt\": not found");
    }

    #[test]
    fn no_owner_never_looks_up() {
        let gate = ObservationGate::new();
        gate.observe(
            &target("/ws/a.txt"),
            FsObservation::Present {
                version: FsVersion::new("v1"),
            },
            None,
        );
        assert_eq!(
            gate.write_intent(&target("/ws/a.txt"), None),
            FsWriteIntent::CreateIfAbsent
        );
        let err = gate.edit_intent(&target("/ws/a.txt"), None).unwrap_err();
        assert_eq!(err.code, FsErrorCode::NotObserved);
        assert_eq!(err.message, "edit requires reading \"/ws/a.txt\" first");
        let err = gate
            .edit_intent(&target("/ws/a.txt"), Some(ObservationOwner(1)))
            .unwrap_err();
        assert_eq!(err.code, FsErrorCode::NotObserved);
    }

    #[test]
    fn present_observation_returns_version_for_edit() {
        let gate = ObservationGate::new();
        let owner = ObservationOwner(9);
        gate.observe(
            &target("/ws/a.txt"),
            FsObservation::Present {
                version: FsVersion::new("v3"),
            },
            Some(owner),
        );
        assert_eq!(
            gate.edit_intent(&target("/ws/a.txt"), Some(owner))
                .unwrap()
                .as_str(),
            "v3"
        );
    }
}
