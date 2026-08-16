//! Shell request/spec types, POSIX bash executor, and exit-status parse for the DeepSeek Harness Rust host.

mod bash;
mod error;
mod render;
mod types;

pub use bash::{BashConfig, ENV_OVERRIDES, LocalBashExecutor, ShellProcess, ShellProcessRead};
pub use error::ShellError;
pub use render::{ParsedExitStatus, parse_exit_status};
pub use types::{
    ShellExecRequest, ShellExecSpec, ShellProcessStatus, ShellRunResult, ShellSandboxInfo,
};
