//! Closed plugin-name registry and YAML mount for the Rust host.

mod error;
mod mount;
mod registry;

#[cfg(test)]
mod phase5_exit;

#[cfg(test)]
mod phase8_exit;

#[cfg(test)]
mod phase8_mcp_exit;

pub use error::BootError;
pub use mount::{boot_yaml, mount_entries, process_interpolate_env};
pub use registry::{PluginRegistry, PluginSetup};

/// YAML `name` for `@deepseek-ai/dsh-credentials`.
pub const PLUGIN_CREDENTIALS: &str = "@deepseek-ai/dsh-credentials";
/// YAML `name` for `@deepseek-ai/dsh-llm`.
pub const PLUGIN_LLM: &str = "@deepseek-ai/dsh-llm";
/// YAML `name` for `@deepseek-ai/dsh-llm-deepseek`.
pub const PLUGIN_LLM_DEEPSEEK: &str = "@deepseek-ai/dsh-llm-deepseek";
/// YAML `name` for `@deepseek-ai/dsh-llm-mock`.
pub const PLUGIN_LLM_MOCK: &str = "@deepseek-ai/dsh-llm-mock";
/// YAML `name` for `@deepseek-ai/dsh-llm-replay`.
pub const PLUGIN_LLM_REPLAY: &str = "@deepseek-ai/dsh-llm-replay";
/// YAML `name` for `@deepseek-ai/dsh-llm-retry`.
pub const PLUGIN_LLM_RETRY: &str = "@deepseek-ai/dsh-llm-retry";
/// YAML `name` for the deterministic retry snapshot adapter.
pub const PLUGIN_RETRY_SNAPSHOT_BACKEND: &str = "retry-snapshot-backend";
/// YAML `name` for `@deepseek-ai/dsh-token-meter`.
pub const PLUGIN_TOKEN_METER: &str = "@deepseek-ai/dsh-token-meter";
/// YAML `name` for `@deepseek-ai/dsh-compaction-basic`.
pub const PLUGIN_COMPACTION_BASIC: &str = "@deepseek-ai/dsh-compaction-basic";
/// YAML `name` for `@deepseek-ai/dsh-agent-instructions`.
pub const PLUGIN_AGENT_INSTRUCTIONS: &str = "@deepseek-ai/dsh-agent-instructions";
/// YAML `name` for `@deepseek-ai/dsh-time-context`.
pub const PLUGIN_TIME_CONTEXT: &str = "@deepseek-ai/dsh-time-context";
/// YAML `name` for `@deepseek-ai/dsh-skill`.
pub const PLUGIN_SKILL: &str = "@deepseek-ai/dsh-skill";
/// YAML `name` for `@deepseek-ai/dsh-skill-filesystem`.
pub const PLUGIN_SKILL_FILESYSTEM: &str = "@deepseek-ai/dsh-skill-filesystem";
/// YAML `name` for `@deepseek-ai/dsh-tool-skill`.
pub const PLUGIN_TOOL_SKILL: &str = "@deepseek-ai/dsh-tool-skill";
/// YAML `name` for `@deepseek-ai/dsh-web`.
pub const PLUGIN_WEB: &str = "@deepseek-ai/dsh-web";
/// YAML `name` for `@deepseek-ai/dsh-web-search-deepseek`.
pub const PLUGIN_WEB_SEARCH_DEEPSEEK: &str = "@deepseek-ai/dsh-web-search-deepseek";
/// YAML `name` for `@deepseek-ai/dsh-tool-web`.
pub const PLUGIN_TOOL_WEB: &str = "@deepseek-ai/dsh-tool-web";
/// YAML `name` for `@deepseek-ai/dsh-jobs-local`.
pub const PLUGIN_JOBS_LOCAL: &str = "@deepseek-ai/dsh-jobs-local";
/// YAML `name` for `@deepseek-ai/dsh-tool-jobs`.
pub const PLUGIN_TOOL_JOBS: &str = "@deepseek-ai/dsh-tool-jobs";
/// YAML `name` for `@deepseek-ai/dsh-compaction-tool-result-pruner`.
pub const PLUGIN_TOOL_RESULT_PRUNER: &str = "@deepseek-ai/dsh-compaction-tool-result-pruner";
/// YAML `name` for `@deepseek-ai/dsh-tools`.
pub const PLUGIN_TOOLS: &str = "@deepseek-ai/dsh-tools";
/// YAML `name` for `@deepseek-ai/dsh-user-approval`.
pub const PLUGIN_USER_APPROVAL: &str = "@deepseek-ai/dsh-user-approval";
/// YAML `name` for `@deepseek-ai/dsh-permission-presets`.
pub const PLUGIN_PERMISSION_PRESETS: &str = "@deepseek-ai/dsh-permission-presets";
/// YAML `name` for `@deepseek-ai/dsh-system-prompt`.
pub const PLUGIN_SYSTEM_PROMPT: &str = "@deepseek-ai/dsh-system-prompt";
/// YAML `name` for `@deepseek-ai/dsh-agent`.
pub const PLUGIN_AGENT: &str = "@deepseek-ai/dsh-agent";
/// YAML `name` for `@deepseek-ai/dsh-subagent`.
pub const PLUGIN_SUBAGENT: &str = "@deepseek-ai/dsh-subagent";
/// YAML `name` for `@deepseek-ai/dsh-subagent-spawn-in-process`.
pub const PLUGIN_SUBAGENT_SPAWN: &str = "@deepseek-ai/dsh-subagent-spawn-in-process";
/// YAML `name` for `@deepseek-ai/dsh-subagent-fork-in-process`.
pub const PLUGIN_SUBAGENT_FORK: &str = "@deepseek-ai/dsh-subagent-fork-in-process";
/// YAML `name` for `@deepseek-ai/dsh-tool-subagent`.
pub const PLUGIN_TOOL_SUBAGENT: &str = "@deepseek-ai/dsh-tool-subagent";
/// YAML `name` for `@deepseek-ai/dsh-tool-subagent-control`.
pub const PLUGIN_TOOL_SUBAGENT_CONTROL: &str = "@deepseek-ai/dsh-tool-subagent-control";
/// YAML `name` for `@deepseek-ai/dsh-tool-subagent-list`.
pub const PLUGIN_TOOL_SUBAGENT_LIST: &str = "@deepseek-ai/dsh-tool-subagent-list";
/// YAML `name` for `@deepseek-ai/dsh-tool-subagent-report`.
pub const PLUGIN_TOOL_SUBAGENT_REPORT: &str = "@deepseek-ai/dsh-tool-subagent-report";
/// YAML `name` for `@deepseek-ai/dsh-session-persistence-jsonl`.
pub const PLUGIN_SESSION_JSONL: &str = "@deepseek-ai/dsh-session-persistence-jsonl";
/// YAML `name` for `@deepseek-ai/dsh-subprocess-local`.
pub const PLUGIN_SUBPROCESS: &str = "@deepseek-ai/dsh-subprocess-local";
/// YAML `name` for `@deepseek-ai/dsh-fs-local`.
pub const PLUGIN_FS: &str = "@deepseek-ai/dsh-fs-local";
/// YAML `name` for `@deepseek-ai/dsh-shell-bash-local`.
pub const PLUGIN_SHELL: &str = "@deepseek-ai/dsh-shell-bash-local";
/// YAML `name` for `@deepseek-ai/dsh-tool-fs`.
pub const PLUGIN_TOOL_FS: &str = "@deepseek-ai/dsh-tool-fs";
/// YAML `name` for `@deepseek-ai/dsh-tool-bash`.
pub const PLUGIN_TOOL_BASH: &str = "@deepseek-ai/dsh-tool-bash";
/// YAML `name` for the headless startup plugin.
pub const PLUGIN_HEADLESS_STARTUP: &str = "headless-startup";
/// YAML `name` for the headless runner plugin.
pub const PLUGIN_HEADLESS_RUNNER: &str = "headless-runner";
/// YAML `name` for the headless auto-approve waterfall listener.
pub const PLUGIN_HEADLESS_AUTO_APPROVE: &str = "headless-auto-approve";
/// YAML `name` for the SDK JSON-RPC server plugin.
pub const PLUGIN_SDK_JSONRPC: &str = "sdk-jsonrpc-server";
/// YAML `name` for `@deepseek-ai/dsh-acp`.
pub const PLUGIN_ACP: &str = "@deepseek-ai/dsh-acp";
/// YAML `name` for `@deepseek-ai/dsh-mcp-client`.
pub const PLUGIN_MCP_CLIENT: &str = "@deepseek-ai/dsh-mcp-client";
/// YAML `name` for `@deepseek-ai/dsh-terminal`.
pub const PLUGIN_TERMINAL: &str = "@deepseek-ai/dsh-terminal";
/// YAML `name` for `@deepseek-ai/dsh-terminal-bash`.
pub const PLUGIN_TERMINAL_BASH: &str = "@deepseek-ai/dsh-terminal-bash";
/// YAML `name` for `@deepseek-ai/dsh-tool-terminal`.
pub const PLUGIN_TOOL_TERMINAL: &str = "@deepseek-ai/dsh-tool-terminal";
/// YAML `name` for the deterministic PTY snapshot backend.
pub const PLUGIN_PTY_SNAPSHOT_BACKEND: &str = "pty-snapshot-backend";
/// YAML `name` for `@deepseek-ai/dsh-host-webserver`.
pub const PLUGIN_HOST_WEBSERVER: &str = "@deepseek-ai/dsh-host-webserver";
/// YAML `name` for `@deepseek-ai/dsh-host-frontend-static`.
pub const PLUGIN_FRONTEND_STATIC: &str = "@deepseek-ai/dsh-host-frontend-static";
/// YAML `name` for `@deepseek-ai/dsh-client-modules`.
pub const PLUGIN_CLIENT_MODULES: &str = "@deepseek-ai/dsh-client-modules";
/// YAML `name` for `@deepseek-ai/dsh-workspace`.
pub const PLUGIN_WORKSPACE: &str = "@deepseek-ai/dsh-workspace";
/// YAML `name` for `@deepseek-ai/dsh-settings`.
pub const PLUGIN_SETTINGS: &str = "@deepseek-ai/dsh-settings";
/// YAML `name` for `@deepseek-ai/dsh-commands`.
pub const PLUGIN_COMMANDS: &str = "@deepseek-ai/dsh-commands";
/// YAML `name` for the web-app plugin.
pub const PLUGIN_WEB_APP: &str = "web-app";
