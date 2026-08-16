//! Model-facing `glob` and `grep` tools that spawn `rg` with `--no-config` first.

use dsh_session::ContentBlock;
use dsh_subprocess::{
    LocalSubprocessRuntime, SubprocessCollect, SubprocessError, SubprocessOutput,
    SubprocessSpawnSpec, SubprocessStdin, SubprocessStdio,
};
use dsh_tools::{AbortFlag, ToolDefinition, ToolError};
use serde_json::{Value, json};

use crate::FsToolContext;

/// Directory names `glob` excludes with a prune glob and a contents glob.
pub const GLOB_VCS_EXCLUDES: &[&str] = &[".git", ".svn", ".hg", ".bzr", ".jj", ".sl"];

const SEARCH_ERROR_NAME: &str = "SearchError";
const SEARCH_ABORTED: &str = "SEARCH_ABORTED";
const SEARCH_FAILED: &str = "SEARCH_FAILED";
const SEARCH_INVALID_PATTERN: &str = "SEARCH_INVALID_PATTERN";
const SEARCH_RAW_OUTPUT_OVERFLOW: &str = "SEARCH_RAW_OUTPUT_OVERFLOW";

/// Validated `glob` arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlobInput {
    /// Path glob passed as `--glob={pattern}`.
    pub pattern: String,
    /// Optional search root, passed after `--`.
    pub path: Option<String>,
}

/// Validated `grep` arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrepInput {
    /// Regular expression passed as `--regexp={pattern}`.
    pub pattern: String,
    /// Optional file or directory, passed after `--`.
    pub path: Option<String>,
    /// Optional positive file glob passed as `--glob={include}`.
    pub include: Option<String>,
}

/// Collect budgets for one `rg` spawn. This type is the explicit defaulting step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchCaps {
    /// Complete stdout cap in bytes. Default `20_000_000`. A lossy read is `SEARCH_RAW_OUTPUT_OVERFLOW`.
    pub raw_output_max_bytes: usize,
    /// SIGTERM→SIGKILL grace in milliseconds. Default `3_000`.
    pub grace_ms: u64,
    /// Stderr diagnostic tail cap in bytes. Default 64 KiB. No spill file.
    pub stderr_max_bytes: usize,
}

impl Default for SearchCaps {
    fn default() -> Self {
        Self {
            raw_output_max_bytes: 20_000_000,
            grace_ms: 3_000,
            stderr_max_bytes: 64 * 1024,
        }
    }
}

/// Build `[binary, "--no-config", ...rest]`.
///
/// `--no-config` is always index 1, including when `rest` is empty. This
/// function never reads `RIPGREP_CONFIG_PATH`; `--no-config` is how ripgrep
/// ignores that variable and any `rg.conf`.
#[must_use]
pub fn ripgrep_argv(binary: &str, rest: &[String]) -> Vec<String> {
    let mut argv = Vec::with_capacity(rest.len() + 2);
    argv.push(binary.to_string());
    argv.push("--no-config".to_string());
    argv.extend_from_slice(rest);
    argv
}

/// Parse `glob` JSON arguments.
///
/// `pattern` is a required string whose trim is non-empty. `path`, when
/// present, is a string whose trim is non-empty.
///
/// # Errors
///
/// Returns a model-facing argument error when a field is missing or empty.
pub fn parse_glob_args(args: &Value) -> Result<GlobInput, String> {
    let pattern = required_string(args, "pattern")?;
    if pattern.trim().is_empty() {
        return Err("pattern must be a non-empty string".into());
    }
    Ok(GlobInput {
        pattern,
        path: optional_nonempty_string(args, "path")?,
    })
}

/// Parse `grep` JSON arguments.
///
/// `pattern` is a required non-empty string; whitespace-only is a valid regex.
/// `path` and `include`, when present, are strings whose trim is non-empty.
///
/// # Errors
///
/// Returns a model-facing argument error when a field is missing or empty.
pub fn parse_grep_args(args: &Value) -> Result<GrepInput, String> {
    let pattern = required_string(args, "pattern")?;
    if pattern.is_empty() {
        return Err("pattern must be a non-empty string".into());
    }
    Ok(GrepInput {
        pattern,
        path: optional_nonempty_string(args, "path")?,
        include: optional_nonempty_string(args, "include")?,
    })
}

/// Fixed `rg --files` argv for one `glob` call (excluding the binary).
///
/// Model values are plain argv elements. A `path` rides behind `--` so a
/// leading-dash root cannot be parsed as a flag. Each [`GLOB_VCS_EXCLUDES`]
/// name is excluded with `--glob=!**/{name}` and `--glob=!**/{name}/**`.
#[must_use]
pub fn build_glob_command(input: &GlobInput) -> Vec<String> {
    let mut parts = vec![
        "--files".to_string(),
        format!("--glob={}", input.pattern),
        "--sort=modified".to_string(),
        "--no-ignore".to_string(),
        "--hidden".to_string(),
    ];
    for name in GLOB_VCS_EXCLUDES {
        parts.push(format!("--glob=!**/{name}"));
        parts.push(format!("--glob=!**/{name}/**"));
    }
    if let Some(path) = &input.path {
        parts.push("--".to_string());
        parts.push(path.clone());
    }
    parts
}

/// Fixed `rg --json` argv for one `grep` call (excluding the binary).
///
/// The pattern and optional include use `--flag=value` form. A `path` rides
/// behind `--`.
#[must_use]
pub fn build_grep_command(input: &GrepInput) -> Vec<String> {
    let mut parts = vec!["--json".to_string(), format!("--regexp={}", input.pattern)];
    if let Some(include) = &input.include {
        parts.push(format!("--glob={include}"));
    }
    if let Some(path) = &input.path {
        parts.push("--".to_string());
        parts.push(path.clone());
    }
    parts
}

/// Spawn `rg` through `subprocess` with `--no-config` as argv[1] and return complete stdout.
///
/// stdin is ignored. stdout is collected with `caps.raw_output_max_bytes` and
/// no spill; stderr with `caps.stderr_max_bytes` and no spill. Exit 0 or 1 is
/// success (1 means no matches). This function never reads `RIPGREP_CONFIG_PATH`.
///
/// # Errors
///
/// [`ToolError::Coded`] with name `SearchError` and one of `SEARCH_ABORTED`,
/// `SEARCH_INVALID_PATTERN`, `SEARCH_RAW_OUTPUT_OVERFLOW`, or `SEARCH_FAILED`.
pub async fn run_ripgrep(
    subprocess: &LocalSubprocessRuntime,
    binary: &str,
    rest: &[String],
    cwd: &str,
    caps: &SearchCaps,
    signal: Option<AbortFlag>,
) -> Result<String, ToolError> {
    if signal.as_ref().is_some_and(AbortFlag::is_aborted) {
        return Err(search_error(
            "search was aborted before completion (tool timeout or caller cancellation)",
            SEARCH_ABORTED,
        ));
    }
    let spec = SubprocessSpawnSpec {
        argv: ripgrep_argv(binary, rest),
        cwd: cwd.to_string(),
        stdio: SubprocessStdio {
            stdin: SubprocessStdin::Ignore,
            stdout: SubprocessOutput::Collect(SubprocessCollect {
                max_bytes: caps.raw_output_max_bytes,
                spill_max_bytes: None,
            }),
            stderr: SubprocessOutput::Collect(SubprocessCollect {
                max_bytes: caps.stderr_max_bytes,
                spill_max_bytes: None,
            }),
        },
        grace_ms: caps.grace_ms,
        signal: signal.clone(),
        env: None,
    };
    let handle = match subprocess.spawn(spec) {
        Ok(handle) => handle,
        Err(SubprocessError::AbortedBeforeSpawn) => {
            return Err(search_error(
                "search was aborted before completion (tool timeout or caller cancellation)",
                SEARCH_ABORTED,
            ));
        }
        Err(_) => {
            return Err(search_error(
                "search could not start its search command (ripgrep launch failed)",
                SEARCH_FAILED,
            ));
        }
    };
    let outcome = match handle.done().await {
        Ok(outcome) => outcome,
        Err(_) => {
            return Err(search_error(
                "search could not start its search command (ripgrep launch failed)",
                SEARCH_FAILED,
            ));
        }
    };
    if signal.as_ref().is_some_and(AbortFlag::is_aborted) {
        return Err(search_error(
            "search was aborted before completion (tool timeout or caller cancellation)",
            SEARCH_ABORTED,
        ));
    }
    let stdout = match handle.stdout_reader() {
        Some(reader) => reader.lock().await.read_from(0),
        None => {
            return Err(search_error(
                "search command produced no collected output streams",
                SEARCH_FAILED,
            ));
        }
    };
    let stderr = match handle.stderr_reader() {
        Some(reader) => reader.lock().await.read_from(0),
        None => {
            return Err(search_error(
                "search command produced no collected output streams",
                SEARCH_FAILED,
            ));
        }
    };
    if let Some(killed) = outcome.signal {
        return Err(search_error(
            format!("search command was killed by signal {killed}"),
            SEARCH_FAILED,
        ));
    }
    let Some(exit_code) = outcome.exit_code else {
        return Err(search_error(
            "search command was killed by signal (unknown)",
            SEARCH_FAILED,
        ));
    };
    if exit_code != 0 && exit_code != 1 {
        return Err(classify_run_failure(exit_code, &stderr.text));
    }
    if stdout.lossy {
        return Err(search_error(
            format!(
                "search produced more raw output than the subprocess retained within the {}-byte cap; narrow pattern, path, or include and retry",
                caps.raw_output_max_bytes
            ),
            SEARCH_RAW_OUTPUT_OVERFLOW,
        ));
    }
    Ok(stdout.text)
}

/// Registerable `glob` definition. Concurrently schedulable like `read`.
pub(crate) fn glob_definition(ctx: FsToolContext) -> ToolDefinition {
    ToolDefinition {
        name: "glob".into(),
        description: "Find files whose paths match a glob pattern. Returns matching file paths, never directories.".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern to match file paths against (e.g. \"**/*.ts\")."
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search in. Defaults to the filesystem cwd."
                }
            },
            "required": ["pattern"]
        }),
        execute: Box::new(move |args, exec| {
            let ctx = ctx.clone();
            Box::pin(async move { glob_execute(&ctx, args, exec.signal).await })
        }),
        render: Box::new(|_args, value| {
            let paths = value
                .get("paths")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            vec![ContentBlock::Text {
                text: if paths.is_empty() {
                    "No files found".into()
                } else {
                    paths
                },
            }]
        }),
        is_concurrency_safe: Some(Box::new(|_| true)),
    }
}

/// Registerable `grep` definition. Concurrently schedulable like `read`.
pub(crate) fn grep_definition(ctx: FsToolContext) -> ToolDefinition {
    ToolDefinition {
        name: "grep".into(),
        description: "Search file contents with a ripgrep regular expression.".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regular expression to search for (ripgrep syntax)."
                },
                "path": {
                    "type": "string",
                    "description": "File or directory to search. Defaults to the filesystem cwd."
                },
                "include": {
                    "type": "string",
                    "description": "One glob filter for which files to search (e.g. \"*.ts\")."
                }
            },
            "required": ["pattern"]
        }),
        execute: Box::new(move |args, exec| {
            let ctx = ctx.clone();
            Box::pin(async move { grep_execute(&ctx, args, exec.signal).await })
        }),
        render: Box::new(|_args, value| {
            vec![ContentBlock::Text {
                text: format_grep_matches(value),
            }]
        }),
        is_concurrency_safe: Some(Box::new(|_| true)),
    }
}

async fn glob_execute(
    ctx: &FsToolContext,
    args: Value,
    signal: AbortFlag,
) -> Result<Value, ToolError> {
    let input = parse_glob_args(&args).map_err(ToolError::Other)?;
    let rest = build_glob_command(&input);
    let stdout = run_ripgrep(
        &ctx.subprocess,
        &ctx.rg_binary,
        &rest,
        &ctx_cwd(ctx),
        &SearchCaps::default(),
        Some(signal),
    )
    .await?;
    let paths: Vec<String> = stdout
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();
    let root = input.path.as_deref().unwrap_or(".");
    Ok(json!({ "root": root, "paths": paths }))
}

async fn grep_execute(
    ctx: &FsToolContext,
    args: Value,
    signal: AbortFlag,
) -> Result<Value, ToolError> {
    let input = parse_grep_args(&args).map_err(ToolError::Other)?;
    let rest = build_grep_command(&input);
    let stdout = run_ripgrep(
        &ctx.subprocess,
        &ctx.rg_binary,
        &rest,
        &ctx_cwd(ctx),
        &SearchCaps::default(),
        Some(signal),
    )
    .await?;
    let matches = parse_grep_matches(&stdout)?;
    Ok(json!({ "matches": matches }))
}

fn ctx_cwd(ctx: &FsToolContext) -> String {
    ctx.fs.cwd.to_string_lossy().into_owned()
}

fn required_string(args: &Value, key: &str) -> Result<String, String> {
    match args.get(key) {
        Some(Value::String(value)) => Ok(value.clone()),
        _ => Err(format!("{key} must be a non-empty string")),
    }
}

fn optional_nonempty_string(args: &Value, key: &str) -> Result<Option<String>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => {
            if value.trim().is_empty() {
                Err(format!("{key} must be a non-empty string when given"))
            } else {
                Ok(Some(value.clone()))
            }
        }
        Some(_) => Err(format!("{key} must be a string")),
    }
}

fn search_error(message: impl Into<String>, code: &str) -> ToolError {
    ToolError::Coded {
        message: message.into(),
        name: SEARCH_ERROR_NAME.into(),
        code: code.into(),
    }
}

fn classify_run_failure(exit_code: i32, stderr: &str) -> ToolError {
    let lower = stderr.to_ascii_lowercase();
    if lower.contains("regex parse error") || lower.contains("error parsing glob") {
        return search_error(
            format!("search pattern rejected by ripgrep: {}", stderr.trim()),
            SEARCH_INVALID_PATTERN,
        );
    }
    let excerpt = stderr.trim();
    if excerpt.is_empty() {
        search_error(format!("search failed (exit {exit_code})"), SEARCH_FAILED)
    } else {
        search_error(
            format!("search failed (exit {exit_code}): {excerpt}"),
            SEARCH_FAILED,
        )
    }
}

fn parse_grep_matches(stdout: &str) -> Result<Vec<Value>, ToolError> {
    let mut matches = Vec::new();
    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }
        if let Some(item) = parse_grep_match_line(line)? {
            matches.push(item);
        }
    }
    Ok(matches)
}

fn parse_grep_match_line(line: &str) -> Result<Option<Value>, ToolError> {
    let parsed: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(_not_json) => return Err(malformed_grep("a line is not JSON")),
    };
    let Some(record) = parsed.as_object() else {
        return Err(malformed_grep("a record is not an object"));
    };
    if record.get("type").and_then(Value::as_str) != Some("match") {
        return Ok(None);
    }
    let Some(data) = record.get("data").and_then(Value::as_object) else {
        return Err(malformed_grep("a match record has no data"));
    };
    let path = data
        .get("path")
        .and_then(Value::as_object)
        .and_then(|path| path.get("text"))
        .and_then(Value::as_str)
        .ok_or_else(|| malformed_grep("a match record has no path text"))?;
    let line_number = data
        .get("line_number")
        .and_then(Value::as_i64)
        .ok_or_else(|| malformed_grep("a match record has no line number"))?;
    let Some(lines) = data.get("lines").and_then(Value::as_object) else {
        return Err(malformed_grep("a match record has no line content"));
    };
    let line_text = if let Some(text) = lines.get("text").and_then(Value::as_str) {
        text.trim_end_matches(['\r', '\n']).to_string()
    } else if lines.get("bytes").and_then(Value::as_str).is_some() {
        "(line is not valid UTF-8)".to_string()
    } else {
        return Err(malformed_grep(
            "a match record has neither line text nor bytes",
        ));
    };
    Ok(Some(json!({
        "path": path,
        "lineNumber": line_number,
        "line": line_text,
    })))
}

fn malformed_grep(detail: &str) -> ToolError {
    search_error(
        format!("grep received malformed ripgrep --json output ({detail})"),
        SEARCH_FAILED,
    )
}

fn format_grep_matches(value: &Value) -> String {
    let Some(matches) = value.get("matches").and_then(Value::as_array) else {
        return String::new();
    };
    if matches.is_empty() {
        return "No matches found".into();
    }
    matches
        .iter()
        .map(|item| {
            let path = item.get("path").and_then(Value::as_str).unwrap_or("");
            let line_number = item.get("lineNumber").and_then(Value::as_i64).unwrap_or(0);
            let line = item.get("line").and_then(Value::as_str).unwrap_or("");
            format!("{path}:{line_number}:{line}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::{
        GLOB_VCS_EXCLUDES, GlobInput, GrepInput, SearchCaps, build_glob_command,
        build_grep_command, parse_glob_args, parse_grep_args, ripgrep_argv, run_ripgrep,
    };
    use dsh_session::CallId;
    use dsh_subprocess::LocalSubprocessRuntime;
    use dsh_tools::{
        AbortFlag, ToolError, ToolExecutionInput, ToolExecutionResult, ToolPresentationMode,
        ToolRuntime,
    };
    use serde_json::json;

    #[test]
    fn no_config_is_the_first_argument_after_the_binary() {
        let argv = ripgrep_argv("/opt/rg", &["--json".into(), "--regexp=needle".into()]);
        assert_eq!(argv[0], "/opt/rg");
        assert_eq!(argv[1], "--no-config");
        assert_eq!(argv[2], "--json");
        assert_eq!(argv[3], "--regexp=needle");
        let empty = ripgrep_argv("rg", &[]);
        assert_eq!(empty, vec!["rg".to_string(), "--no-config".to_string()]);
    }

    #[test]
    fn grep_and_glob_fixed_argv() {
        assert_eq!(
            build_grep_command(&GrepInput {
                pattern: "needle".into(),
                path: None,
                include: None
            }),
            vec!["--json".to_string(), "--regexp=needle".to_string()]
        );
        let glob = build_glob_command(&GlobInput {
            pattern: "**/*.ts".into(),
            path: None,
        });
        assert_eq!(glob[0], "--files");
        assert_eq!(glob[1], "--glob=**/*.ts");
        assert!(glob.contains(&"--sort=modified".into()));
        assert!(glob.iter().any(|a| a == "--glob=!**/.git"));
        assert!(glob.iter().any(|a| a == "--glob=!**/.git/**"));
    }

    #[test]
    fn ripgrep_argv_never_puts_config_after_model_args() {
        let argv = ripgrep_argv("rg", &["--pre".into(), "evil".into()]);
        assert_eq!(argv[1], "--no-config");
        assert_ne!(argv[0], "--no-config");
    }

    #[test]
    fn glob_command_excludes_every_vcs_name_and_appends_path() {
        let glob = build_glob_command(&GlobInput {
            pattern: "*.rs".into(),
            path: Some("src".into()),
        });
        assert_eq!(
            &glob[0..5],
            &[
                "--files".to_string(),
                "--glob=*.rs".to_string(),
                "--sort=modified".to_string(),
                "--no-ignore".to_string(),
                "--hidden".to_string(),
            ]
        );
        for name in GLOB_VCS_EXCLUDES {
            assert!(
                glob.contains(&format!("--glob=!**/{name}")),
                "missing prune glob for {name}"
            );
            assert!(
                glob.contains(&format!("--glob=!**/{name}/**")),
                "missing contents glob for {name}"
            );
        }
        assert_eq!(glob[glob.len() - 2], "--");
        assert_eq!(glob[glob.len() - 1], "src");
    }

    #[test]
    fn grep_command_optional_include_and_path() {
        assert_eq!(
            build_grep_command(&GrepInput {
                pattern: "needle".into(),
                path: Some("crates".into()),
                include: Some("*.rs".into()),
            }),
            vec![
                "--json".to_string(),
                "--regexp=needle".to_string(),
                "--glob=*.rs".to_string(),
                "--".to_string(),
                "crates".to_string(),
            ]
        );
    }

    #[test]
    fn parse_glob_and_grep_require_pattern() {
        assert!(parse_glob_args(&json!({})).is_err());
        assert!(parse_glob_args(&json!({"pattern": "  "})).is_err());
        let glob = parse_glob_args(&json!({"pattern": "**/*.ts", "path": "src"})).unwrap();
        assert_eq!(glob.pattern, "**/*.ts");
        assert_eq!(glob.path.as_deref(), Some("src"));
        assert!(parse_grep_args(&json!({"pattern": ""})).is_err());
        let grep = parse_grep_args(&json!({"pattern": " ", "include": "*.rs"})).unwrap();
        assert_eq!(grep.pattern, " ");
        assert_eq!(grep.include.as_deref(), Some("*.rs"));
    }

    fn test_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dsh-tool-fs-search-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_script(dir: &std::path::Path, name: &str, body: &str) -> String {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms).unwrap();
        }
        path.to_string_lossy().into_owned()
    }

    fn default_caps() -> SearchCaps {
        SearchCaps::default()
    }

    fn coded(err: &ToolError) -> (&str, &str) {
        match err {
            ToolError::Coded { name, code, .. } => (name.as_str(), code.as_str()),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn run_ripgrep_puts_no_config_at_argv_1() {
        let dir = test_dir();
        let binary = write_script(&dir, "show-arg1", r#"printf '%s\n' "$1""#);
        let runtime = LocalSubprocessRuntime::new();
        let out = run_ripgrep(
            &runtime,
            &binary,
            &[],
            dir.to_str().unwrap(),
            &default_caps(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(out.trim(), "--no-config");
    }

    #[tokio::test]
    async fn run_ripgrep_exit_1_is_success() {
        let dir = test_dir();
        let binary = write_script(&dir, "exit-one", "exit 1");
        let runtime = LocalSubprocessRuntime::new();
        let out = run_ripgrep(
            &runtime,
            &binary,
            &[],
            dir.to_str().unwrap(),
            &default_caps(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(out, "");
    }

    #[tokio::test]
    async fn run_ripgrep_invalid_pattern_from_stderr() {
        let dir = test_dir();
        let binary = write_script(
            &dir,
            "bad-pattern",
            "printf '%s\n' 'Regex Parse Error: unclosed group' >&2; exit 2",
        );
        let runtime = LocalSubprocessRuntime::new();
        let err = run_ripgrep(
            &runtime,
            &binary,
            &[],
            dir.to_str().unwrap(),
            &default_caps(),
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(coded(&err), ("SearchError", "SEARCH_INVALID_PATTERN"));
    }

    #[tokio::test]
    async fn run_ripgrep_other_nonzero_is_search_failed() {
        let dir = test_dir();
        let binary = write_script(&dir, "fail", "printf '%s\n' boom >&2; exit 2");
        let runtime = LocalSubprocessRuntime::new();
        let err = run_ripgrep(
            &runtime,
            &binary,
            &[],
            dir.to_str().unwrap(),
            &default_caps(),
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(coded(&err), ("SearchError", "SEARCH_FAILED"));
    }

    #[tokio::test]
    async fn run_ripgrep_lossy_stdout_is_overflow() {
        let dir = test_dir();
        let binary = write_script(&dir, "huge", "dd if=/dev/zero bs=1024 count=8 2>/dev/null");
        let runtime = LocalSubprocessRuntime::new();
        let caps = SearchCaps {
            raw_output_max_bytes: 16,
            ..SearchCaps::default()
        };
        let err = run_ripgrep(&runtime, &binary, &[], dir.to_str().unwrap(), &caps, None)
            .await
            .unwrap_err();
        assert_eq!(coded(&err), ("SearchError", "SEARCH_RAW_OUTPUT_OVERFLOW"));
    }

    #[tokio::test]
    async fn run_ripgrep_aborted_flag_is_search_aborted() {
        let dir = test_dir();
        let runtime = LocalSubprocessRuntime::new();
        let signal = AbortFlag::new();
        signal.abort();
        let err = run_ripgrep(
            &runtime,
            "/bin/true",
            &[],
            dir.to_str().unwrap(),
            &default_caps(),
            Some(signal),
        )
        .await
        .unwrap_err();
        assert_eq!(coded(&err), ("SearchError", "SEARCH_ABORTED"));
    }

    #[tokio::test]
    async fn live_rg_lists_cargo_toml_when_available() {
        let runtime = LocalSubprocessRuntime::new();
        if runtime.resolve_executable("rg", None).await.is_err() {
            return;
        }
        let cwd = env!("CARGO_MANIFEST_DIR");
        let rest = vec!["--files".into(), "--glob=Cargo.toml".into()];
        let out = run_ripgrep(&runtime, "rg", &rest, cwd, &default_caps(), None)
            .await
            .unwrap();
        assert!(out.contains("Cargo.toml"), "expected Cargo.toml in {out:?}");
    }

    #[tokio::test]
    async fn glob_and_grep_are_registered() {
        let dir = test_dir();
        let ctx = crate::FsToolContext {
            fs: std::sync::Arc::new(dsh_fs::LocalFileSystem::new(&dir)),
            gate: std::sync::Arc::new(dsh_fs::ObservationGate::new()),
            owner: dsh_fs::ObservationOwner(1),
            sandbox: None,
            subprocess: std::sync::Arc::new(LocalSubprocessRuntime::new()),
            rg_binary: "rg".into(),
        };
        let mut tools = ToolRuntime::new(ToolPresentationMode::Native);
        crate::register_fs_tools(&mut tools, ctx);
        let glob = tools
            .execute(ToolExecutionInput {
                call_id: CallId::new("c1"),
                root_call_id: None,
                name: "glob".into(),
                arguments: json!({}),
                parent: None,
                signal: AbortFlag::new(),
            })
            .await;
        match glob {
            ToolExecutionResult::Failure { error, .. } => {
                assert_ne!(
                    error.info.as_ref().map(|info| info.code.as_str()),
                    Some("UNKNOWN_TOOL")
                );
            }
            other => panic!("{other:?}"),
        }
        let grep = tools
            .execute(ToolExecutionInput {
                call_id: CallId::new("c2"),
                root_call_id: None,
                name: "grep".into(),
                arguments: json!({}),
                parent: None,
                signal: AbortFlag::new(),
            })
            .await;
        match grep {
            ToolExecutionResult::Failure { error, .. } => {
                assert_ne!(
                    error.info.as_ref().map(|info| info.code.as_str()),
                    Some("UNKNOWN_TOOL")
                );
            }
            other => panic!("{other:?}"),
        }
    }
}
