# dsh-system-prompt

English | [中文](README.zh.md)

Assembles ordered system-prompt sections, dynamic runtime-context snapshots, tool schemas, and prompt variables for the Rust host. `{{var}}` interpolation is strict: unknown, undefined, and malformed references fail; a lone `{{` with no later `}}` is literal prose; substituted values are not scanned again.

Harness identity is the order-−100 section `harness:identity`. The deployment persona is `deployment:persona` at order 0. `TOOL_ORDER_REST` (`<unlisted-tools>`) marks where unlisted tools are inserted lexicographically. Snapshot identity (skip a new user message when the rendered text is unchanged) is applied by `dsh-agent-loop`, not this crate. `Clone` shares the live registry so later `section` calls stay visible to `assemble`.

`plugin::register` provides `systemPrompt` from YAML `persona`, `includeHarnessIdentity` (default true), and `includeRuntimeContext` (default false).

## Known Limitations and Deferred Work

- Scoped section/variable shadowing and the Cordis `system-prompt/assemble` waterfall plugin are Phase 5. This crate is a library with an in-process listener list.
