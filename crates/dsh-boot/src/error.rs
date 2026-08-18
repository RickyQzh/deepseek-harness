//! Boot errors.

/// Failure to parse, interpolate, look up, or mount a compose row.
#[derive(Debug, thiserror::Error)]
pub enum BootError {
    /// YAML `name` is not in the closed registry.
    #[error("unknown plugin name `{name}`")]
    UnknownPlugin {
        /// YAML plugin name that was not registered.
        name: String,
    },
    /// Compose parse, patch, `!!js`, or interpolation failure.
    #[error(transparent)]
    Compose(#[from] dsh_compose::ComposeError),
    /// Kernel setup failure after the row was spawned.
    #[error(transparent)]
    Kernel(#[from] dsh_kernel::KernelError),
}
