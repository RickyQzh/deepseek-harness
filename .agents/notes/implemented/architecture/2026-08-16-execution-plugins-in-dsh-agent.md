# Agent Note: Execution YAML plugins register from dsh-agent as a sibling of spine

Status: implemented

English | [中文](2026-08-16-execution-plugins-in-dsh-agent.zh.md)

## Problem

Phase 5 needs YAML-named subprocess, fs, shell, and model-facing fs/bash tools. Folding those setups into `register_spine_plugins` would make every spine boot depend on the execution crates. Putting those `register` calls in `dsh-boot` would add product crate dependencies and recreate the cycle [Spine YAML plugins](2026-08-16-spine-plugins-in-dsh-agent.md) exists to prevent.

## Decision

Each execution crate depends on `dsh-boot` and `dsh-kernel` and exports `plugin::register`. `dsh_agent::register_execution_plugins` is a sibling of `register_spine_plugins`: callers that want bash and fs tools call both. `dsh-boot` stays free of product crate dependencies. YAML names stay the closed constants already in `dsh-boot` (`@deepseek-ai/dsh-subprocess-local`, `@deepseek-ai/dsh-fs-local`, `@deepseek-ai/dsh-shell-bash-local`, `@deepseek-ai/dsh-tool-fs`, `@deepseek-ai/dsh-tool-bash`).

`Context::provide(name, value: T)` stores `T`; `inject::<T>()` and `get::<T>()` return `Arc<T>`. Execution plugins provide the inner value (`LocalSubprocessRuntime`, `LocalFileSystem`, `LocalBashExecutor`) and inject that type so constructors receive a single Arc.

`@deepseek-ai/dsh-fs-local` cwd is config `cwd`, else `DSH_CWD`, else the process cwd. `@deepseek-ai/dsh-shell-bash-local` injects `subprocess` and provides unfenced `shell` (`BashConfig::default()`). `@deepseek-ai/dsh-tool-fs` injects `tools`, `fs`, and `subprocess`, then `register_fs_tools` with `ObservationOwner(1)`, a process-wide `ObservationGate`, `sandbox: None`, and `rg_binary: "rg"`. `@deepseek-ai/dsh-tool-bash` injects `tools` and `shell` and calls `register_bash_tool(runtime, shell, None)`. `ToolRuntime::registered_names` returns the sorted model-facing names.

## Alternatives considered

**Fold execution into `register_spine_plugins`.** Rejected: spine YAML (credentials, llm, tools, agent, sessions) must boot without pulling subprocess, fs, shell, or tool crates, and later bins call the sibling explicitly.

**Put `register_execution_plugins` in `dsh-boot`.** Rejected: that is the same product-dependency cycle as putting spine composition in `dsh-boot`.

**`provide(Arc::new(T))` then `inject::<Arc<T>>`.** Rejected: `provide` already wraps `T` in Arc, so that stores `Arc<Arc<T>>` and constructors that want `Arc<T>` do not type-check.

**Per-agent `ObservationOwner`.** Rejected for this phase: TypeScript actor identity is not mounted; one process-wide `ObservationOwner(1)` is the accepted gap.

## Consequences

Headless and JSON-RPC bins that want bash and fs call `register_spine_plugins` then `register_execution_plugins`. An unknown execution YAML name still fails loud. Phase 5 bash is unfenced. Filesystem observation is process-wide owner 1.

`cargo test -p dsh-agent --offline execution_yaml_registers_bash` boots the spine plus execution YAML list and asserts `registered_names` includes `bash` and `read`.

## Related

Spine composition is [Spine YAML plugins register from dsh-agent, not dsh-boot](2026-08-16-spine-plugins-in-dsh-agent.md). The program-level rewrite proposal is [Rewrite core and backend in Rust](../../proposed/architecture/2026-08-14-rust-rewrite.md).
