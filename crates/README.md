# crates

English | [中文](README.zh.md)

Rust workspace members for the DeepSeek Harness host process. Layout, keep-or-drop, and crate boundaries are recorded in the [Rust rewrite Agent Note](../.agents/notes/proposed/architecture/2026-08-14-rust-rewrite.md). TypeScript packages under `packages/` remain the shipping host until a later phase cuts over.

## Members

| Crate | Responsibility |
|---|---|
| [`dsh-brand`](dsh-brand/README.md) | Branded-id primitive (`Branded<B>`). Product ids live in owning crates. |
| [`dsh-kernel`](dsh-kernel/README.md) | Context, Fiber, services, effects, isolate, event bus. |
