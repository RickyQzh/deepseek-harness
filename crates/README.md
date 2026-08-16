# crates

English | [中文](README.zh.md)

Rust workspace members for the DeepSeek Harness host process. Layout, keep-or-drop, and crate boundaries are recorded in the [Rust rewrite Agent Note](../.agents/notes/proposed/architecture/2026-08-14-rust-rewrite.md). TypeScript packages under `packages/` remain the shipping host until a later phase cuts over.

## Members

| Crate | Responsibility |
|---|---|
| [`dsh-brand`](dsh-brand/README.md) | Branded-id primitive (`Branded<B>`). Product ids live in owning crates. |
| [`dsh-kernel`](dsh-kernel/README.md) | Context, Fiber, services, effects, isolate, event bus. |
| [`dsh-events`](dsh-events/README.md) | Re-export of the kernel event bus (`emit` / `serial` / `parallel` / `waterfall`). |
| [`dsh-schema`](dsh-schema/README.md) | Plugin config schema + JSON Schema for Settings UI. |
| [`dsh-compose`](dsh-compose/README.md) | Closed YAML dialect: interpolators, patches, layer order, disabled predicates. |
| [`dsh-session`](dsh-session/README.md) | Session ids, closed `SessionEvent` enum, surface, derive, repair, chunk rows. |
| [`dsh-session-persist`](dsh-session-persist/README.md) | JSONL + zstd session codec (SQLite later). |
| [`dsh-tools`](dsh-tools/README.md) | Tool execution types and lossless-JSON argument freeze. |
| [`dsh-system-prompt`](dsh-system-prompt/README.md) | Ordered system-prompt sections, runtime-context snapshots, and strict `{{var}}` interpolation. |
| [`dsh-credentials`](dsh-credentials/README.md) | Per-request POSIX credential-reference resolve (env, YAML map, memory). |
| [`dsh-llm`](dsh-llm/README.md) | Provider-neutral LLM stream contract, block assembler, and mock adapter. |
| [`dsh-llm-deepseek`](dsh-llm-deepseek/README.md) | DeepSeek `POST /chat/completions` SSE adapter: serialize, translate, per-request key, idle timeout. |
| [`dsh-agent-loop`](dsh-agent-loop/README.md) | Scripted loop: durable inbox, idle / maintenance / running, runtime-context snapshots, request reconstruction, tool-call scheduler. |
| [`dsh-subprocess`](dsh-subprocess/README.md) | Fully specified argv spawn and credential-scrubbed child env. |
| [`dsh-sandbox`](dsh-sandbox/README.md) | Fail-closed sandbox modes, writable roots, and escalation. |
