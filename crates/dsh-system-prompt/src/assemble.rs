//! Ordered prompt assembly, tool order, and context-snapshot helpers.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::PromptError;
use crate::interpolate::{interpolate, is_variable_name};

pub use dsh_llm::ToolSchema;

/// Deployment persona section name.
pub const PERSONA_SECTION: &str = "deployment:persona";
/// Prompt order of the persona slot.
pub const PERSONA_ORDER: i32 = 0;
/// Reserved [`SystemPromptConfig::tool_order`] marker for unlisted tools.
pub const TOOL_ORDER_REST: &str = "<unlisted-tools>";
/// Harness identity section name.
pub const HARNESS_IDENTITY_SECTION: &str = "harness:identity";
/// Prompt order of the harness identity slot.
pub const HARNESS_IDENTITY_ORDER: i32 = -100;
/// Fixed DeepSeek Harness identity prose.
pub const HARNESS_IDENTITY_TEXT: &str = "You are an AI agent powered by DeepSeek Harness.";
/// Model-facing prefix for a non-empty runtime-context snapshot.
pub const CONTEXT_SNAPSHOT_PREFIX: &str =
    "Current runtime context. This snapshot supersedes earlier runtime-context snapshots.\n\n";

/// Per-assembly context passed to dynamic providers and assemble listeners.
#[derive(Clone, Debug, Default)]
pub struct AssembleContext {}

/// One resolved system-prompt section, before interpolation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssembledSection {
    /// Contributing section name.
    pub name: String,
    /// Resolved section text.
    pub text: String,
}

/// One resolved runtime-context contribution, before interpolation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssembledContext {
    /// Contributing context name.
    pub name: String,
    /// Resolved context text.
    pub text: String,
}

/// Assembled sections, contexts, tools, and variables for one render.
#[derive(Clone, Debug, PartialEq)]
pub struct PromptAssembly {
    /// Ordered system-prompt sections.
    pub sections: Vec<AssembledSection>,
    /// Ordered runtime-context contributions.
    pub contexts: Vec<AssembledContext>,
    /// Tools in configured or lexicographic order.
    pub tools: Vec<ToolSchema>,
    /// Registered variables; `None` means defined but unset for this assembly.
    pub variables: BTreeMap<String, Option<String>>,
}

/// Registry input for one system-prompt section.
#[derive(Clone, Debug)]
pub struct PromptSection {
    /// Unique section name.
    pub name: String,
    /// Ascending concatenation order.
    pub order: i32,
    /// Static text or a provider evaluated at assemble.
    pub text: SectionText,
    /// When true, this section is restored as the sole prompt section after listeners.
    pub complete: bool,
}

/// Static text or a function evaluated with the current [`AssembleContext`].
#[derive(Clone, Debug)]
pub enum SectionText {
    /// Fixed prose, possibly containing `{{var}}` references.
    Static(String),
    /// Provider evaluated once per assemble.
    Dynamic(fn(&AssembleContext) -> String),
}

/// Registry input for one runtime-context contribution.
#[derive(Clone, Debug)]
pub struct PromptContext {
    /// Unique context name.
    pub name: String,
    /// Ascending join order.
    pub order: i32,
    /// Static text or a provider evaluated at assemble.
    pub text: SectionText,
}

/// Deployment-authored fragment of the system prompt.
#[derive(Clone, Debug)]
pub struct SystemPromptConfig {
    /// Include the fixed harness identity before the persona (default true).
    pub include_harness_identity: bool,
    /// Include dynamic runtime-context snapshots (default true).
    pub include_runtime_context: bool,
    /// Order-0 persona template.
    pub persona: String,
    /// Model-facing tool names in order, with [`TOOL_ORDER_REST`] exactly once.
    pub tool_order: Option<Vec<String>>,
}

impl Default for SystemPromptConfig {
    fn default() -> Self {
        Self {
            include_harness_identity: true,
            include_runtime_context: true,
            persona: String::new(),
            tool_order: None,
        }
    }
}

/// One named contribution to a rendered runtime-context snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextSnapshotSection {
    /// Contributing context name.
    pub name: String,
    /// Interpolated model-facing text.
    pub text: String,
}

/// Interpolate each section, drop empty strings, and join with blank lines.
///
/// # Errors
///
/// Returns interpolation failures from any section.
pub fn render_prompt(assembly: &PromptAssembly) -> Result<String, PromptError> {
    let mut parts = Vec::new();
    for section in &assembly.sections {
        let text = interpolate(&section.name, &section.text, &assembly.variables, "section")?;
        if !text.is_empty() {
            parts.push(text);
        }
    }
    Ok(parts.join("\n\n"))
}

/// Interpolate each context and keep contributions that render to non-empty text.
///
/// # Errors
///
/// Returns interpolation failures from any context.
pub fn render_context_sections(
    assembly: &PromptAssembly,
) -> Result<Vec<ContextSnapshotSection>, PromptError> {
    let mut sections = Vec::new();
    for context in &assembly.contexts {
        let text = interpolate(&context.name, &context.text, &assembly.variables, "context")?;
        if !text.is_empty() {
            sections.push(ContextSnapshotSection {
                name: context.name.clone(),
                text,
            });
        }
    }
    Ok(sections)
}

/// Join already-rendered snapshot sections.
///
/// An empty body is `""`; otherwise the result is [`CONTEXT_SNAPSHOT_PREFIX`] plus the body.
#[must_use]
pub fn join_context_sections(sections: &[ContextSnapshotSection]) -> String {
    let body = sections
        .iter()
        .map(|section| section.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    if body.is_empty() {
        String::new()
    } else {
        format!("{CONTEXT_SNAPSHOT_PREFIX}{body}")
    }
}

/// Render the complete dynamic context snapshot.
///
/// # Errors
///
/// Returns interpolation failures from any context.
pub fn render_context_snapshot(assembly: &PromptAssembly) -> Result<String, PromptError> {
    Ok(join_context_sections(&render_context_sections(assembly)?))
}

/// Registry for ordered sections, contexts, tools, variables, and assemble listeners.
#[derive(Clone)]
pub struct SystemPrompt {
    sections: Vec<PromptSection>,
    contexts: Vec<PromptContext>,
    variables: BTreeMap<String, fn(&AssembleContext) -> Option<String>>,
    tool_providers: Vec<fn(&AssembleContext) -> Vec<ToolSchema>>,
    assemble_listeners: Vec<fn(PromptAssembly, &AssembleContext) -> PromptAssembly>,
    tool_order: Option<Vec<String>>,
    runtime_context_suppressed: bool,
}

impl SystemPrompt {
    /// Register harness identity (when configured), the persona, and optional context suppression.
    ///
    /// # Errors
    ///
    /// Returns when `tool_order` lists a name more than once or omits [`TOOL_ORDER_REST`].
    pub fn new(config: SystemPromptConfig) -> Result<Self, PromptError> {
        let tool_order = validate_tool_order(config.tool_order)?;
        let mut prompt = Self {
            sections: Vec::new(),
            contexts: Vec::new(),
            variables: BTreeMap::new(),
            tool_providers: Vec::new(),
            assemble_listeners: Vec::new(),
            tool_order,
            runtime_context_suppressed: false,
        };
        if config.include_harness_identity {
            prompt.section(PromptSection {
                name: HARNESS_IDENTITY_SECTION.to_string(),
                order: HARNESS_IDENTITY_ORDER,
                text: SectionText::Static(HARNESS_IDENTITY_TEXT.to_string()),
                complete: false,
            })?;
        }
        prompt.section(PromptSection {
            name: PERSONA_SECTION.to_string(),
            order: PERSONA_ORDER,
            text: SectionText::Static(config.persona),
            complete: false,
        })?;
        if !config.include_runtime_context {
            prompt.suppress_runtime_context();
        }
        Ok(prompt)
    }

    /// Register an ordered prompt section. Duplicate names fail.
    ///
    /// # Errors
    ///
    /// Returns when a section with the same name is already registered.
    pub fn section(&mut self, section: PromptSection) -> Result<(), PromptError> {
        if self
            .sections
            .iter()
            .any(|existing| existing.name == section.name)
        {
            return Err(PromptError::Invalid(format!(
                "prompt section \"{}\" is already registered",
                section.name
            )));
        }
        self.sections.push(section);
        Ok(())
    }

    /// Register ordered dynamic context. Duplicate names fail.
    ///
    /// # Errors
    ///
    /// Returns when a context with the same name is already registered.
    pub fn context(&mut self, context: PromptContext) -> Result<(), PromptError> {
        if self
            .contexts
            .iter()
            .any(|existing| existing.name == context.name)
        {
            return Err(PromptError::Invalid(format!(
                "prompt context \"{}\" is already registered",
                context.name
            )));
        }
        self.contexts.push(context);
        Ok(())
    }

    /// Drop every runtime-context contribution from later assemblies.
    pub fn suppress_runtime_context(&mut self) {
        self.runtime_context_suppressed = true;
    }

    /// Register a tool-schema provider evaluated at each assemble.
    pub fn tools(&mut self, provider: fn(&AssembleContext) -> Vec<ToolSchema>) {
        self.tool_providers.push(provider);
    }

    /// Register a prompt variable. Duplicate or invalid names fail.
    ///
    /// # Errors
    ///
    /// Returns when `name` fails `^[a-z][a-z0-9_]*$` or is already registered.
    pub fn variable(
        &mut self,
        name: &str,
        provider: fn(&AssembleContext) -> Option<String>,
    ) -> Result<(), PromptError> {
        if !is_variable_name(name) {
            return Err(PromptError::Invalid(format!(
                "invalid prompt variable name \"{name}\" (must match /^[a-z][a-z0-9_]*$/)"
            )));
        }
        if self.variables.contains_key(name) {
            return Err(PromptError::Invalid(format!(
                "prompt variable \"{name}\" is already registered"
            )));
        }
        self.variables.insert(name.to_string(), provider);
        Ok(())
    }

    /// Append an assemble listener. Listeners run left-to-right; the returned assembly is authoritative.
    pub fn on_assemble(
        &mut self,
        listener: fn(PromptAssembly, &AssembleContext) -> PromptAssembly,
    ) {
        self.assemble_listeners.push(listener);
    }

    /// Evaluate providers, order tools, run listeners, then restore a complete section when present.
    ///
    /// When runtime context is suppressed, the returned `contexts` list is empty even if a listener
    /// added entries.
    ///
    /// # Errors
    ///
    /// Returns when more than one complete section is active, a tool provider uses
    /// [`TOOL_ORDER_REST`] as a tool name, or `tool_order` lists unknown names.
    pub fn assemble(&self, context: &AssembleContext) -> Result<PromptAssembly, PromptError> {
        let mut variables = BTreeMap::new();
        for (name, provider) in &self.variables {
            variables.insert(name.clone(), provider(context));
        }

        let mut section_defs: Vec<&PromptSection> = self.sections.iter().collect();
        section_defs.sort_by_key(|section| section.order);
        let complete_defs: Vec<&&PromptSection> = section_defs
            .iter()
            .filter(|section| section.complete)
            .collect();
        if complete_defs.len() > 1 {
            let names = complete_defs
                .iter()
                .map(|section| format!("{:?}", section.name))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(PromptError::Invalid(format!(
                "multiple complete prompt sections are active: {names}"
            )));
        }

        let mut complete_section = None;
        let sections = section_defs
            .iter()
            .map(|section| {
                let assembled = AssembledSection {
                    name: section.name.clone(),
                    text: resolve_text(&section.text, context),
                };
                if section.complete {
                    complete_section = Some(assembled.clone());
                }
                assembled
            })
            .collect();

        let contexts = if self.runtime_context_suppressed {
            Vec::new()
        } else {
            let mut context_defs: Vec<&PromptContext> = self.contexts.iter().collect();
            context_defs.sort_by_key(|entry| entry.order);
            context_defs
                .iter()
                .map(|entry| AssembledContext {
                    name: entry.name.clone(),
                    text: resolve_text(&entry.text, context),
                })
                .collect()
        };

        let mut collected = Vec::new();
        for provider in &self.tool_providers {
            collected.extend(provider(context));
        }
        let tools = order_tools(collected, self.tool_order.as_deref())?;

        let mut assembly = PromptAssembly {
            sections,
            contexts,
            tools,
            variables,
        };
        for listener in &self.assemble_listeners {
            assembly = listener(assembly, context);
        }

        if complete_section.is_none() && !self.runtime_context_suppressed {
            return Ok(assembly);
        }
        if let Some(complete) = complete_section {
            assembly.sections = vec![complete];
        }
        if self.runtime_context_suppressed {
            assembly.contexts = Vec::new();
        }
        Ok(assembly)
    }
}

fn resolve_text(text: &SectionText, context: &AssembleContext) -> String {
    match text {
        SectionText::Static(value) => value.clone(),
        SectionText::Dynamic(provider) => provider(context),
    }
}

fn validate_tool_order(
    tool_order: Option<Vec<String>>,
) -> Result<Option<Vec<String>>, PromptError> {
    let Some(order) = tool_order else {
        return Ok(None);
    };
    let mut seen = HashSet::new();
    for name in &order {
        if !seen.insert(name.clone()) {
            return Err(PromptError::Invalid(format!(
                "toolOrder lists \"{name}\" more than once"
            )));
        }
    }
    if !seen.contains(TOOL_ORDER_REST) {
        return Err(PromptError::Invalid(format!(
            "toolOrder must contain the \"{TOOL_ORDER_REST}\" rest entry (where unlisted tools are inserted)"
        )));
    }
    Ok(Some(order))
}

fn order_tools(
    mut tools: Vec<ToolSchema>,
    tool_order: Option<&[String]>,
) -> Result<Vec<ToolSchema>, PromptError> {
    if tools.iter().any(|tool| tool.name == TOOL_ORDER_REST) {
        return Err(PromptError::Invalid(format!(
            "tool provider returned reserved tool name \"{TOOL_ORDER_REST}\" (reserved for toolOrder's rest entry)"
        )));
    }
    let Some(order) = tool_order else {
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        return Ok(tools);
    };
    let known: BTreeSet<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
    let unknown: Vec<&str> = order
        .iter()
        .filter(|name| name.as_str() != TOOL_ORDER_REST && !known.contains(name.as_str()))
        .map(String::as_str)
        .collect();
    if !unknown.is_empty() {
        let listed = unknown
            .iter()
            .map(|name| format!("\"{name}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let known_list = if known.is_empty() {
            "(none)".to_string()
        } else {
            known.into_iter().collect::<Vec<_>>().join(", ")
        };
        let noun = if unknown.len() > 1 { "tools" } else { "tool" };
        return Err(PromptError::Invalid(format!(
            "toolOrder lists unregistered {noun} {listed}; known tools: {known_list}"
        )));
    }
    let listed: HashSet<&str> = order.iter().map(String::as_str).collect();
    let mut rest: Vec<ToolSchema> = tools
        .iter()
        .filter(|tool| !listed.contains(tool.name.as_str()))
        .cloned()
        .collect();
    rest.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(order
        .iter()
        .flat_map(|name| {
            if name == TOOL_ORDER_REST {
                rest.clone()
            } else {
                tools
                    .iter()
                    .filter(|tool| tool.name == *name)
                    .cloned()
                    .collect()
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::{
        AssembleContext, HARNESS_IDENTITY_TEXT, PERSONA_SECTION, PromptContext, PromptSection,
        SectionText, SystemPrompt, SystemPromptConfig, TOOL_ORDER_REST, ToolSchema,
        join_context_sections, render_context_sections, render_context_snapshot, render_prompt,
    };
    use serde_json::json;

    fn prompt(persona: &str) -> SystemPrompt {
        SystemPrompt::new(SystemPromptConfig {
            persona: persona.into(),
            ..SystemPromptConfig::default()
        })
        .expect("config")
    }

    fn tool(name: &str) -> ToolSchema {
        ToolSchema {
            name: name.into(),
            description: name.into(),
            parameters: json!({"type": "object"}),
        }
    }

    #[test]
    fn identity_then_persona_then_guidance() {
        let mut sp = prompt("You are a test agent on {{model}}.");
        sp.section(PromptSection {
            name: "tool:noop".into(),
            order: 100,
            text: SectionText::Static("Use the noop tool wisely.".into()),
            complete: false,
        })
        .unwrap();
        sp.variable("model", |_| Some("mock".into())).unwrap();
        let assembly = sp.assemble(&AssembleContext::default()).unwrap();
        assert_eq!(
            render_prompt(&assembly).unwrap(),
            format!(
                "{HARNESS_IDENTITY_TEXT}\n\nYou are a test agent on mock.\n\nUse the noop tool wisely."
            )
        );
    }

    #[test]
    fn snapshot_prefix_and_identity_skip_when_unchanged_is_join_only() {
        let mut sp = prompt("");
        sp.context(PromptContext {
            name: "cwd".into(),
            order: 0,
            text: SectionText::Static("cwd=/tmp".into()),
        })
        .unwrap();
        let assembly = sp.assemble(&AssembleContext::default()).unwrap();
        let sections = render_context_sections(&assembly).unwrap();
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].name, "cwd");
        assert_eq!(
            join_context_sections(&sections),
            "Current runtime context. This snapshot supersedes earlier runtime-context snapshots.\n\ncwd=/tmp"
        );
        assert_eq!(
            render_context_snapshot(&assembly).unwrap(),
            join_context_sections(&sections)
        );
        assert_eq!(join_context_sections(&[]), "");
    }

    #[test]
    fn tool_order_inserts_rest_lexicographically() {
        let mut sp = SystemPrompt::new(SystemPromptConfig {
            tool_order: Some(vec!["zeta".into(), TOOL_ORDER_REST.into(), "alpha".into()]),
            ..SystemPromptConfig::default()
        })
        .unwrap();
        sp.tools(|_| vec![tool("mid"), tool("alpha"), tool("zeta"), tool("beta")]);
        let names: Vec<_> = sp
            .assemble(&AssembleContext::default())
            .unwrap()
            .tools
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, vec!["zeta", "beta", "mid", "alpha"]);
    }

    #[test]
    fn omitted_tool_order_is_lexicographic() {
        let mut sp = prompt("");
        sp.tools(|_| vec![tool("zeta"), tool("alpha")]);
        let names: Vec<_> = sp
            .assemble(&AssembleContext::default())
            .unwrap()
            .tools
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, vec!["alpha", "zeta"]);
    }

    #[test]
    fn complete_section_is_restored_after_listeners() {
        let mut sp = prompt("");
        sp.section(PromptSection {
            name: "only".into(),
            order: 50,
            text: SectionText::Static("COMPLETE".into()),
            complete: true,
        })
        .unwrap();
        sp.on_assemble(|mut assembly, _| {
            assembly.sections.push(super::AssembledSection {
                name: "injected".into(),
                text: "nope".into(),
            });
            assembly
        });
        let assembly = sp.assemble(&AssembleContext::default()).unwrap();
        assert_eq!(assembly.sections.len(), 1);
        assert_eq!(assembly.sections[0].name, "only");
        assert_eq!(assembly.sections[0].text, "COMPLETE");
    }

    #[test]
    fn two_complete_sections_fail() {
        let mut sp = prompt("");
        sp.section(PromptSection {
            name: "a".into(),
            order: 1,
            text: SectionText::Static("A".into()),
            complete: true,
        })
        .unwrap();
        let error = sp
            .section(PromptSection {
                name: "b".into(),
                order: 2,
                text: SectionText::Static("B".into()),
                complete: true,
            })
            .and_then(|()| sp.assemble(&AssembleContext::default()))
            .expect_err("two complete");
        assert!(
            error
                .to_string()
                .contains("multiple complete prompt sections are active")
        );
        assert!(error.to_string().contains("\"a\""));
        assert!(error.to_string().contains("\"b\""));
    }

    #[test]
    fn suppress_runtime_context_drops_contexts() {
        let mut sp = SystemPrompt::new(SystemPromptConfig {
            include_runtime_context: false,
            ..SystemPromptConfig::default()
        })
        .unwrap();
        sp.context(PromptContext {
            name: "cwd".into(),
            order: 0,
            text: SectionText::Static("cwd=/tmp".into()),
        })
        .unwrap();
        let assembly = sp.assemble(&AssembleContext::default()).unwrap();
        assert!(assembly.contexts.is_empty());
        assert_eq!(render_context_snapshot(&assembly).unwrap(), "");
    }

    #[test]
    fn persona_section_name_is_stable() {
        let assembly = prompt("hi").assemble(&AssembleContext::default()).unwrap();
        assert!(
            assembly
                .sections
                .iter()
                .any(|s| s.name == PERSONA_SECTION && s.text == "hi")
        );
    }
}
