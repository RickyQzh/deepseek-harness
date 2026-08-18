# dsh-subagent-in-process

English | [中文](README.zh.md)

In-process one-shot spawn and fork providers plus the shared child driver. Two YAML names: `@deepseek-ai/dsh-subagent-spawn-in-process` (default provider `spawn`) and `@deepseek-ai/dsh-subagent-fork-in-process` (default provider `fork`). This crate is not added to `register_spine_plugins` and is not mounted in `base.cordis.yml`.

Spawn sets `inherits_parent_context = false` and starts a fresh child session. Fork sets `inherits_parent_context = true` and seeds the child with the parent's events through the last `turn/end` inclusive (empty seed when no completed turn). Both advertise every start-time capability as true. One-shot `start` never calls `prepare_continuable`.

`start_in_process_run` mints `SessionId` `sub-{pid}-{nanos}`, stamps `parent_session`, `origin = subagent`, `delegation_depth = parent + 1`, copies parent `cwd` when present, appends descriptor version 2 `{ mode: "one-shot", provider, label }`, resumes through `AgentRegistry::resume` (not `create`, which forces `parent_session: None`), copies parent provider/model/max_tokens from `parent.lock()` without holding that guard across `.await`, `followup`s the prompt on the child, then `run_until_idle` on the child handle only. It never takes the parent driver permit. After `subagent/start`, it emits `subagent/end` when the child turn completes and when `followup` or `run_until_idle` fails (stop reason `Error`). `TurnEndReason::Blocked` maps to `SubagentStopReason::Refusal`. Result `output` is assistant text from the child's own suffix after `seed_length`.

Requires `agents` on the same kernel context. Missing `agents` fails at start.

## Config

| Key | Default | Meaning |
|---|---|---|
| `providerName` | `spawn` or `fork` | Registry name on `subagents`. |

Unknown keys fail load. Empty `providerName` fails load.

## Model Experience

### Child-agent request

#### What the model sees

Spawn delivers the task as the child's only user message. Fork prepends the parent's completed-turn messages, then the task.

#### Token effect

Spawn pays for a new independent context. Fork duplicates the completed parent prefix into the child.

#### KV Cache effect

Independent of the parent request cache.

## Known Limitations and Deferred Work

- Continuable fork/spawn (`prepare_continuable` consumers) are not started from this path.
- `outputSchema`, `toolFilter`, and `persona` are advertised but not applied in this phase.
- The plugins are not mounted in `base.cordis.yml` in this phase.
