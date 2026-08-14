//! Kernel errors.

/// Framework error with a stable discriminant.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum KernelError {
    /// An effect or plugin was requested on a fiber that cannot accept work.
    #[error("cannot create effect on inactive context")]
    InactiveEffect,
    /// Plugin `setup` returned this message.
    #[error("plugin setup failed: {0}")]
    SetupFailed(String),
}
