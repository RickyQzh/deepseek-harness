//! Filesystem types and `FS_*` error codes for the Rust host.

mod error;
mod types;

pub use error::{FsError, FsErrorCode};
pub use types::{
    FsEditOutcome, FsEditRequest, FsInfo, FsInfoType, FsObservation, FsTarget, FsTargetKey,
    FsTargetKeyTag, FsVersion, FsVersionTag, FsWriteIntent, FsWriteOutcome,
};
