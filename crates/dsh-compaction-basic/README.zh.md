# dsh-compaction-basic

[English](README.md) | 中文

面向 Rust 宿主的仅日志压缩（compaction）后端：`plugin::register` 注入 `llm` 与 `tokenMeter`，以单个 `BasicCompactionEngine` 提供 `compaction`（不是 `Arc`），并在 `auto: true`（默认）时注册 `agent/pre-step` 压力监听器与 `agent/request-error` 溢出监听器。YAML 名称为 `@deepseek-ai/dsh-compaction-basic`。未知键会在加载时失败。本 crate 不加入 `register_spine_plugins`。

未匹配的 `compaction/start` 就是锁。一次成功的区域写入顺序为 `compaction/start`、一次 `LlmPurpose::Compaction` 的 `LlmRuntime::stream` 调用、`compaction/summary`、带 `compact_checkpoint_source` 与 `surfaceOp: replace` 的 `user/message`，然后是 `compaction/end`。time 等于 seq（i64）。替换内容是三个文本块：前导语加 `<compacted-summary>`、摘要正文、以及 `</compacted-summary>`。溢出恢复在 `compact_if_needed(ContextOverflow)` 之前快照 `replace_generation`，仅当 generation 前进时不调用 `next()` 并返回 `Retry`；否则必须 `next()`。`auto: false` 两个监听器都不注册。没有持久化 `request/header` 时压力路径为空操作。可选 YAML `@deepseek-ai/dsh-compaction-tool-result-pruner` 提供 `toolResultPruner`；压力与溢出在其存在时调用它。

默认值：`thresholdRatio` 0.8、`retainRatio` 0.16、`maxTokens` 8192、`compactionRetries` 1、`maxOverflowRetries` 1、`auto` true、摘要提供方/模型为空。保留策略取估计大小达到保留预算的最小整段 surface 尾部，并扩展切割点直到两端都通过 `tool_pairing_balanced_before` / `tool_pairing_balanced_after`。监听器使用任务局部的 `CompactionScope`，不得在摘要器 `.await` 期间持有 `Mutex<LoopAgent>`。

参见[压缩能力 seam Agent Note](../../.agents/notes/implemented/feature/2026-06-18-compaction-capability-seam.md)与[调用后恢复](../../.agents/notes/implemented/architecture/2026-07-10-after-call-compaction-pressure-and-overflow-recovery.md)。

## 已知限制与暂缓事项

- `/compact`、GUI 与 spine 注册属于后续工作；快照 compaction-recovery 不要求 pruner。
- 当已路由适配器没有 `contextWindow` 时，自动压力通过委派发出警告；溢出恢复不需要容量元数据。
- 摘要是一次辅助 `LlmRuntime::stream` 调用，不是子类钩子。
