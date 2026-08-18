# Agent Note: 不持有循环互斥锁的溢出恢复 CompactionScope

Status: implemented

[English](2026-08-16-compaction-scope-overflow-retry.md) | 中文

## 问题

Rust 的 `agent/request-error` waterfall `T` 是 `RequestErrorAction`，而不是 TypeScript 的 `{ agent, failure, signal }` 对象。compaction-basic 仍须读取终态失败码、改写实时会话，并对一次摘要用的 `LlmRuntime::stream` 调用 `.await`。若在该 await 期间持有 `Mutex<LoopAgent>`，Shared 驱动上的 `followup` / `steer` 会被阻塞。改变 waterfall `T` 会重开[重试动作](../simplification/2026-07-27-request-error-retry-action.md)的返回约定。

## 决策

`LoopAgent` 在现有的 `agent/pre-step` 与 `agent/request-error` waterfall 外包一层任务局部的 `CompactionScope`（Exclusive 的 `run_until_idle` 与 Shared 的 `run_until_idle_locked`）。Exclusive 保存指向已停放 `Session` 的指针；Shared 保存指向 `Mutex<LoopAgent>` 的指针。`with_session` 运行同步回调，并在 Shared 上仅在该回调期间加锁。监听器不得在 `with_session` 内部 `.await`。

`dsh-compaction-basic` 实现 `CompactionEngine`：未匹配的 `compaction/start` 是仅日志锁；一次成功的区域依次追加 start、summary、带 `compact_checkpoint_source` 的 `user/message` 替换，然后是 `compaction/end`。溢出恢复读取 `RetryScope::failure`，快照 `replace_generation`，在不跨越摘要器持有循环互斥锁的情况下运行 `compact_if_needed(ContextOverflow)`，并且仅当 generation 前进时不调用 `next()` 而返回 `Retry`。`auto: false` 两个自动监听器都不注册。`ReplayAdapter::resolve_model` 从 YAML `providers[].models[].contextWindow` 返回 `LlmModelContext { context_window }`。`CONTEXT_WINDOW_EXCEEDED` 不在默认可重试集合中，因此重试会委派，压缩可以运行。

`CompactionEngine` 返回的 future 在同一生命周期下借用 `self`、`session`、`options` 和 `signal`（edition 2024 仅标在 `&self` 上的 `'_` 无法捕获会话）。

## 测试

`dsh-compaction-basic` 固定锁的相邻性与 generation 证明的重试：`compact_region_writes_lock_summary_replace_end`、`overflow_retries_only_when_replace_generation_advances` 以及 `overflow_without_generation_change_fails_the_turn`。`dsh-llm` 固定 `replay_resolve_model_reads_context_window`。`dsh-agent-loop` 保留 Exclusive 与 Shared 的 waterfall 测试。

## 考虑过的替代方案

- **把 waterfall `T` 加宽为携带 agent** — 不予采用，因为 Task 60 已封闭 `RequestErrorAction`，而且重试已用任务局部的 `RetryScope` 传递失败事实。
- **在摘要器 `.await` 期间持有 `Mutex<LoopAgent>`** — 不予采用，因为 Shared 的 `followup` 必须在进行中的模型工作期间拼接 inbox。
- **克隆会话、在克隆上压缩、再合并事件** — 不予采用，因为 `surfaceOp: replace` 与 `replace_generation` 必须落在重试所重建的实时日志上。

## 后果

Exclusive 与 Shared 压缩共用同一监听器主体。该 scope 是带有文档化驱动器不变量的 `unsafe` 指针别名：在 `run` 返回之前，驱动器不得使用已停放的 `LoopAgent`。`/compact`、GUI 与 spine 注册仍属后续工作；可选的 tool-result pruner 按 YAML 名称注册，但不是快照 compaction-recovery 路径的必需项。
