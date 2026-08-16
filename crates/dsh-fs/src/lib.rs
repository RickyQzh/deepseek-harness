//! Filesystem types, `FS_*` error codes, local UTF-8 backend, in-process sandbox fence, and per-owner observation policy for the Rust host.

mod containment;
mod error;
mod fsio;
mod local;
mod observation;
pub mod plugin;
mod sandbox;
mod types;

#[cfg(test)]
mod phase4_exit;

pub use containment::is_path_under;
pub use error::{FsError, FsErrorCode};
pub use local::LocalFileSystem;
pub use observation::{ObservationGate, ObservationOwner};
pub use sandbox::{SandboxFence, checked_target};
pub use types::{
    FsEditOutcome, FsEditRequest, FsInfo, FsInfoType, FsObservation, FsTarget, FsTargetKey,
    FsTargetKeyTag, FsVersion, FsVersionTag, FsWriteIntent, FsWriteOutcome,
};
