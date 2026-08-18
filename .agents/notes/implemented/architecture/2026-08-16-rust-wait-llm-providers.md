# Agent Note: Bounded wait for LLM providers after headless-runner spawn

Status: implemented

English | [中文](2026-08-16-rust-wait-llm-providers.zh.md)

## Problem

`headless-runner` setup `tokio::spawn`s `run` and returns so later YAML plugins can mount. `run` then reads `AgentRegistry::list_providers()` before `create` or `resume`. Sibling adapter plugins may still be inside `register_adapter`, so that first poll can be empty and the process exits 1 with `plugin setup failed: no LLM provider registered`.

`boot_yaml` starts every row's plugin fiber, then `await_ready`. `@deepseek-ai/dsh-tool-subagent` injects `subagents` and calls `get_provider` while `@deepseek-ai/dsh-subagent-spawn-in-process` may still be in `register_provider`, so load fails with `tool-subagent: unknown provider "spawn"`.

## Decision

Before `create` or `resume`, `run_inner` calls private `wait_until_providers`, which polls `list_providers()` until the list is non-empty or 64 yield-plus-1ms attempts elapse. Empty after the bound still fails with `no LLM provider registered`. `AgentRegistry` does not grow a wait method.

`@deepseek-ai/dsh-tool-subagent` setup calls private `wait_until_named_provider` with the same bound before `register_delegate_tool`. Empty after the bound still fails with `tool-subagent: unknown provider "{name}"`. JSON-RPC `initialize` does not poll `list_providers`: the bin calls `serve` after `boot_yaml` returns.

## Testing

`wait_until_providers_sees_ids_after_empty_polls` returns ids that appear after empty polls. `wait_until_providers_fails_when_empty_after_bound` fails with `no LLM provider registered` when every poll is empty. `wait_until_sees_ready_after_empty_polls` and `wait_until_named_provider_fails_when_missing_after_bound` pin the delegate wait.

## Alternatives considered

**Return from `headless-runner` setup only after `run` finishes.** Rejected: setup must return so remaining YAML plugins can mount; that is why `run` is spawned.

**Mount YAML rows strictly in list order.** Rejected: `ctx.plugin` plus `await_ready` is the existing boot; a second sequential mount would change every plugin's overlap assumptions.

**Wait forever until a provider appears.** Rejected: a missing adapter or subagent provider must still fail loud.

**Add `AgentRegistry::wait_for_providers` or `SubagentRuntime::wait_for_provider`.** Rejected: the helpers stay private in `dsh-headless` and `dsh-tool-subagent`.

**Sleep a fixed duration once.** Rejected: a single sleep still loses under load and delays the path where registrations are already visible.

## Consequences

A host that never registers an LLM adapter or the named subagent provider waits up to the bound (about 64ms) before the same setup error. JSON-RPC `initialize` still fails immediately when the named LLM provider is absent after a successful boot.

## Related

JSON-RPC serves after `boot_yaml` so `initialize` sees sibling `register_adapter` ([Rust SDK JSON-RPC server](2026-08-16-rust-sdk-jsonrpc-server.md)). Headless consumer text is on [dsh-headless](../../../../crates/dsh-headless/README.md). Delegate load is on [dsh-tool-subagent](../../../../crates/dsh-tool-subagent/README.md).
