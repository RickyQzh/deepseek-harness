# dsh-agent-loop

English | [中文](README.zh.md)

Scripted agent driver for the Rust host: durable inbox splices, the idle / maintenance / running phase machine, runtime-context snapshot identity, request reconstruction from `derive_messages` plus `request/header`, and a tool-call scheduler.

`followup` / `steer` / `inject` / `cancel` only mutate the inbox and abort flag. `run_until_idle` is the turn driver. A turn always appends `turn/start` before the first claim; an empty first enter still logs `turn/end` completed without a step.

A `max-tokens` step is sticky for that turn: a later completed step does not replace it, and the next turn starts with no reason. Idle `cancel` is a no-op. Abort during tool dispatch drains in-flight bodies and records `ABORTED_BEFORE_DISPATCH` for calls that never started. After abort, `turn` returns false so the current `run_until_idle` does not start another turn; leftover inbox entries wait for a later driver. Exclusive tools are a barrier of one; parallel-safe tools share `max_parallel_tool_calls`. Parallel-safe calls run `ToolRuntime::prepare` serially, overlap `dispatch` bodies, then `finalize`.

## Known Limitations and Deferred Work

- Factory `create` / `resume` and kernel plugin wrapping are Phase 5. Live registry lives in `dsh-agent`. This crate owns `LoopAgent` directly. Bash / fs tools are Phase 4; tests register mock tools.
