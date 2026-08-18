//! Background-job Service Definition: branded ids, snapshots, and [`JobRegistry`].
//!
//! This crate is not a YAML plugin name. Load [`dsh-jobs-local`](../dsh-jobs-local) to provide `jobs`.

pub mod brand;
pub mod types;

#[cfg(test)]
mod phase8_pty_exit;

pub use brand::{JobId, JobIdTag};
pub use types::{
    JobError, JobHooks, JobKind, JobOutcome, JobRead, JobRegistry, JobSnapshot, JobStart,
    JobStatus, KillResult,
};
