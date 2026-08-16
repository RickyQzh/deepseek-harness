# Agent Note: Spine YAML plugins register from dsh-agent, not dsh-boot

Status: implemented

English | [中文](2026-08-16-spine-plugins-in-dsh-agent.zh.md)

## Problem

`PluginRegistry` and the closed YAML `name` constants live in `dsh-boot`. Each product plugin's `register` needs that type. If `dsh-boot` also depended on those product crates to call `register`, Cargo would form a cycle (`dsh-boot` → product crates → `dsh-boot`).

## Decision

Product crates depend on `dsh-boot` and `dsh-kernel` and export `plugin::register` (llm also exports `plugin::register_llm`, `plugin::register_mock`, and `replay::register`). `dsh_agent::register_spine_plugins` is the spine composition function that registers credentials, llm (mock, replay, DeepSeek), tools, system-prompt, agent, and the JSONL session store. `dsh-boot` stays free of product crate dependencies; its tests keep a probe plugin only.

YAML names stay the closed constants in `dsh-boot` (`@deepseek-ai/dsh-credentials`, `@deepseek-ai/dsh-llm`, `@deepseek-ai/dsh-llm-deepseek`, `@deepseek-ai/dsh-llm-mock`, `@deepseek-ai/dsh-llm-replay`, `@deepseek-ai/dsh-tools`, `@deepseek-ai/dsh-system-prompt`, `@deepseek-ai/dsh-agent`, `@deepseek-ai/dsh-session-persistence-jsonl`). Setup waits with `ctx.inject` inside the plugin fiber; `ctx.plugin` does not take inject names. `inject` of an already-provided service returns as soon as the slot exists; it does not wait for sibling plugins that still `register_adapter` or `register` on that mutex. `@deepseek-ai/dsh-agent` therefore stores the injected `llm` and `tools` mutex Arcs on `AgentRegistry` rather than cloning the inner maps at setup.

The DeepSeek plugin stores a `CredentialRef` on `DeepSeekConnectionOptions` and resolves the secret per `stream` through `LayeredCredentials`; it never stores the raw key on the adapter. Replay shares one `Arc<ReplayAdapter>` across configured provider ids and pops `assistant/chunk` runs from `DSH_SNAPSHOT_FILE`.

## Alternatives considered

**Put `register_spine_plugins` in `dsh-boot` with product crate dependencies.** Rejected: that is the cycle this note exists to prevent.

**Duplicate `register_spine_plugins` in `dsh-cli` and `dsh-sdk-jsonrpc-server`.** Rejected: two copies would drift, and those bins do not exist yet as composition owners.

**Extract a new `dsh-product` crate.** Rejected for this phase: the task forbids extra crates, and `dsh-agent` already depends on llm, tools, and system-prompt.

**Keep product crates free of `dsh-boot` by passing YAML names as string literals.** Rejected: `register` still takes `&mut PluginRegistry`, which is defined in `dsh-boot`.

## Consequences

Headless and JSON-RPC bins call `dsh_agent::register_spine_plugins` rather than importing every spine crate. Adding a spine plugin means a `register` in its crate plus one call in `dsh-agent`. This function does not register execution plugins (subprocess, fs, shell, tool-fs, tool-bash); those names are registered by the sibling `register_execution_plugins` ([Execution YAML plugins](2026-08-16-execution-plugins-in-dsh-agent.md)).

`cargo test -p dsh-agent --offline spine` boots a mock YAML list and asserts `credentials`, `llm`, `tools`, `systemPrompt`, `agents`, and `sessions`, and that `AgentRegistry::list_providers` includes `mock`. An unknown YAML name still fails loud. That YAML list does not mount `@deepseek-ai/dsh-llm-replay` or `@deepseek-ai/dsh-llm-deepseek`.

## Related

The program-level rewrite proposal is [Rewrite core and backend in Rust](../../proposed/architecture/2026-08-14-rust-rewrite.md).
