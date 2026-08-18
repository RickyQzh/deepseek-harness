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
    /// `provide` used a name already occupied in the same realm.
    #[error("service `{name}` is already provided in this realm")]
    ServiceAlreadyProvided {
        /// Service name that collided.
        name: String,
    },
    /// `get`/`inject` found a value that is not `T`.
    #[error("service `{name}` has a different type than requested")]
    ServiceTypeMismatch {
        /// Service name that was requested.
        name: String,
    },
    /// The waiting fiber ended before the named service appeared.
    #[error("fiber disposed before service `{name}` was provided")]
    InjectWaitDisposed {
        /// Service name that was awaited.
        name: String,
    },
    /// A preset-owned fiber provided a service into the root realm.
    #[error("preset row `{plugin}` provides `{service}` into the root realm")]
    PresetProvidesIntoRoot {
        /// Preset row id.
        plugin: String,
        /// Service name that was offered to the root realm.
        service: String,
    },
}
