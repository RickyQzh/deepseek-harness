//! Instruction-file discovery and bounded host or provider reads.

use std::path::{Path, PathBuf};

use dsh_fs::{FsInfoType, LocalFileSystem};

use crate::config::{AgentInstructionsConfig, resolve_dsh_home};
use crate::render::{USER_GLOBAL_FILE, instruction_content_sha1};

/// An instruction file whose UTF-8 content was read successfully.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedInstructionFile {
    /// Absolute filesystem path.
    pub absolute_path: PathBuf,
    /// Project-relative or `$DSH_HOME/AGENTS.md` path shown to the model.
    pub display_path: String,
    /// Exact UTF-8 file text.
    pub content: String,
}

/// Walk from `cwd` up through `project_root_markers`, then load every existing candidate.
pub async fn load_baseline_files(
    cwd: &Path,
    config: &AgentInstructionsConfig,
    file_system: Option<&LocalFileSystem>,
) -> Vec<LoadedInstructionFile> {
    if config.max_bytes == 0 || config.max_source_bytes == 0 {
        return Vec::new();
    }
    let dsh_home = resolve_dsh_home(config.dsh_home.as_deref());
    let mut discovered = Vec::new();
    let user_global = dsh_home.join(USER_GLOBAL_FILE);
    if path_is_file(&user_global, file_system).await {
        discovered.push((user_global, format!("$DSH_HOME/{USER_GLOBAL_FILE}")));
    }
    let cwd = resolve_path(cwd);
    let project_root = find_project_root(&cwd, &config.project_root_markers, file_system).await;
    for dir in ancestor_chain(&project_root, &cwd) {
        for candidates in [
            &config.instruction_file_candidates,
            &config.local_instruction_file_candidates,
        ] {
            for candidate in candidates {
                let path = dir.join(candidate);
                if path_is_file(&path, file_system).await {
                    discovered.push((path.clone(), relative_display(&project_root, &path)));
                }
            }
        }
    }
    let mut loaded = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (absolute_path, display_path) in discovered {
        if !seen.insert(absolute_path.clone()) {
            continue;
        }
        let Some(content) =
            read_bounded(&absolute_path, config.max_source_bytes, file_system).await
        else {
            continue;
        };
        loaded.push(LoadedInstructionFile {
            absolute_path,
            display_path,
            content,
        });
    }
    dedup_instruction_files_by_directory(loaded)
}

/// Walk upward to the first directory containing a configured root marker.
pub async fn find_project_root(
    cwd: &Path,
    markers: &[String],
    file_system: Option<&LocalFileSystem>,
) -> PathBuf {
    let mut current = resolve_path(cwd);
    loop {
        for marker in markers {
            if path_exists(&current.join(marker), file_system).await {
                return current;
            }
        }
        match current.parent() {
            Some(parent) if parent != current => current = parent.to_path_buf(),
            _ => return resolve_path(cwd),
        }
    }
}

/// Inclusive root-to-cwd directory chain, broadest first.
#[must_use]
pub fn ancestor_chain(root: &Path, cwd: &Path) -> Vec<PathBuf> {
    let mut chain = Vec::new();
    let mut current = resolve_path(cwd);
    let resolved_root = resolve_path(root);
    while current != resolved_root {
        chain.push(current.clone());
        match current.parent() {
            Some(parent) if parent != current => current = parent.to_path_buf(),
            _ => break,
        }
    }
    chain.push(resolved_root);
    chain.reverse();
    chain
}

fn relative_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| path.to_string_lossy().into_owned())
}

fn resolve_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn dedup_instruction_files_by_directory(
    files: Vec<LoadedInstructionFile>,
) -> Vec<LoadedInstructionFile> {
    let mut kept_digests_by_dir: std::collections::HashMap<
        String,
        std::collections::HashSet<String>,
    > = std::collections::HashMap::new();
    let mut kept = Vec::new();
    for file in files {
        let dir = match std::path::Path::new(&file.display_path).parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.to_string_lossy().into_owned(),
            _ => ".".to_string(),
        };
        let digest = instruction_content_sha1(file.content.trim());
        let digests = kept_digests_by_dir.entry(dir).or_default();
        if !digests.insert(digest) {
            continue;
        }
        kept.push(file);
    }
    kept
}

async fn path_exists(path: &Path, file_system: Option<&LocalFileSystem>) -> bool {
    match file_system {
        Some(fs) => match fs.resolve(&path.to_string_lossy(), None, None).await {
            Ok(target) => fs.stat(&target, None).await.ok().flatten().is_some(),
            Err(_) => false,
        },
        None => path.exists(),
    }
}

async fn path_is_file(path: &Path, file_system: Option<&LocalFileSystem>) -> bool {
    match file_system {
        Some(fs) => {
            let Ok(target) = fs.resolve(&path.to_string_lossy(), None, None).await else {
                return false;
            };
            match fs.stat(&target, None).await {
                Ok(Some(info)) => info.kind == FsInfoType::File,
                Ok(None) | Err(_) => false,
            }
        }
        None => path.is_file(),
    }
}

async fn read_bounded(
    path: &Path,
    max_source_bytes: u64,
    file_system: Option<&LocalFileSystem>,
) -> Option<String> {
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() > max_source_bytes {
            return None;
        }
    }
    let content = match file_system {
        Some(fs) => {
            let target = fs.resolve(&path.to_string_lossy(), None, None).await.ok()?;
            fs.read_text(&target, None).await.ok()?
        }
        None => std::fs::read_to_string(path).ok()?,
    };
    if content.len() as u64 > max_source_bytes {
        return None;
    }
    Some(content)
}

#[cfg(test)]
mod tests {
    use crate::render::instruction_content_sha1;

    #[test]
    fn sha1_abc_matches_fips_vector() {
        assert_eq!(
            instruction_content_sha1("abc"),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
    }
}
