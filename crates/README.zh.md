# crates

[English](README.md) | 中文

DeepSeek Harness 宿主进程的 Rust workspace 成员。布局、保留/放弃项以及 crate 边界记录在 [Rust 重写 Agent Note](../.agents/notes/proposed/architecture/2026-08-14-rust-rewrite.md)。在后续阶段切换之前，`packages/` 下的 TypeScript 包仍是已发布的宿主。

## 成员

| Crate | 职责 |
|---|---|
| [`dsh-brand`](dsh-brand/README.md) | 品牌 id 原语（`Branded<B>`）。具体产品 id 放在所属 crate。 |
| [`dsh-kernel`](dsh-kernel/README.md) | Context、Fiber、服务、effect、isolate、事件总线。 |
| [`dsh-events`](dsh-events/README.md) | 内核事件总线的再导出（`emit` / `serial` / `parallel` / `waterfall`）。 |
| [`dsh-schema`](dsh-schema/README.md) | 插件配置 schema + Settings UI 的 JSON Schema。 |
| [`dsh-compose`](dsh-compose/README.md) | 封闭 YAML 方言：插值器、补丁、层序、disabled 谓词。 |
| [`dsh-session`](dsh-session/README.md) | 会话 id、封闭 `SessionEvent` 枚举、surface、derive、修复、chunk 行。 |
| [`dsh-session-persist`](dsh-session-persist/README.md) | JSONL + zstd 会话编解码（SQLite 稍后）。 |
