//! Fully specified argv spawn, credential-scrubbed child environments, and POSIX process trees.

mod env;
mod error;
mod types;

pub use env::{DSH_ENV_PREFIX, EnvEntry, child_env, scrubbed_parent_env, sensitive_env_name};
pub use error::{MAX_GRACE_MS, SubprocessError};
pub use types::{
    CollectedOutput, SubprocessCollect, SubprocessOutcome, SubprocessOutput, SubprocessOutputRead,
    SubprocessSpawnSpec, SubprocessStdin, SubprocessStdio,
};
