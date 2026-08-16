//! Skill provider registry: rank, registration order, then name-sorted summaries.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Standard precedence rank for packaged skill providers and local bundled roots.
pub const BUNDLED_SKILL_RANK: i32 = 600;

/// Registry and provider failures.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SkillError {
    /// A provider with this name is already registered.
    #[error("a skill provider named \"{0}\" is already registered")]
    DuplicateProvider(String),
    /// No winning skill exists for this name.
    #[error("skill \"{0}\" is unknown or no longer available")]
    Unknown(String),
    /// The name is not kebab-case skill grammar.
    #[error("invalid skill name \"{0}\"")]
    InvalidName(String),
    /// The skill exists but is not model-invocable.
    #[error("skill \"{0}\" is not available for model invocation")]
    NotModelInvocable(String),
    /// Provider load or configuration failure.
    #[error("{0}")]
    Other(String),
}

impl SkillError {
    /// Unknown or unloaded skill name.
    #[must_use]
    pub fn unknown(name: impl Into<String>) -> Self {
        Self::Unknown(name.into())
    }
}

/// Return whether `name` matches `/^[a-z0-9]+(?:-[a-z0-9]+)*$/`.
#[must_use]
pub fn is_skill_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() || !is_name_alnum(bytes[0]) {
        return false;
    }
    let mut i = 1;
    while i < bytes.len() {
        if bytes[i] == b'-' {
            i += 1;
            if i >= bytes.len() || !is_name_alnum(bytes[i]) {
                return false;
            }
            i += 1;
            while i < bytes.len() && is_name_alnum(bytes[i]) {
                i += 1;
            }
        } else if is_name_alnum(bytes[i]) {
            i += 1;
        } else {
            return false;
        }
    }
    true
}

fn is_name_alnum(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit()
}

/// Independent model and user invocation controls.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillInvocationPolicy {
    /// Whether model-facing catalogs and loaders include this skill.
    pub model_invocable: bool,
    /// Whether human-facing command catalogs and loaders include this skill.
    pub user_invocable: bool,
}

impl Default for SkillInvocationPolicy {
    fn default() -> Self {
        Self {
            model_invocable: true,
            user_invocable: true,
        }
    }
}

/// Invocation-neutral skill metadata returned by [`SkillRegistry::list`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillSummary {
    /// Kebab-case identifier used to address the skill.
    pub name: String,
    /// Short routing description shown by discovery consumers.
    pub description: String,
    /// Optional extra routing guidance.
    pub when_to_use: Option<String>,
    /// Resolved model and user invocation controls.
    pub invocation: SkillInvocationPolicy,
    /// Discovery source that produced this winning skill.
    pub source: String,
    /// Provider that owns this skill body.
    pub provider: String,
}

/// Complete parsed skill definition, including the body loaded by [`SkillRegistry::get`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillDefinition {
    /// Winning summary for this definition.
    pub summary: SkillSummary,
    /// Markdown instruction body after frontmatter removal.
    pub content: String,
    /// Absolute file path when the skill came from disk.
    pub path: Option<PathBuf>,
}

/// Provider catalog entry used by the registry to merge and later load skills.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillCandidate {
    /// Invocation-neutral metadata.
    pub summary: SkillSummary,
    /// Lower ranks win duplicate skill names before provider registration order.
    pub rank: i32,
    /// Opaque provider-owned handle passed back to [`SkillProvider::get`].
    pub locator: String,
    /// Absolute file path when the provider has one.
    pub path: Option<PathBuf>,
}

/// One source of skills, such as local directories.
pub trait SkillProvider: Send + Sync {
    /// Unique provider name in the registry.
    fn name(&self) -> &str;
    /// List available skill candidates for the current lookup context.
    fn list(&self, cwd: Option<&str>) -> Vec<SkillCandidate>;
    /// Load a complete skill body for a previously listed locator.
    ///
    /// # Errors
    ///
    /// [`SkillError`] when the locator is unknown or the body cannot be loaded.
    fn get(&self, locator: &str) -> Result<SkillDefinition, SkillError>;
}

struct IndexedCandidate {
    order: usize,
    candidate: SkillCandidate,
    provider: Arc<dyn SkillProvider>,
}

/// Host-global registry of skill providers.
pub struct SkillRegistry {
    providers: Mutex<Vec<Arc<dyn SkillProvider>>>,
}

impl SkillRegistry {
    /// Empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            providers: Mutex::new(Vec::new()),
        }
    }

    /// Register `provider`. Duplicate [`SkillProvider::name`] values fail.
    ///
    /// # Errors
    ///
    /// [`SkillError::DuplicateProvider`] when a provider with that name is already registered.
    pub fn register_provider(&self, provider: Arc<dyn SkillProvider>) -> Result<(), SkillError> {
        let mut providers = self
            .providers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if providers
            .iter()
            .any(|existing| existing.name() == provider.name())
        {
            return Err(SkillError::DuplicateProvider(provider.name().to_string()));
        }
        providers.push(provider);
        Ok(())
    }

    /// Winning summaries: lower rank, then registration order, then sort by name.
    #[must_use]
    pub fn list(&self, cwd: Option<&str>) -> Vec<SkillSummary> {
        let mut summaries: Vec<SkillSummary> = self
            .winning(cwd)
            .into_iter()
            .map(|entry| entry.candidate.summary)
            .collect();
        summaries.sort_by(|left, right| left.name.cmp(&right.name));
        summaries
    }

    /// Load the winning definition for `name`.
    ///
    /// # Errors
    ///
    /// [`SkillError::InvalidName`], [`SkillError::Unknown`], or a provider load failure.
    pub fn get(&self, name: &str, cwd: Option<&str>) -> Result<SkillDefinition, SkillError> {
        if !is_skill_name(name) {
            return Err(SkillError::InvalidName(name.to_string()));
        }
        let winner = self
            .winning(cwd)
            .into_iter()
            .find(|entry| entry.candidate.summary.name == name);
        match winner {
            Some(entry) => entry.provider.get(&entry.candidate.locator),
            None => Err(SkillError::unknown(name)),
        }
    }

    fn winning(&self, cwd: Option<&str>) -> Vec<IndexedCandidate> {
        let providers: Vec<Arc<dyn SkillProvider>> = self
            .providers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let mut indexed = Vec::new();
        for (order, provider) in providers.iter().enumerate() {
            for candidate in provider.list(cwd) {
                indexed.push(IndexedCandidate {
                    order,
                    candidate,
                    provider: Arc::clone(provider),
                });
            }
        }
        indexed.sort_by(|left, right| {
            left.candidate
                .rank
                .cmp(&right.candidate.rank)
                .then(left.order.cmp(&right.order))
        });
        let mut seen = HashSet::new();
        let mut winners = Vec::new();
        for entry in indexed {
            if seen.insert(entry.candidate.summary.name.clone()) {
                winners.push(entry);
            }
        }
        winners
    }
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Render one loaded skill as the canonical `<skill_content>` block.
///
/// The name rides an escaped attribute; the body is embedded verbatim. This phase
/// has no `resourceBase`, so the hint names the provider only.
#[must_use]
pub fn render_skill_content(name: &str, provider: &str, content: &str) -> String {
    let resource_hint = [
        format!(
            "Resources for this skill are managed by provider \"{}\".",
            escape_text(provider)
        ),
        "Load referenced resources only as needed.".to_string(),
    ];
    [
        format!("<skill_content name=\"{}\">", escape_attr(name)),
        "<skill_resources>".to_string(),
        resource_hint[0].clone(),
        resource_hint[1].clone(),
        "</skill_resources>".to_string(),
        String::new(),
        "<skill_instructions>".to_string(),
        content.to_string(),
        "</skill_instructions>".to_string(),
        "</skill_content>".to_string(),
    ]
    .join("\n")
}

/// Escape a skill name used as an HTML attribute (`&`, `"`, `<`).
#[must_use]
pub fn escape_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}

/// Escape model-facing prose so provider text cannot open or close framing tags.
#[must_use]
pub fn escape_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StaticProvider {
        name: String,
        rank: i32,
        skill_name: String,
        description: String,
    }

    impl StaticProvider {
        fn new(name: &str, rank: i32, skill_name: &str, description: &str) -> Self {
            Self {
                name: name.into(),
                rank,
                skill_name: skill_name.into(),
                description: description.into(),
            }
        }
    }

    impl SkillProvider for StaticProvider {
        fn name(&self) -> &str {
            &self.name
        }

        fn list(&self, _cwd: Option<&str>) -> Vec<SkillCandidate> {
            vec![SkillCandidate {
                summary: SkillSummary {
                    name: self.skill_name.clone(),
                    description: self.description.clone(),
                    when_to_use: None,
                    invocation: SkillInvocationPolicy {
                        model_invocable: true,
                        user_invocable: true,
                    },
                    source: "test".into(),
                    provider: self.name.clone(),
                },
                rank: self.rank,
                locator: self.skill_name.clone(),
                path: None,
            }]
        }

        fn get(&self, locator: &str) -> Result<SkillDefinition, SkillError> {
            if locator != self.skill_name {
                return Err(SkillError::unknown(locator));
            }
            Ok(SkillDefinition {
                summary: self.list(None).into_iter().next().unwrap().summary,
                content: self.description.clone(),
                path: None,
            })
        }
    }

    #[test]
    #[allow(unused_mut)]
    fn list_picks_lower_rank_then_sorts_by_name() {
        let mut reg = SkillRegistry::new();
        reg.register_provider(Arc::new(StaticProvider::new("a", 200, "dup", "lose")))
            .unwrap();
        reg.register_provider(Arc::new(StaticProvider::new("b", 100, "dup", "win")))
            .unwrap();
        let list = reg.list(None);
        assert_eq!(
            list.iter().find(|s| s.name == "dup").unwrap().description,
            "win"
        );
    }
}
