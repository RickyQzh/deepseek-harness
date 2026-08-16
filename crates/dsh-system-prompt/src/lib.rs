//! Ordered system-prompt sections, dynamic context, tools, and strict `{{var}}` interpolation.

mod assemble;
mod error;
mod interpolate;
pub mod plugin;

pub use assemble::{
    AssembleContext, AssembledContext, AssembledSection, CONTEXT_SNAPSHOT_PREFIX,
    ContextSnapshotSection, HARNESS_IDENTITY_ORDER, HARNESS_IDENTITY_SECTION,
    HARNESS_IDENTITY_TEXT, PERSONA_ORDER, PERSONA_SECTION, PromptAssembly, PromptContext,
    PromptSection, SectionText, SystemPrompt, SystemPromptConfig, TOOL_ORDER_REST,
    join_context_sections, render_context_sections, render_context_snapshot, render_prompt,
};
pub use dsh_llm::ToolSchema;
pub use error::PromptError;
pub use interpolate::interpolate;
