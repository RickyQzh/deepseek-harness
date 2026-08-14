# crates

[English](README.md) | 中文

DeepSeek Harness 宿主进程的 Rust workspace 成员。布局、保留/放弃项以及 crate 边界记录在 [Rust 重写 Agent Note](../.agents/notes/proposed/architecture/2026-08-14-rust-rewrite.md)。在后续阶段切换之前，`packages/` 下的 TypeScript 包仍是已发布的宿主。

## 成员

| Crate | 职责 |
|---|---|
| [`dsh-brand`](dsh-brand/README.md) | 品牌 id 原语（`Branded<B>`）。具体产品 id 放在所属 crate。 |
