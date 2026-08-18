# dsh-compaction-basic

English | [中文](README.zh.md)

Log-only compaction backend for the Rust host: `plugin::register` injects `llm` and `tokenMeter`, provides `compaction` as a single `BasicCompactionEngine` (not `Arc`), and with `auto: true` (default) registers `agent/pre-step` pressure and `agent/request-error` overflow listeners. YAML name `@deepseek-ai/dsh-compaction-basic`. Unknown keys fail load. This crate is not added to `register_spine_plugins`.

An unmatched `compaction/start` is the lock. A successful region writes `compaction/start`, one `LlmPurpose::Compaction` `LlmRuntime::stream` call, `compaction/summary`, a `user/message` with `compact_checkpoint_source` and `surfaceOp: replace`, then `compaction/end`. Time equals seq as i64. Replacement content is three text blocks: preamble plus `<compacted-summary>`, the summary text, and `</compacted-summary>`. Overflow recovery snapshots `replace_generation` before `compact_if_needed(ContextOverflow)` and returns `Retry` without `next()` only when generation advanced; otherwise it must `next()`. `auto: false` registers neither listener. Pressure no-ops without a durable `request/header`. Optional YAML `@deepseek-ai/dsh-compaction-tool-result-pruner` provides `toolResultPruner`; pressure and overflow call it when present.

Defaults: `thresholdRatio` 0.8, `retainRatio` 0.16, `maxTokens` 8192, `compactionRetries` 1, `maxOverflowRetries` 1, `auto` true, empty summarization provider/model. Retention keeps the smallest tail of whole surface units whose estimated size meets the retain budget and expands the cut until both edges pass `tool_pairing_balanced_before` / `tool_pairing_balanced_after`. Listeners use task-local `CompactionScope` and must not hold `Mutex<LoopAgent>` across the summarizer `.await`.

See the [compaction capability-seam Agent Note](../../.agents/notes/implemented/feature/2026-06-18-compaction-capability-seam.md) and [after-call recovery](../../.agents/notes/implemented/architecture/2026-07-10-after-call-compaction-pressure-and-overflow-recovery.md).

## Known Limitations and Deferred Work

- `/compact`, GUI, and spine registration are later work; snapshot compaction-recovery does not require the pruner.
- Automatic pressure warns by delegating when the routed adapter has no `contextWindow`; overflow recovery does not need capacity metadata.
- Summarization is one auxiliary `LlmRuntime::stream` call, not a subclass hook.
