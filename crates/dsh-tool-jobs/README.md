# dsh-tool-jobs

English | [中文](README.zh.md)

Model-facing `job_output`, `job_list`, and `job_kill` over [`dsh-jobs-local`](../dsh-jobs-local/README.md). YAML name `@deepseek-ai/dsh-tool-jobs`. This crate is not added to `register_spine_plugins` and is not mounted in `base.cordis.yml`. If `agents` is not provided, completion notices are skipped and the plugin still loads. Tools authorize with `ToolExecution.session_id`; `None` sees only unowned jobs.

Argument names are `id`, not `job_id`. `job_output` is `{ id, wait?, timeout_ms? }`. `job_list` is `{}`. `job_kill` is `{ id }`. Empty `id` fails validation. Unknown id is a tool error containing `unknown job`. Reads render the body or `(no new output)`, then `[status: …]`. Kill of live work returns `requested cancellation of job {id}`. Kill of an already-terminal job returns already-finished text. A timed-out `wait: true` returns the current snapshot and is not a tool error.

Unreported owned completions use `MessageSource::Plugin { plugin: "tool-jobs", form: Some("notice"), compaction_id: None, source_command_id: None }`. An idle owner is woken with `AgentHandle::followup` under default `wakeup` delivery; a busy owner is `inject`ed. After `maxConsecutiveWakes` consecutive wakes (default 3), notices degrade to `inject`. `AgentHandle::lock()` is used only synchronously. User-authored pre-step input resets the wake budget.

## Config

| Key | Default | Meaning |
|---|---|---|
| `waitTimeoutMs` | `30000` | Wait used when `wait: true` omits `timeout_ms`. |
| `maxWaitTimeoutMs` | `600000` | Cap for model-supplied waits. |
| `completionDelivery` | `wakeup` | `wakeup` opens a turn on an idle owner; `quiet` injects. |
| `maxConsecutiveWakes` | `3` | Turns one owner may open by wake before notices degrade to injection. |

Unknown keys fail load. A default wait above the cap fails at load.

## Model Experience

The model sees `job_output`, `job_list`, and `job_kill`. Results end with `[status: …]`. Unreported owned completions arrive as plugin notices.

#### KV Cache effect

Append-only tool results and notices after the reusable request prefix.

## Known Limitations and Deferred Work

- The `tool:jobs` system-prompt section is omitted until `SystemPrompt` accepts post-provide registration.
- Isolate and preset controller layers are not implemented.
- The plugin is not mounted in `base.cordis.yml` in this phase.
