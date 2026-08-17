//! Fully specified argv spawn, credential-scrubbed child environments, POSIX process trees, and POSIX PTY allocation.

mod collect;
mod env;
mod error;
pub mod plugin;
mod runtime;
mod spawn;
mod terminal;
mod types;

#[cfg(test)]
mod phase4_exit;

pub use collect::OutputCollector;
pub use env::{DSH_ENV_PREFIX, EnvEntry, child_env, scrubbed_parent_env, sensitive_env_name};
pub use error::{MAX_GRACE_MS, SubprocessError};
pub use runtime::LocalSubprocessRuntime;
pub use spawn::{SubprocessHandle, kill_group, spawn_subprocess, spawn_subprocess_with_spill_dir};
pub use terminal::{SubprocessTerminalHandle, SubprocessTerminalSpawnSpec, spawn_terminal};
pub use types::{
    CollectedOutput, SubprocessCollect, SubprocessOutcome, SubprocessOutput, SubprocessOutputRead,
    SubprocessSpawnSpec, SubprocessStdin, SubprocessStdio,
};
