//! Prompt assembly errors.

/// Prompt assembly, registration, or interpolation failure.
#[derive(Debug, thiserror::Error)]
pub enum PromptError {
    /// Invalid configuration, registration, or interpolation.
    #[error("{0}")]
    Invalid(String),
}
