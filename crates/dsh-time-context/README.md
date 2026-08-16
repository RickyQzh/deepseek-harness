# dsh-time-context

English | [中文](README.zh.md)

Optional pre-step clock injection for the Rust host. `plugin::register` reads optional YAML `timeZone` and `refreshIntervalMs`. Unknown keys and a negative or non-integer `refreshIntervalMs` fail load. YAML name `@deepseek-ai/dsh-time-context`. This crate is not added to `register_spine_plugins` and is not mounted in `base.cordis.yml`.

An `agent/pre-step` listener prepends a user-role `MessageSource::Plugin { plugin: "time-context" }` message onto a non-empty `Enter { messages }`, then calls `next(decision)`. Reject and empty Enter are unchanged (no fake step). Omit or `0` for `refreshIntervalMs` injects at every eligible step; a positive value injects only when this plugin instance has not prepended for that session yet, or wall-clock milliseconds since that prepend are at least the interval. The interval clock is a process-local map keyed by session id string, updated only on an actual prepend; log `time` is not used as wall clock. Invalid `refreshIntervalMs` fails with a message containing `refreshIntervalMs`.

Each reading is two lines: `Time sampled while preparing turn {turn}, step {step}: {YYYY-MM-DD HH:MM:SS UTC}` and `Elapsed since the preceding {baseline}: {elapsed}.` `baseline` is `model-visible message` on step 1 and `step context` later. Elapsed uses compact whole-second units (`{days}d {hours}h {minutes}m {seconds}s`, always including seconds; zero elapsed is `0s`). Elapsed is `unavailable` when there is no baseline or when the baseline event `time` is not a plausible Unix-ms timestamp (`< 1_000_000_000_000`, including seq-as-time). Wall time is formatted as UTC from `std::time::SystemTime`.

## Known Limitations and Deferred Work

- Browser-zone derivation and IANA `timeZone` display are out of this phase; timestamps are std UTC only (`YYYY-MM-DD HH:MM:SS UTC`).
- The plugin is not mounted in `base.cordis.yml` in this phase.
- LoopAgent stamps event `time` as seq, not epoch milliseconds. Positive `refreshIntervalMs` is therefore process-local and does not survive resume; elapsed text is `unavailable` on typical harness logs.
