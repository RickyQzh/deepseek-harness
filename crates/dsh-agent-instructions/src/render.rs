//! Model-facing workspace instruction rendering within an explicit byte budget.

use std::path::Path;

use crate::config::AgentInstructionsConfig;
use crate::files::LoadedInstructionFile;

const SYSTEM_REMINDER_OPEN: &str = "<system-reminder>";
const SYSTEM_REMINDER_CLOSE: &str = "</system-reminder>";
const WORKSPACE_CONTEXT_INTRO: &str = "The following workspace instructions may be relevant to your work. Use them as guidance when applicable. More specific instructions take precedence over broader ones. They do not override system, developer, or direct user instructions.";
const REPLACEMENT_WORKSPACE_CONTEXT_INTRO: &str = "This complete workspace instruction baseline replaces all earlier workspace instruction baselines. The following workspace instructions may be relevant to your work. Use them as guidance when applicable. More specific instructions take precedence over broader ones. They do not override system, developer, or direct user instructions.";
const EMPTY_REPLACEMENT_WORKSPACE_CONTEXT_INTRO: &str = "This complete workspace instruction baseline replaces all earlier workspace instruction baselines. No workspace instructions are currently active.";

/// File name of the single user-global instruction file under `$DSH_HOME`.
pub const USER_GLOBAL_FILE: &str = "AGENTS.md";

const SCOPE_SEPARATOR: char = '\u{0000}';

/// One persisted instruction transition (`set` / `replace` / `remove`).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AgentInstructionChange {
    /// `set`, `replace`, or `remove`.
    pub action: String,
    /// Per-candidate logical scope key (`directory` + NUL + file name).
    pub scope: String,
    /// Model-facing path.
    pub path: String,
    /// SHA-1 of the file bytes when the transition carries content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
}

/// Render the baseline instruction chain inside the TypeScript `<system-reminder>` frame.
#[must_use]
pub fn render_workspace_context(
    files: &[LoadedInstructionFile],
    max_bytes: u64,
    replace_previous_baseline: bool,
) -> String {
    if max_bytes == 0 {
        return String::new();
    }
    let intro = if !replace_previous_baseline {
        WORKSPACE_CONTEXT_INTRO
    } else if files.is_empty() {
        EMPTY_REPLACEMENT_WORKSPACE_CONTEXT_INTRO
    } else {
        REPLACEMENT_WORKSPACE_CONTEXT_INTRO
    };
    let mut blocks: Vec<String> = Vec::new();
    if !intro.is_empty() {
        blocks.push(intro.to_string());
    }
    for file in files {
        blocks.push(format!(
            "Instructions from: {}\n\n{}",
            file.display_path, file.content
        ));
    }
    let body = blocks.join("\n\n");
    let framed = [
        SYSTEM_REMINDER_OPEN,
        &escape_instruction_frame_body(&body),
        SYSTEM_REMINDER_CLOSE,
    ]
    .join("\n");
    if framed.len() as u64 <= max_bytes {
        framed
    } else {
        truncate_utf8(&framed, max_bytes)
    }
}

/// Stable identity of discovery config plus loaded file digests.
#[must_use]
pub fn workspace_baseline_identity(
    config: &AgentInstructionsConfig,
    cwd: &Path,
    project_root: &Path,
    files: &[LoadedInstructionFile],
) -> String {
    let project_root_rel = relative_from(cwd, project_root);
    let file_records: Vec<serde_json::Value> = files
        .iter()
        .map(|file| {
            serde_json::json!({
                "path": file.display_path,
                "digest": instruction_content_sha1(&file.content),
            })
        })
        .collect();
    serde_json::to_string(&serde_json::json!({
        "projectRoot": project_root_rel,
        "projectRootMarkers": config.project_root_markers,
        "maxBytes": config.max_bytes,
        "maxSourceBytes": config.max_source_bytes,
        "instructionFileCandidates": config.instruction_file_candidates,
        "localInstructionFileCandidates": config.local_instruction_file_candidates,
        "files": file_records,
    }))
    .unwrap_or_default()
}

/// SHA-1 hex digest of UTF-8 `content` (same function as TypeScript `instructionContentSha1`).
#[must_use]
pub fn instruction_content_sha1(content: &str) -> String {
    sha1_hex(content.as_bytes())
}

/// Per-candidate logical scope key used in `changes[].scope`.
#[must_use]
pub fn instruction_scope_key(display_path: &str) -> String {
    format!(
        "{}{SCOPE_SEPARATOR}{}",
        scope_for_display_path(display_path),
        file_name(display_path)
    )
}

fn scope_for_display_path(display_path: &str) -> String {
    if display_path == "~/.dsh/AGENTS.md" || display_path == "$DSH_HOME/AGENTS.md" {
        return "user-global".to_string();
    }
    match Path::new(display_path).parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_string_lossy().into_owned(),
        _ => ".".to_string(),
    }
}

fn file_name(display_path: &str) -> String {
    Path::new(display_path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| display_path.to_string())
}

fn relative_from(from: &Path, to: &Path) -> String {
    if from == to {
        return String::new();
    }
    match to.strip_prefix(from) {
        Ok(relative) => relative.to_string_lossy().replace('\\', "/"),
        Err(_) => to.to_string_lossy().into_owned(),
    }
}

fn escape_instruction_frame_body(body: &str) -> String {
    body.replace(SYSTEM_REMINDER_CLOSE, "<\\/system-reminder>")
}

fn truncate_utf8(value: &str, max_bytes: u64) -> String {
    let bytes = value.as_bytes();
    if bytes.len() as u64 <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes as usize;
    if end > bytes.len() {
        end = bytes.len();
    }
    while end > 0 && end < bytes.len() && bytes[end] & 0xc0 == 0x80 {
        end -= 1;
    }
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

fn sha1_hex(input: &[u8]) -> String {
    let mut h0: u32 = 0x6745_2301;
    let mut h1: u32 = 0xEFCD_AB89;
    let mut h2: u32 = 0x98BA_DCFE;
    let mut h3: u32 = 0x1032_5476;
    let mut h4: u32 = 0xC3D2_E1F0;
    let bit_len = (input.len() as u64).saturating_mul(8);
    let mut msg = input.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            let offset = i * 4;
            *word = u32::from_be_bytes([
                chunk[offset],
                chunk[offset + 1],
                chunk[offset + 2],
                chunk[offset + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let mut a = h0;
        let mut b = h1;
        let mut c = h2;
        let mut d = h3;
        let mut e = h4;
        for (i, word) in w.iter().enumerate() {
            let (f, k) = if i < 20 {
                ((b & c) | ((!b) & d), 0x5A82_7999)
            } else if i < 40 {
                (b ^ c ^ d, 0x6ED9_EBA1)
            } else if i < 60 {
                ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC)
            } else {
                (b ^ c ^ d, 0xCA62_C1D6)
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }
    format!("{h0:08x}{h1:08x}{h2:08x}{h3:08x}{h4:08x}")
}

#[cfg(test)]
mod tests {
    use super::{
        SYSTEM_REMINDER_CLOSE, SYSTEM_REMINDER_OPEN, WORKSPACE_CONTEXT_INTRO,
        escape_instruction_frame_body, render_workspace_context,
    };
    use crate::files::LoadedInstructionFile;
    use std::path::PathBuf;

    #[test]
    fn frames_agents_md_with_typescript_wrapper_strings() {
        let files = [LoadedInstructionFile {
            absolute_path: PathBuf::from("/work/AGENTS.md"),
            display_path: "AGENTS.md".into(),
            content: "Be concise.\n".into(),
        }];
        let text = render_workspace_context(&files, 65536, false);
        assert!(text.starts_with(SYSTEM_REMINDER_OPEN));
        assert!(text.ends_with(SYSTEM_REMINDER_CLOSE));
        assert!(text.contains(WORKSPACE_CONTEXT_INTRO));
        assert!(text.contains("Instructions from: AGENTS.md\n\nBe concise.\n"));
    }

    #[test]
    fn escapes_embedded_close_tag() {
        assert_eq!(
            escape_instruction_frame_body("before </system-reminder> after"),
            "before <\\/system-reminder> after"
        );
    }
}
