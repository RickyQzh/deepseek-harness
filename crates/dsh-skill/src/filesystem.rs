//! Local filesystem skill provider.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use dsh_fs::LocalFileSystem;
use serde_yaml::Value;

use crate::{
    BUNDLED_SKILL_RANK, SkillCandidate, SkillDefinition, SkillError, SkillInvocationPolicy,
    SkillProvider, SkillSummary, is_skill_name,
};

const PROJECT_DSH_RANK: i32 = 100;
const PROJECT_AGENTS_RANK: i32 = 200;
const CUSTOM_RANK: i32 = 300;
const USER_DSH_RANK: i32 = 400;
const USER_AGENTS_RANK: i32 = 500;

/// Filesystem provider configuration resolved from YAML or tests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilesystemSkillConfig {
    /// Whether project and user roots are included around custom roots.
    pub include_default_roots: bool,
    /// Unique provider name. Defaults to `filesystem`.
    pub provider_name: String,
    /// DeepSeek Harness config root. Defaults to `$DSH_HOME` or `~/.dsh`.
    pub dsh_home: Option<PathBuf>,
    /// Shared agent config root. Defaults to `$DSH_AGENTS_HOME` or `~/.agents`.
    pub agents_home: Option<PathBuf>,
    /// Additional skill roots scanned after project roots and before user roots.
    pub custom_skill_dirs: Vec<PathBuf>,
    /// Bundled skill root; defaults to `$DSH_BUNDLED_SKILL_DIR` when default roots are included.
    pub bundled_skill_dir: Option<PathBuf>,
}

impl Default for FilesystemSkillConfig {
    fn default() -> Self {
        Self {
            include_default_roots: true,
            provider_name: "filesystem".into(),
            dsh_home: None,
            agents_home: None,
            custom_skill_dirs: Vec::new(),
            bundled_skill_dir: None,
        }
    }
}

struct SkillRoot {
    path: PathBuf,
    source: &'static str,
    rank: i32,
    skip_system: bool,
}

/// Provider that maps local project, user, custom, and bundled skill roots into [`crate::SkillRegistry`].
pub struct FilesystemSkillProvider {
    fs: LocalFileSystem,
    name: String,
    include_default_roots: bool,
    dsh_home: PathBuf,
    agents_home: PathBuf,
    custom_skill_dirs: Vec<PathBuf>,
    bundled_skill_dir: Option<PathBuf>,
}

impl FilesystemSkillProvider {
    /// Build a provider that resolves project roots against `fs.cwd` when `cwd` is omitted.
    #[must_use]
    pub fn new(fs: LocalFileSystem, config: FilesystemSkillConfig) -> Self {
        let include_default_roots = config.include_default_roots;
        let bundled_skill_dir = config.bundled_skill_dir.or_else(|| {
            if include_default_roots {
                std::env::var_os("DSH_BUNDLED_SKILL_DIR").map(PathBuf::from)
            } else {
                None
            }
        });
        Self {
            fs,
            name: config.provider_name,
            include_default_roots,
            dsh_home: config.dsh_home.unwrap_or_else(default_dsh_home),
            agents_home: config.agents_home.unwrap_or_else(default_agents_home),
            custom_skill_dirs: config.custom_skill_dirs,
            bundled_skill_dir,
        }
    }

    /// Workspace cwd used when [`SkillProvider::list`] is called with `None`.
    #[must_use]
    pub fn cwd(&self) -> &Path {
        &self.fs.cwd
    }

    fn roots(&self, cwd: Option<&str>) -> Vec<SkillRoot> {
        let mut roots = Vec::new();
        let project_cwd = cwd
            .map(PathBuf::from)
            .unwrap_or_else(|| self.fs.cwd.clone());
        if self.include_default_roots {
            let project_root = find_project_root(&project_cwd);
            roots.push(SkillRoot {
                path: project_root.join(".dsh/skills"),
                source: "project-dsh",
                rank: PROJECT_DSH_RANK,
                skip_system: false,
            });
            roots.push(SkillRoot {
                path: project_root.join(".agents/skills"),
                source: "project-agents",
                rank: PROJECT_AGENTS_RANK,
                skip_system: false,
            });
        }
        for path in &self.custom_skill_dirs {
            roots.push(SkillRoot {
                path: path.clone(),
                source: "custom",
                rank: CUSTOM_RANK,
                skip_system: false,
            });
        }
        if self.include_default_roots {
            roots.push(SkillRoot {
                path: self.dsh_home.join("skills"),
                source: "user-dsh",
                rank: USER_DSH_RANK,
                skip_system: true,
            });
            roots.push(SkillRoot {
                path: self.agents_home.join("skills"),
                source: "user-agents",
                rank: USER_AGENTS_RANK,
                skip_system: false,
            });
        }
        if let Some(path) = &self.bundled_skill_dir {
            roots.push(SkillRoot {
                path: path.clone(),
                source: "bundled",
                rank: BUNDLED_SKILL_RANK,
                skip_system: false,
            });
        }
        roots
    }
}

impl SkillProvider for FilesystemSkillProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn list(&self, cwd: Option<&str>) -> Vec<SkillCandidate> {
        let mut candidates = Vec::new();
        for root in self.roots(cwd) {
            candidates.extend(discover_root(&root, &self.name));
        }
        candidates
    }

    fn get(&self, locator: &str) -> Result<SkillDefinition, SkillError> {
        match parse_skill_file(Path::new(locator)) {
            Some(parsed) => Ok(SkillDefinition {
                summary: SkillSummary {
                    name: parsed.name,
                    description: parsed.description,
                    when_to_use: parsed.when_to_use,
                    invocation: parsed.invocation,
                    source: "filesystem".into(),
                    provider: self.name.clone(),
                },
                content: parsed.content,
                path: Some(PathBuf::from(locator)),
            }),
            None => Err(SkillError::unknown(locator)),
        }
    }
}

struct ParsedSkill {
    name: String,
    description: String,
    when_to_use: Option<String>,
    invocation: SkillInvocationPolicy,
    content: String,
}

fn discover_root(root: &SkillRoot, provider: &str) -> Vec<SkillCandidate> {
    let entries = match list_skill_root_entries(&root.path) {
        Some(entries) => entries,
        None => return Vec::new(),
    };
    let mut skills = Vec::new();
    for entry in entries {
        if root.skip_system && entry.file_name == ".system" {
            continue;
        }
        let locator = if entry.is_dir {
            entry.path.join("SKILL.md")
        } else if entry.is_file && entry.file_name.ends_with(".md") {
            entry.path.clone()
        } else {
            continue;
        };
        let Some(parsed) = parse_skill_file(&locator) else {
            continue;
        };
        skills.push(SkillCandidate {
            summary: SkillSummary {
                name: parsed.name,
                description: parsed.description,
                when_to_use: parsed.when_to_use,
                invocation: parsed.invocation,
                source: root.source.into(),
                provider: provider.into(),
            },
            rank: root.rank,
            locator: locator.to_string_lossy().into_owned(),
            path: Some(locator),
        });
    }
    skills
}

struct RootEntry {
    file_name: String,
    path: PathBuf,
    is_dir: bool,
    is_file: bool,
}

fn list_skill_root_entries(root: &Path) -> Option<Vec<RootEntry>> {
    let read = match fs::read_dir(root) {
        Ok(read) => read,
        Err(error) if is_absent(&error) => {
            eprintln!(
                "skill-filesystem: skipping missing skill root {}",
                root.display()
            );
            return Some(Vec::new());
        }
        Err(error) => {
            eprintln!(
                "skill-filesystem: skipping unreadable skill root {}: {error}",
                root.display()
            );
            return Some(Vec::new());
        }
    };
    let mut entries = Vec::new();
    for entry in read {
        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let (is_dir, is_file) = match fs::metadata(&path) {
            Ok(info) => (info.is_dir(), info.is_file()),
            Err(_) => continue,
        };
        entries.push(RootEntry {
            file_name,
            path,
            is_dir,
            is_file,
        });
    }
    Some(entries)
}

fn parse_skill_file(path: &Path) -> Option<ParsedSkill> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if is_absent(&error) => return None,
        Err(error) => {
            eprintln!(
                "skill-filesystem: skill file {} ignored: {error}",
                path.display()
            );
            return None;
        }
    };
    let parsed = match parse_frontmatter(&raw) {
        Ok(Some(parsed)) => parsed,
        Ok(None) => {
            eprintln!(
                "skill-filesystem: skill file {} ignored: missing YAML frontmatter",
                path.display()
            );
            return None;
        }
        Err(error) => {
            eprintln!(
                "skill-filesystem: skill file {} ignored: invalid YAML frontmatter: {error}",
                path.display()
            );
            return None;
        }
    };
    let name = mapping_str(&parsed.data, "name")?;
    let description = mapping_str(&parsed.data, "description")?;
    if name.is_empty() || description.is_empty() {
        eprintln!(
            "skill-filesystem: skill file {} ignored: frontmatter requires name and description",
            path.display()
        );
        return None;
    }
    if !is_skill_name(&name) {
        eprintln!(
            "skill-filesystem: skill file {} ignored: invalid skill name \"{name}\"",
            path.display()
        );
        return None;
    }
    let invocation = match parse_invocation_policy(&parsed.data) {
        Ok(invocation) => invocation,
        Err(error) => {
            eprintln!(
                "skill-filesystem: skill file {} ignored: invalid invocation frontmatter: {error}",
                path.display()
            );
            return None;
        }
    };
    Some(ParsedSkill {
        name,
        description,
        when_to_use: mapping_str(&parsed.data, "whenToUse").filter(|value| !value.is_empty()),
        invocation,
        content: parsed.body.trim().to_string(),
    })
}

struct Frontmatter {
    data: Value,
    body: String,
}

fn parse_frontmatter(raw: &str) -> Result<Option<Frontmatter>, serde_yaml::Error> {
    let first_line_end = match raw.find('\n') {
        Some(index) => index,
        None => return Ok(None),
    };
    let first_line = raw[..first_line_end].trim_end_matches('\r');
    if first_line != "---" {
        return Ok(None);
    }
    let start = first_line_end + 1;
    let Some(closing) = find_closing_frontmatter(raw, start) else {
        return Ok(None);
    };
    let yaml = &raw[start..closing.start];
    let parsed: Value = serde_yaml::from_str(yaml)?;
    if !parsed.is_mapping() {
        return Ok(None);
    }
    Ok(Some(Frontmatter {
        data: parsed,
        body: raw[closing.body_start..].to_string(),
    }))
}

struct Closing {
    start: usize,
    body_start: usize,
}

fn find_closing_frontmatter(raw: &str, start: usize) -> Option<Closing> {
    let mut line_start = start;
    while line_start <= raw.len() {
        let next_newline = raw[line_start..].find('\n').map(|rel| line_start + rel);
        let line_end = next_newline.unwrap_or(raw.len());
        let line = raw[line_start..line_end].trim_end_matches('\r');
        if line == "---" {
            return Some(Closing {
                start: line_start,
                body_start: next_newline.map(|index| index + 1).unwrap_or(raw.len()),
            });
        }
        line_start = match next_newline {
            Some(index) => index + 1,
            None => return None,
        };
    }
    None
}

fn parse_invocation_policy(data: &Value) -> Result<SkillInvocationPolicy, String> {
    reject_legacy_key(data, "disableModelInvocation", "disable-model-invocation")?;
    reject_legacy_key(data, "modelInvocable", "disable-model-invocation")?;
    reject_legacy_key(data, "userInvocable", "user-invocable")?;
    let disable_model = frontmatter_boolean(data, "disable-model-invocation")?;
    let user_invocable = frontmatter_boolean(data, "user-invocable")?;
    Ok(SkillInvocationPolicy {
        model_invocable: disable_model != Some(true),
        user_invocable: user_invocable != Some(false),
    })
}

fn reject_legacy_key(data: &Value, legacy: &str, canonical: &str) -> Result<(), String> {
    if mapping_has(data, legacy) {
        return Err(format!(
            "frontmatter field \"{legacy}\" is unsupported; use \"{canonical}\""
        ));
    }
    Ok(())
}

fn frontmatter_boolean(data: &Value, key: &str) -> Result<Option<bool>, String> {
    let Some(value) = mapping_get(data, key) else {
        return Ok(None);
    };
    match value {
        Value::Bool(flag) => Ok(Some(*flag)),
        Value::Number(number) if number.as_i64() == Some(1) => Ok(Some(true)),
        Value::Number(number) if number.as_i64() == Some(0) => Ok(Some(false)),
        Value::String(text) => match text.as_str() {
            "1" => Ok(Some(true)),
            "0" => Ok(Some(false)),
            other => match other.to_ascii_lowercase().as_str() {
                "true" | "yes" | "on" => Ok(Some(true)),
                "false" | "no" | "off" => Ok(Some(false)),
                _ => Err(format!("frontmatter field \"{key}\" must be a boolean")),
            },
        },
        _ => Err(format!("frontmatter field \"{key}\" must be a boolean")),
    }
}

fn mapping_get<'a>(data: &'a Value, key: &str) -> Option<&'a Value> {
    data.as_mapping()?.get(&Value::String(key.to_string()))
}

fn mapping_has(data: &Value, key: &str) -> bool {
    mapping_get(data, key).is_some()
}

fn mapping_str(data: &Value, key: &str) -> Option<String> {
    match mapping_get(data, key)? {
        Value::String(text) => Some(text.clone()),
        _ => None,
    }
}

fn find_project_root(cwd: &Path) -> PathBuf {
    let mut current = cwd.to_path_buf();
    loop {
        if current.join(".git").exists() {
            return current;
        }
        let Some(parent) = current.parent() else {
            return cwd.to_path_buf();
        };
        if parent == current {
            return cwd.to_path_buf();
        }
        current = parent.to_path_buf();
    }
}

fn default_dsh_home() -> PathBuf {
    std::env::var_os("DSH_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".dsh"))
}

fn default_agents_home() -> PathBuf {
    std::env::var_os("DSH_AGENTS_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".agents"))
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

fn is_absent(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound || error.kind() == io::ErrorKind::NotADirectory
}

#[cfg(test)]
mod tests {
    use super::FilesystemSkillProvider;
    use crate::{FilesystemSkillConfig, SkillProvider, SkillRegistry};
    use dsh_fs::LocalFileSystem;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

    fn test_temp_dir(prefix: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dsh-skill-{prefix}-{}-{}",
            std::process::id(),
            TEST_DIR_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn provider_get_demo(
        provider: &FilesystemSkillProvider,
    ) -> Result<crate::SkillDefinition, crate::SkillError> {
        let cwd = provider.cwd().to_string_lossy().into_owned();
        let demo = provider
            .list(Some(&cwd))
            .into_iter()
            .find(|candidate| candidate.summary.name == "demo-skill")
            .ok_or_else(|| crate::SkillError::unknown("demo-skill"))?;
        provider.get(&demo.locator)
    }

    #[tokio::test]
    async fn filesystem_loads_directory_bundle() {
        let root = test_temp_dir("skills");
        let dir = root.join(".dsh/skills/demo-skill");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: demo-skill\ndescription: Demo.\n---\nBody here\n",
        )
        .unwrap();
        let fs = LocalFileSystem::new(root.clone());
        let provider = FilesystemSkillProvider::new(
            fs,
            FilesystemSkillConfig {
                include_default_roots: true,
                ..Default::default()
            },
        );
        let def = provider_get_demo(&provider).unwrap();
        assert_eq!(def.content.trim(), "Body here");
    }

    #[test]
    fn get_preserves_winning_list_discovery_source() {
        let root = test_temp_dir("skills-source");
        let dir = root.join(".dsh/skills/demo-skill");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: demo-skill\ndescription: Demo.\n---\nBody here\n",
        )
        .unwrap();
        let fs = LocalFileSystem::new(root.clone());
        let provider = FilesystemSkillProvider::new(
            fs,
            FilesystemSkillConfig {
                include_default_roots: true,
                ..Default::default()
            },
        );
        let registry = SkillRegistry::new();
        registry.register_provider(Arc::new(provider)).unwrap();
        let cwd = root.to_string_lossy();
        let list_source = registry
            .list(Some(cwd.as_ref()))
            .into_iter()
            .find(|summary| summary.name == "demo-skill")
            .expect("demo-skill listed")
            .source;
        assert_ne!(list_source, "filesystem");
        let definition = registry.get("demo-skill", Some(cwd.as_ref())).unwrap();
        assert_eq!(definition.summary.source, list_source);
    }
}
