//! Shell request/spec types and exit-status parse for the DeepSeek Harness Rust host.

mod error;
mod render;
mod types;

pub use error::ShellError;
pub use render::{ParsedExitStatus, parse_exit_status};
pub use types::{
    ShellExecRequest, ShellExecSpec, ShellProcess, ShellProcessStatus, ShellRunResult,
    ShellSandboxInfo,
};
