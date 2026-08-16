//! Shell request/spec types, POSIX bash executors, and exit-status parse for the DeepSeek Harness Rust host.

mod bash;
mod bash_sandbox;
mod classify;
mod error;
mod render;
mod types;

#[cfg(test)]
mod phase4_exit;

pub use bash::{BashConfig, ENV_OVERRIDES, LocalBashExecutor, ShellProcess, ShellProcessRead};
pub use bash_sandbox::{Confine, SandboxBashExecutor};
pub use classify::{
    RunnerFailureMatch, classify_denial, classify_runner_failure, matches_signature,
};
pub use error::ShellError;
pub use render::{ParsedExitStatus, parse_exit_status};
pub use types::{
    ShellExecRequest, ShellExecSpec, ShellProcessStatus, ShellRunResult, ShellSandboxInfo,
};
