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
}
