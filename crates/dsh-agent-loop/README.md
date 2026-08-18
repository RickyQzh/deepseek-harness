# dsh-agent-loop

English | [中文](README.zh.md)

Scripted agent driver for the Rust host: durable inbox splices, the idle / maintenance / running phase machine, runtime-context snapshot identity, request reconstruction from `derive_messages` plus `request/header`, and a tool-call scheduler.

`followup` / `steer` / `inject` / `cancel` only mutate the inbox and abort flag. `run_until_idle` is the turn driver. A turn always appends `turn/start` before the first claim; an empty first enter still logs `turn/end` completed without a step. `LoopAgent::new` takes a kernel `Context` and shared `Arc<Mutex<ToolRuntime>>` / `Arc<Mutex<LlmRuntime>>`. Pre-step admission is the `agent/pre-step` waterfall (`EVENT_AGENT_PRE_STEP`) over `PreStepDecision`; a listener that returns without `next()` short-circuits. Model-request recovery is the `agent/request-error` waterfall (`EVENT_AGENT_REQUEST_ERROR`) over `RequestErrorAction`; the default is `Fail`, and `Retry` repeats the request in the same step. `LoopAgent` enters task-local `CompactionScope` (Exclusive and Shared) plus `dsh_llm::retry::RetryScope` around those waterfalls so compaction can mutate the live session without holding `Mutex<LoopAgent>` across a summarizer `.await`, then appends drained `llm/retry` / `llm/retry-started` records afterward (`seq`/`time` = current log length). `run_until_idle_locked` drives the same loop while locking a `Mutex<LoopAgent>` only around synchronous session mutations.

A `max-tokens` step is sticky for that turn: a later completed step does not replace it, and the next turn starts with no reason. Idle `cancel` is a no-op. Abort during tool dispatch drains in-flight bodies and records `ABORTED_BEFORE_DISPATCH` for calls that never started. After abort, `turn` returns false so the current `run_until_idle` does not start another turn; leftover inbox entries wait for a later driver. Exclusive tools are a barrier of one; parallel-safe tools share `max_parallel_tool_calls`. Parallel-safe calls run `ToolRuntime::prepare` serially, overlap `dispatch` bodies, then `finalize`. `prepare` and `finalize` hold the tools `Mutex` on Tokio's blocking pool so the runtime worker can poll in-flight dispatch futures and `cancel`.

## Known Limitations and Deferred Work

- Factory `create` / `resume` and kernel plugin wrapping are Phase 5. Live registry lives in `dsh-agent`. This crate owns `LoopAgent` directly. Bash / fs tools are Phase 4; tests register mock tools.
