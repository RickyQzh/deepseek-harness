# dsh-compaction

English | [中文](README.zh.md)

Abstract compaction types for the Rust host: `CompactionEngine`, `CompactionResult`, `CompactionId`, the compact checkpoint `MessageSource`, and tool-pairing cut helpers.

This crate is not a YAML plugin and does not export `register`. The concrete backend is [`dsh-compaction-basic`](../dsh-compaction-basic/README.md); YAML must not list `@deepseek-ai/dsh-compaction`.

`compact_checkpoint_source` builds `MessageSource::Plugin` with `plugin` `"compact"` and the transaction `compactionId`. `is_compact_checkpoint_source` is true when `kind` is plugin and `plugin` is `"compact"`; it does not require `compactionId`.

`tool_pairing_balanced_before` and `tool_pairing_balanced_after` answer whether a current-surface cut has unanswered assistant tool calls. Cuts follow `session.surface_nodes()` order, not numeric seq order. The per-session cache is keyed by `SessionId` string and `replace_generation`; an unchanged generation folds only new tail nodes, and a replacement rebuilds membership. Missing seqs and a `tool/result` with no open call panic as corrupt surface state.

See the [compaction capability-seam Agent Note](../../.agents/notes/implemented/feature/2026-06-18-compaction-capability-seam.md).

## Known Limitations and Deferred Work

- This crate is not loadable from YAML and has no `plugin::register`; the closed boot registry must not list `@deepseek-ai/dsh-compaction`.
- Lock, surface replace, summarization, and `compaction/start|summary|end` payloads live in `dsh-compaction-basic`.
- The pairing cache is keyed by `SessionId` string rather than session object identity.
