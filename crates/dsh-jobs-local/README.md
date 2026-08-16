# dsh-jobs-local

English | [中文](README.zh.md)

Process-local [`JobRegistry`](../dsh-jobs/README.md) provider. `plugin::register` provides `jobs` as `LocalJobRegistry`. `inject::<LocalJobRegistry>()` yields `Arc<LocalJobRegistry>`, so `start` / `list` / `get` / `read` / `kill` / `wait` take `&self` with an interior mutex. YAML name `@deepseek-ai/dsh-jobs-local`. This crate is not added to `register_spine_plugins` and is not mounted in `base.cordis.yml`.

`LocalJobRegistry::new(max_concurrent_per_owner)` sets the active-job cap. Config `maxConcurrentJobsPerOwner` is a positive integer and defaults to `10`. Before `run()`, `start()` counts the exact owner's `running` and `stopping` records; all unowned jobs share one bucket. Ids are `<kind>-N` with per-kind process-global counters: the first bash job is `bash-1` even if a subagent job already exists. A panic from `run` is not caught; admission has already passed and nothing is registered until `run` returns.

Settlement is first-wins. `on_job_done` fires after the record is terminal. `wait` returns the current snapshot on timeout and leaves the job alive. `Drop` and kernel dispose call `cancel` on live jobs. `cancel` is synchronous and idempotent. A panicked `done` future is recorded as `failed`.

## Config

| Key | Default | Meaning |
|---|---|---|
| `maxConcurrentJobsPerOwner` | `10` | Maximum `running` plus `stopping` jobs per owner or in the shared unowned bucket. |

Unknown keys fail load.

## Model Experience

Indirectly through producer plugins and [`dsh-tool-jobs`](../dsh-tool-jobs/README.md).

#### KV Cache effect

No direct invalidation.

## Known Limitations and Deferred Work

- One process-global registry; isolate and preset layers are not implemented.
- Jobs are process-local; records die with the harness process.
- The plugin is not mounted in `base.cordis.yml` in this phase.
