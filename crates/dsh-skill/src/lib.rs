//! Skill provider registry, local filesystem provider, and `skill` tool for the Rust host.

mod filesystem;
pub mod plugin;
mod registry;
mod tool;

#[cfg(test)]
mod phase6_exit;

pub use filesystem::{FilesystemSkillConfig, FilesystemSkillProvider};
pub use plugin::{register_skill, register_skill_filesystem, register_tool_skill};
pub use registry::{
    BUNDLED_SKILL_RANK, SkillCandidate, SkillDefinition, SkillError, SkillInvocationPolicy,
    SkillProvider, SkillRegistry, SkillSummary, is_skill_name, render_skill_content,
};
