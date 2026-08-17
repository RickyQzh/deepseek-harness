# dsh-tool-terminal

English | [中文](README.zh.md)

Six model-facing `terminal_*` tools over [`dsh-terminal`](../dsh-terminal/README.md): `terminal_open`, `terminal_send`, `terminal_read`, `terminal_signal`, `terminal_close`, and `terminal_list`. YAML name `@deepseek-ai/dsh-tool-terminal`. Owner identity is `ToolExecution.session_id` (`SessionId`); a missing session fails closed with `terminal tools require an initiating session`. This crate does not depend on `dsh-agent`, `dsh-acp`, or `dsh-jobs`. It is not added to `register_spine_plugins` and is not mounted in `base.cordis.yml`.

`terminal_send` is foreground-only in this crate. `enableRunInBackground` default **true** advertises `run_in_background` and appends ` Background mode returns a job id for job_output/job_kill.` to the description; `run_in_background: true` still returns `background terminal sends require @deepseek-ai/dsh-jobs and @deepseek-ai/dsh-tool-jobs` until jobs are mounted. When `enableRunInBackground` is false, the schema omits `run_in_background` and a forced `true` fails with `background terminal sends are disabled by tool-terminal configuration`. `submit` defaults to true. Close reason is `model request`. Concurrent `kill` of an in-flight close renders `terminal session {id} was already closing`.

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

- Background `run_in_background` sends return the jobs-required sentence until `dsh-jobs` and `dsh-tool-jobs` are mounted.
- `register` is not called from `register_base_plugins`.
- Named ACP `pty-tools` is not assembled.
