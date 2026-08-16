# dsh-tool-subagent

English | [中文](README.zh.md)

Model-facing `subagent`, `subagent_fork`, `send_message`, `list_agents`, and `report` over [`dsh-subagent`](../dsh-subagent/README.md). Four YAML names: `@deepseek-ai/dsh-tool-subagent`, `@deepseek-ai/dsh-tool-subagent-control`, `@deepseek-ai/dsh-tool-subagent-list`, and `@deepseek-ai/dsh-tool-subagent-report`. This crate is not added to `register_spine_plugins` and is not mounted in `base.cordis.yml`.

A `@deepseek-ai/dsh-tool-subagent` row binds one `provider` to one `toolName`. `backgroundMode: continuable` with `run_in_background` default true calls `start_continuable_background` and returns `{ kind: "continuable", subagentId }` immediately. `backgroundMode: one-shot` (the default) waits for `SubagentRuntime::start`; `run_in_background: true` is a tool error until jobs are wired. Fork stays one-shot: configure `{ provider: fork, toolName: subagent_fork, backgroundMode: one-shot }`. Calling identity is `ToolExecution.session_id`.

`report` registers only through `SubagentRuntime::register_continuable_setup` onto a cloned child `ToolRuntime`. It is never registered on the parent's shared runtime, so one-shot fork children do not see `report`. `{ output }` delivers a user-role `MessageSource::SubagentReport` via `followup` (`wakeup`) or `inject` (`quiet`) and returns `{ messageId }`. `send_message` `{ subagent_id, message }` calls `followup_child` with `MessageSource::Coordinator`. `list_agents` lists continuable children only; one-shot fork runs do not appear. Presentation for `report` is generic.

## Config

### `@deepseek-ai/dsh-tool-subagent`

| Key | Default | Meaning |
|---|---|---|
| `provider` | required | `subagents` provider name. |
| `toolName` | `subagent` | Model-facing tool name. |
| `enableRunInBackground` | `true` | Expose `run_in_background`; `false` omits it and rejects forced background calls. |
| `backgroundMode` | `one-shot` | `one-shot` waits for `start`; `continuable` defaults to background and returns the child id. |
| `maxDepth` | `3` | Absolute depth cap, or `"provider-managed"` to send no cap. A numeric cap requires `depthLimit`. |

### `@deepseek-ai/dsh-tool-subagent-report`

| Key | Default | Meaning |
|---|---|---|
| `reportDelivery` | `wakeup` | `wakeup` uses `followup`; `quiet` uses `inject`. |

Control and list accept no keys. Unknown keys fail load. Missing `provider` fails load.

## Model Experience

The model sees `subagent` / `subagent_fork` (per instance name), and `send_message`, `list_agents`, and `report` when those plugins are mounted. Continuable starts return `started subagent {id}`. One-shot success returns the child's final text. Settlement arrives as a `subagent-settled` notice, independently of `report`.

#### KV Cache effect

Append-only tool results and notices after the reusable request prefix.

## Known Limitations and Deferred Work

- One-shot `run_in_background: true` is unavailable until a jobs service is wired.
- Out-of-process providers, `toolFilter`, `persona`, and `agentOptions` are not ported.
- The plugins are not mounted in `base.cordis.yml` in this phase.
