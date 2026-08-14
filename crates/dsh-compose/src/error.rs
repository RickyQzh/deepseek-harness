//! Compose errors.

/// Fail-loud compose failure.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ComposeError {
    /// YAML contained a `!!js` tag.
    #[error(
        "`!!js` is not supported in the Rust compose dialect; use closed interpolators or a disabled predicate"
    )]
    JsTagNotSupported,
    /// An interpolator or patch id did not resolve.
    #[error("missing referent `{referent}`")]
    MissingReferent {
        /// Name that did not resolve (`env:VAR`, patch id, or `dshHome:`).
        referent: String,
    },
    /// `${...}` was not one of the closed interpolators.
    #[error("unknown interpolator `${{{expr}}}`")]
    UnknownInterpolator {
        /// Text between `${` and `}`.
        expr: String,
    },
    /// YAML parse or dump failure.
    #[error("invalid YAML: {0}")]
    InvalidYaml(String),
    /// Patch `name` did not match the target entry.
    #[error("patch name mismatch for `{id}` (expected `{expected}`, got `{actual}`)")]
    NameMismatch {
        /// Target id.
        id: String,
        /// `name` on the existing entry.
        expected: String,
        /// `name` on the patch.
        actual: String,
    },
    /// `insert` targeted an id that is not a group.
    #[error("patch insert: entry `{id}` is not a group")]
    NotAGroup {
        /// Target id.
        id: String,
    },
    /// Entry config failed its registered schema.
    #[error("invalid config for `{name}`")]
    InvalidConfig {
        /// Entry id, when present.
        id: Option<String>,
        /// Plugin name.
        name: String,
        /// Schema failure.
        source: dsh_schema::SchemaError,
    },
}
