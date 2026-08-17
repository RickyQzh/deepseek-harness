# Agent Note: Phase 6 product plugins register from dsh-base, not dsh-agent

Status: implemented

English | [中文](2026-08-16-rust-dsh-base-plugins.zh.md)

## Problem

Phase 6 product plugins each export `register`, but they must not join `register_spine_plugins` or `register_execution_plugins`. Putting that aggregator on `dsh-agent` would make `dsh-agent` depend on `dsh-subagent`. Putting it on `dsh-boot` would recreate the product-crate cycle [Spine YAML plugins](2026-08-16-spine-plugins-in-dsh-agent.md) exists to prevent.

Default CLI and jsonrpc boots must keep Phase 5 `MINIMAL_YAML` when `DSH_CORDIS_CONFIG` is unset so `headless-ok` stays green.

## Decision

`crates/dsh-base` owns `register_base_plugins`. It registers approval (`register` and `register_auto_approve`), permission presets, llm-retry plus `register_retry_snapshot_backend`, token-meter, compaction-basic plus `register_pruner`, agent-instructions, time-context, the skill trio, web plus DeepSeek search plus tool-web, jobs-local plus tool-jobs, subagent plus in-process spawn/fork, `dsh_tool_subagent::plugin::register` (four YAML names), and `dsh_mcp_client::register_mcp_plugins` (`@deepseek-ai/dsh-mcp-client`). It does not call spine, execution, headless, or `sdk-jsonrpc-server` register functions.

`dsh-cli` and `dsh-sdk-jsonrpc-server` depend on `dsh-base` and call `register_base_plugins` after spine and execution. `dsh-headless` does not; a test-only `dev-dependency` boots `BASE_YAML`. `dsh-boot` stays free of product crates. `dsh-base` does not depend on `dsh-headless`, `dsh-cli`, or `dsh-sdk-jsonrpc-server`. Duplicate `headless-auto-approve` registration from `register_headless_plugins` overwrites the same setup.

Static trees are `crates/dsh-headless/base.cordis.yml` and `crates/dsh-sdk-jsonrpc-server/base.cordis.yml` (`BASE_YAML`). They include Phase 5 spine and execution rows, `headless-auto-approve`, user-approval `policy: ask`, the three-row permission-presets table (read-only, workspace-write, danger-full-access), token-meter, compaction-basic, llm-retry, agent-instructions `maxBytes: 65536`, the skill trio, web `searchProvider: deepseek-official`, web-search-deepseek `apiKeyEnv: DEEPSEEK_API_KEY`, tool-web `fetch: false` and `searchTimeoutMs: 60000`, jobs-local and tool-jobs, subagent plus spawn/fork `providerName` rows, two `@deepseek-ai/dsh-tool-subagent` mounts (spawn/continuable `subagent` and fork/one-shot `subagent_fork`), then control, list (`PLUGIN_TOOL_SUBAGENT_LIST`), and report. Headless YAML ends with `headless-startup` then `headless-runner` and uses mock text `base-ok`. Jsonrpc YAML mounts `sdk-jsonrpc-server` instead, claims mock `provider: deepseek-official`, and omits headless-startup/runner. Neither file contains `!!js`. Time-context, the pruner, retry-snapshot-backend, workflow, goal, session-title, settings-file, commands, `@deepseek-ai/dsh-compaction`, `@deepseek-ai/dsh-jobs`, and `@deepseek-ai/dsh-mcp-client` are omitted from those files. Unset `DSH_CORDIS_CONFIG` still loads `MINIMAL_YAML`.

## Alternatives considered

**Put `register_base_plugins` on `dsh-agent`.** Rejected: `dsh-agent` must not depend on `dsh-subagent`.

**Put `register_base_plugins` on `dsh-boot`.** Rejected: that is the product-dependency cycle spine composition already rejected.

**Duplicate the register list in `dsh-cli` and `dsh-sdk-jsonrpc-server`.** Rejected: two copies would drift; both bins call `dsh_base::register_base_plugins`.

**Switch the unset-`DSH_CORDIS_CONFIG` fallback from `MINIMAL_YAML` to `BASE_YAML`.** Rejected: Phase 5 `headless-ok` / jsonrpc mock must stay the default.

**Port `!!js` interpolators or an env ternary for approval policy.** Rejected: approval policy is literal `ask`.

**Mount time-context, the pruner, retry-snapshot-backend, `@deepseek-ai/dsh-mcp-client`, or `@deepseek-ai/dsh-tool-subagent-control/list-agents` in default base YAML.** Rejected: those names are registered for a later `--patch` or overlay, or are not registered at all; unknown names fail loud. List-agents is `@deepseek-ai/dsh-tool-subagent-list`.

## Consequences

`cargo test -p dsh-base --offline` boots a copy of headless `base.cordis.yml` without `headless-startup` / `headless-runner`, asserts `approval`, `tokenMeter`, `compaction`, `skills`, `web`, `jobs`, `subagents`, and tools `skill` / `web_search` / `subagent` without `web_fetch`, resolves `@deepseek-ai/dsh-mcp-client` from `register_base_plugins` without a default MCP row, and still fails loud on an unknown YAML name. `cargo test -p dsh-headless --offline` keeps `MINIMAL_YAML` green and pins `BASE_YAML` stdout `base-ok\n` with exit 0.

## Related

Spine and execution composition are [Spine YAML plugins](2026-08-16-spine-plugins-in-dsh-agent.md) and [Execution YAML plugins](2026-08-16-execution-plugins-in-dsh-agent.md). The MCP client type is [Freeze the Rust MCP stdio client](../../proposed/architecture/2026-08-17-rust-mcp-client.md). The program-level rewrite proposal is [Rewrite core and backend in Rust](../../proposed/architecture/2026-08-14-rust-rewrite.md).
