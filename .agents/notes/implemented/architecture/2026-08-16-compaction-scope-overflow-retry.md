# Agent Note: CompactionScope for overflow recovery without holding the loop mutex

Status: implemented

English | [中文](2026-08-16-compaction-scope-overflow-retry.zh.md)

## Problem

Rust `agent/request-error` waterfall `T` is `RequestErrorAction`, not the TypeScript `{ agent, failure, signal }` object. Compaction-basic still must read the terminal failure code, mutate the live session, and `.await` one summarizer `LlmRuntime::stream` call. Holding `Mutex<LoopAgent>` across that await would block `followup` / `steer` on the Shared driver. Changing waterfall `T` would reopen the [retry-action](../simplification/2026-07-27-request-error-retry-action.md) return contract.

## Decision

`LoopAgent` enters a task-local `CompactionScope` around the existing `agent/pre-step` and `agent/request-error` waterfalls (Exclusive `run_until_idle` and Shared `run_until_idle_locked`). Exclusive stores a pointer to the parked `Session`; Shared stores a pointer to `Mutex<LoopAgent>`. `with_session` runs a synchronous callback and, on Shared, locks only for that callback. Listeners must not `.await` inside `with_session`.

`dsh-compaction-basic` implements `CompactionEngine`: unmatched `compaction/start` is the log-only lock; a successful region appends start, summary, a `user/message` replace with `compact_checkpoint_source`, then `compaction/end`. Overflow recovery reads `RetryScope::failure`, snapshots `replace_generation`, runs `compact_if_needed(ContextOverflow)` without holding the loop mutex across the summarizer, and returns `Retry` without `next()` only when generation advanced. `auto: false` registers neither automatic listener. `ReplayAdapter::resolve_model` returns `LlmModelContext { context_window }` from YAML `providers[].models[].contextWindow`. `CONTEXT_WINDOW_EXCEEDED` is not in the default retryable set, so retry delegates and compaction can run.

The `CompactionEngine` returned future borrows `self`, `session`, `options`, and `signal` under one lifetime (edition 2024 `'_` on `&self` alone cannot capture the session).

## Testing

`dsh-compaction-basic` pins lock adjacency and generation-proof retry: `compact_region_writes_lock_summary_replace_end`, `overflow_retries_only_when_replace_generation_advances`, and `overflow_without_generation_change_fails_the_turn`. `dsh-llm` pins `replay_resolve_model_reads_context_window`. `dsh-agent-loop` keeps Exclusive and Shared waterfall tests.

## Alternatives considered

- **Widen waterfall `T` to carry the agent** — rejected because Task 60 closed `RequestErrorAction` and retry already uses task-local `RetryScope` for failure facts.
- **Hold `Mutex<LoopAgent>` across the summarizer `.await`** — rejected because Shared `followup` must splice the inbox during in-flight model work.
- **Clone the session, compact the clone, merge events** — rejected because `surfaceOp: replace` and `replace_generation` must land on the live log the retry reconstructs.

## Consequences

Exclusive and Shared compaction share one listener body. The scope is `unsafe` pointer aliasing with a documented driver invariant: the driver does not use the parked `LoopAgent` until `run` returns. `/compact`, GUI, and spine registration remain later work; the optional tool-result pruner is registered by YAML name but is not required on the snapshot compaction-recovery path.
