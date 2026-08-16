# dsh-agent

English | [中文](README.zh.md)

Live `LoopAgent` registry for the DeepSeek Harness Rust host: `create`, `followup`, `run_until_idle`, and `status` keyed by session id.

The registry holds each agent behind `tokio::sync::Mutex<LoopAgent>` because `run_until_idle(&mut self)` cannot overlap `cancel`. `whenIdle` is `run_until_idle` then Idle. Live events use `Session::set_append_sink`.

## Known Limitations and Deferred Work

- Persist flush, CLI, JSON-RPC dispatch, and kernel plugin wrapping are later Phase 5 work. This crate does not load plugins.
