# dsh-jobs

English | [中文](README.zh.md)

Background-job Service Definition for the DeepSeek Harness Rust host: branded [`JobId`](src/brand.rs), snapshots, and the [`JobRegistry`](src/types.rs) trait. This crate is not a YAML plugin name; loading `@deepseek-ai/dsh-jobs` fails as unknown. The process-local provider is [`dsh-jobs-local`](../dsh-jobs-local/README.md). This crate is not added to `register_spine_plugins` and is not mounted in `base.cordis.yml`.

`JobId` is a local newtype around `dsh_brand::Branded<JobIdTag>` (`new` / `as_str`). `JobKind` is `Bash`, `Subagent`, or `PtySend`; id prefixes are `bash`, `subagent`, and `pty-send`. `JobStart.owner_session` is an optional `SessionId`, not a live agent. `JobStart.run` is synchronous and must not re-enter the registry that is starting the job. `dsh-jobs` does not depend on `dsh-agent`.

Owned access compares session ids. Ids such as `bash-1` are predictable, so this fence is the boundary. `list(Some(other))` is empty, not an error. `get` / `read` / `kill` of a foreign owner fail. `caller: None` sees unowned jobs. Unowned jobs (`owner_session: None`) are open to any caller.

Settlement is first-wins: one terminal status, waiters released once, `on_job_done` once. `wait` is not on the trait; [`LocalJobRegistry::wait`](../dsh-jobs-local/README.md) owns bounded waits.

## Model Experience

Indirectly through producer plugins and [`dsh-tool-jobs`](../dsh-tool-jobs/README.md).

#### KV Cache effect

No direct invalidation.

## Known Limitations and Deferred Work

- Isolate and preset layers are not implemented; one process-global registry is provided by `dsh-jobs-local`.
- `attachController` / `servesOwner` are omitted in this phase.
- The crate is not mounted in `base.cordis.yml` in this phase.
