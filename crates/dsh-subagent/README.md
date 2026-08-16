# dsh-subagent

English | [中文](README.zh.md)

Subagent Service Definition for the DeepSeek Harness Rust host: a named-provider registry on `subagents` (`SubagentRuntime`) and a capability-checking one-shot `start`. YAML name `@deepseek-ai/dsh-subagent`. This crate is not added to `register_spine_plugins` and is not mounted in `base.cordis.yml`. In-process spawn and fork providers live in [`dsh-subagent-in-process`](../dsh-subagent-in-process/README.md). `dsh-agent` does not depend on this crate.

`inject::<SubagentRuntime>()` yields `Arc<SubagentRuntime>`, so `register_provider` and `start` take `&self` with an interior mutex. Duplicate provider names fail. Unknown names fail. `start` checks advertised start-time capabilities, then delegates to the named provider; it does not enter continuation. `prepare_continuable` is a data stub on each provider and is not called from one-shot `start`.

`subagent/descriptor` version 2 is a log-only session event (`SessionEvent::SubagentDescriptor { data }`). Kernel events `subagent/start` and `subagent/end` are not session events. A published run that emits start must also emit end, including when the child turn fails after publication. Depth default is 3; `assert_subagent_max_depth` fails when parent `delegationDepth` (absent = 0) plus one exceeds the cap.

## Config

None. Unknown keys fail load.

## Model Experience

Indirectly through in-process providers and a later model-facing tool. This registry contributes no tool schema.

#### KV Cache effect

No direct invalidation.

## Known Limitations and Deferred Work

- Continuable children and `startContinuable` are not implemented in this phase.
- Out-of-process providers (ACP, Codex, Claude Code, SDK) are not ported.
- The plugin is not mounted in `base.cordis.yml` in this phase.
