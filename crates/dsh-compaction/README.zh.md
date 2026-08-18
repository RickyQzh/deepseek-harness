# dsh-compaction

[English](README.md) | 中文

面向 Rust 宿主的抽象压缩（compaction）类型：`CompactionEngine`、`CompactionResult`、`CompactionId`、compact 检查点 `MessageSource`，以及工具配对切割辅助函数。

本 crate 不是 YAML 插件，也不导出 `register`。具体后端是 [`dsh-compaction-basic`](../dsh-compaction-basic/README.md)；YAML 不得列出 `@deepseek-ai/dsh-compaction`。

`compact_checkpoint_source` 构造 `plugin` 为 `"compact"` 且带有事务 `compactionId` 的 `MessageSource::Plugin`。当 `kind` 为 plugin 且 `plugin` 为 `"compact"` 时，`is_compact_checkpoint_source` 为 true；它不要求存在 `compactionId`。

`tool_pairing_balanced_before` 与 `tool_pairing_balanced_after` 判断当前表层切割处是否仍有未回答的 assistant 工具调用。切割按 `session.surface_nodes()` 顺序，而不是按 seq 数值排序。每会话 cache 以 `SessionId` 字符串和 `replace_generation` 为键；generation 未变时只折叠新增尾部节点，替换发生时则重建成员关系。缺失的 seq，以及没有对应未闭合调用的 `tool/result`，会作为损坏的表层状态而 panic。

详见 [压缩能力 seam Agent Note](../../.agents/notes/implemented/feature/2026-06-18-compaction-capability-seam.md)。

## 已知限制与暂缓事项

- 本 crate 不能从 YAML 加载，也没有 `plugin::register`；封闭的 boot 注册表不得列出 `@deepseek-ai/dsh-compaction`。
- 锁、表层替换、摘要，以及 `compaction/start|summary|end` 载荷位于 `dsh-compaction-basic`。
- 配对 cache 以 `SessionId` 字符串为键，而不是会话对象身份。
