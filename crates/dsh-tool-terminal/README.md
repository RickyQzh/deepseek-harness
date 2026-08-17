# dsh-tool-terminal

English | [中文](README.zh.md)

Six model-facing `terminal_*` tools over [`dsh-terminal`](../dsh-terminal/README.md): `terminal_open`, `terminal_send`, `terminal_read`, `terminal_signal`, `terminal_close`, and `terminal_list`. YAML name `@deepseek-ai/dsh-tool-terminal`. Owner identity is `ToolExecution.session_id` (`SessionId`); a missing session fails closed with `terminal tools require an initiating session`. This crate depends on [`dsh-jobs`](../dsh-jobs/README.md) and [`dsh-jobs-local`](../dsh-jobs-local/README.md) for background sends and does not depend on `dsh-agent` or `dsh-acp`. Plugin inject stays `tools` / `terminals` / `systemPrompt`; `jobs` is looked up at execute time. It is not added to `register_spine_plugins` and is not mounted in `base.cordis.yml`.

`enableRunInBackground` default **true** advertises `run_in_background` and appends ` Background mode returns a job id for job_output/job_kill.` to the description. When `jobs` is provided, `run_in_background: true` starts a `pty-send` job and returns `{ kind: "background", jobId }` rendered as `started background job {jobId}`. Missing jobs still returns `background terminal sends require @deepseek-ai/dsh-jobs and @deepseek-ai/dsh-tool-jobs`. When `enableRunInBackground` is false, the schema omits `run_in_background` and a forced `true` fails with `background terminal sends are disabled by tool-terminal configuration`. `submit` defaults to true. Close reason is `model request`. Concurrent `kill` of an in-flight close renders `terminal session {id} was already closing`.

`maxResultBytes` default `256 * 1024`, minimum `64`. Unknown config keys fail load. Null config uses defaults. Bounding is UTF-8 byte-oriented inside `render.rs` (`\n[output truncated]`). Presentation helpers (`present_*`) are pure functions of args and are not stored on `ToolDefinition`.

## Config

| Key | Default | Meaning |
|---|---|---|
| `enableRunInBackground` | `true` | Advertise and accept `run_in_background`. False omits the schema field. |
| `maxResultBytes` | `262144` | UTF-8 cap (minimum `64`) for one complete terminal result. |

```yaml
- name: '@deepseek-ai/dsh-tool-terminal'
  config:
    enableRunInBackground: true
    maxResultBytes: 262144
```

## Model Experience

The model sees the six `terminal_*` tools and this `tool:pty` section (order 106):

```markdown
Use a terminal session only when work needs persistent terminal state or interactive stdin; prefer shell/read/write/edit for bounded one-shot operations. Track every terminal session id and close sessions that no longer matter. An inferred_idle or timeout result does not prove the foreground command exited.
```

#### KV Cache effect

Prefix-stable guidance plus append-only tool results after the reusable request prefix.

## Known Limitations and Deferred Work

- `register` is called from `register_base_plugins`; default YAML still omits the PTY rows.
- Named ACP `pty-tools` is not assembled.
