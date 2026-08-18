# dsh-base

English | [中文](README.zh.md)

Phase 6 product plugin aggregator for the DeepSeek Harness Rust host. `register_base_plugins` registers approval (including `headless-auto-approve`), permission presets, llm-retry plus the retry snapshot backend, token-meter, compaction-basic plus the tool-result pruner, agent-instructions, time-context, the skill trio, web plus DeepSeek search plus tool-web, jobs-local plus tool-jobs, subagent plus in-process spawn/fork, and the four `dsh-tool-subagent` YAML names.

This function does not register spine, execution, headless, or `sdk-jsonrpc-server` plugins. `dsh-agent` does not depend on `dsh-subagent`; the CLI and jsonrpc bins call `register_base_plugins` so those YAML names exist. Default YAML when `DSH_CORDIS_CONFIG` is unset remains `MINIMAL_YAML`. Static Phase 6 trees are `dsh-headless/base.cordis.yml` and `dsh-sdk-jsonrpc-server/base.cordis.yml` (`BASE_YAML`); those files omit time-context, the pruner, retry-snapshot-backend, workflow, goal, session-title, settings-file, and commands, and they contain no `!!js`. Unknown YAML names still fail loud. `register_base_plugins` maps `@deepseek-ai/dsh-mcp-client` and the three PTY YAML names `@deepseek-ai/dsh-terminal`, `@deepseek-ai/dsh-terminal-bash`, and `@deepseek-ai/dsh-tool-terminal` plus `pty-snapshot-backend`; default YAML does not mount an MCP server or PTY rows and must not allocate a PTY.

## Known Limitations and Deferred Work

- Unset `DSH_CORDIS_CONFIG` still boots `MINIMAL_YAML`; `BASE_YAML` is used when that variable points at the file.
- `web_fetch` stays off (`fetch: false`).
- A `standard` profile is not implemented.
