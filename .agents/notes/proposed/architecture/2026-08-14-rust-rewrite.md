# Agent Note: Rewrite core and backend in Rust; keep the TypeScript web UI

Status: proposed

English | [中文](2026-08-14-rust-rewrite.zh.md)

## Problem

DeepSeek Harness is a plugin-composed coding-agent runtime: a running instance is an ordered tree of plugins that contribute services, typed events, and reversible effects into a shared Cordis `Context`. There is no privileged core to patch. The product today is **219** `@deepseek-ai/dsh-*` packages (~1,316 `src` files, ~227k LOC) plus vendored Cordis, a C Landlock launcher, a Python JSON-RPC SDK, a React web UI, and a VitePress docs site. Every harness package is a Cordis plugin. A language change is therefore a **plugin-runtime plus product** port, not a library rewrite.

The host is Node. Source launch is `node --import tsx/esm`. Workflow and Code Mode engines are `worker_threads` plus `node:vm`. Dynamic self-modification evaluates JavaScript as a live Cordis plugin. Config YAML interpolates `!!js` by `eval` against the plugin context. Those mechanisms have no Rust equivalent and must be replaced, not transcribed.

There is **no** existing Agent Note proposing a host-language migration. Native Landlock is C11 (~300 lines, static musl) because a Rust/landstrip audit surface was [rejected](../../rejected/feature/2026-07-26-evaluate-landstrip-for-windows-sandbox-rung.md) after an earlier Rust launcher was replaced. A rewrite that silently re-Rusts that binary reopens a recorded security invariant.

The repo is pre-release: backends reject old on-disk formats; `SESSION_FORMAT_VERSION` stays `0` with no compatibility promise. External clients still exist: PyPI `deepseek-harness-sdk`, ACP stdio, Claude Code/Codex `hooks.json`, and the shipped TypeScript browser. Those wires are the real compatibility pressure, not the package-group “stable API” label.

This note is the single architecture proposal for rewriting **core and backend in Rust** while allowing the web frontend to stay TypeScript. It is not an implementation PR. Per-phase task plans are written when that phase starts; this document locks keep/drop, crate boundaries, wire freeze, and cutover order so later workstreams do not invent a second product.

## Proposal

Ship **one Rust host process** that owns the agent loop, session log, tools, LLM HTTP, filesystem/subprocess/sandbox, persistence, CLI, ACP, and SDK JSON-RPC. Keep `packages/client/*`, `apps/web`, and `website/` in TypeScript. The browser continues to speak the existing four-quadrant HTTP + WebSocket protocol. Python continues to `Popen` a runtime binary over NDJSON JSON-RPC; only the spawned exe changes.

Keep Cordis **semantics** (named services, inject-wait, reverse-order dispose, waterfall `next()`, isolate realms, id-targeted whole-config patches). Drop the Cordis **implementation**: Proxies, TypeScript declaration merging, ESM ModuleLoader, and `!!js` `eval`. v1 plugins are trusted in-process Rust crates listed in a profile manifest, not hot-loaded JavaScript.

Keep the product invariants that make this harness itself: append-only session log, model-visible ⟺ logged, capability seams as Definition / Provider / Consumer, sandbox as same-world argv wrapping (not a VM), and keyless snapshots through real assembled examples. Implement today’s session format (`SESSION_FORMAT_VERSION = 0`, JSONL zstd default, SQLite `SCHEMA_VERSION = 15`) so existing fixtures replay. Do not invent a parallel log in the rewrite.

First shipping Rust product is **headless + SDK JSON-RPC** (Python talks to the Rust bin). Second is a Rust GUI host that serves the existing TypeScript SPA. Optional capabilities (ACP, MCP, terminal, LSP, workflow, Windows ACL) follow. E2B, pi-ai, Cordis HMR-as-plugin, Typert-as-TypeScript-analyzer, and `node:vm` self-modification are out of v1.

## Scope and non-goals

**In scope for the rewrite program**

- Kernel that replaces vendored Cordis for the host process (`dsh-kernel`, compose, events, schema).
- Product spine: scope, session, system-prompt, tools, agent, agent-loop, LLM DeepSeek adapter, JSONL persistence, credentials, settings, identity.
- Local execution world: `ctx.fs` + `ctx.subprocess` as one world; bash (POSIX) / pwsh (win32); sandbox wrapping host `bwrap` / C `landlock-run` / `sandbox-exec` / Rust Windows ACL; fs-sandbox fence.
- Interaction: approval, permission presets, commands, ask-user.
- Compaction, token meter, agent-instructions, skills filesystem, DeepSeek web search, jobs, in-process subagent spawn/fork.
- `dsh` CLI (clap), headless runner, SDK JSON-RPC server, later axum GUI host.
- Snapshot harness port that consumes existing `session.jsonl` fixtures.
- Python runtime-bin pointing at the Rust exe.

**Stay TypeScript**

- All `packages/client/*` and `apps/web` (React, slots, conversation nodes).
- `website/` VitePress projector.
- `dsh-typert-generator` until an IDL replaces it; the generator is a TypeScript `ts.Program` walker.
- Browser halves of dual-face packages (`./client` exports). The Rust host must still emit `__DSH_BOOT__` and serve `/plugins/<id>/client.js`.

**Stay C / host CLI**

- `native/landlock-run` C11 musl helper. The Rust host **spawns** it. Do not Landlock the agent process. Do not rewrite `main.c` in Rust without a new Agent Note that reopens the landstrip rejection.
- Host `bwrap` and `sandbox-exec`.

**Stay Python**

- `python/sdk` (`deepseek_harness`). It never imports the host language.

**Out of v1 (product packages may remain in the TypeScript tree until deleted)**

- E2B POC (`packages/e2b/`).
- `dsh-llm-pi-ai` design twin.
- `dsh-tool-cordis` / `dsh-cordis-host-runner` (`node:vm` live plugin mount).
- Cordis module HMR (`cordis-plugin-hmr`); user-patch file watch can return later as a simple watcher.
- Typert type-graph extractor. Replace with a small IDL for GUI + SDK methods.
- `!!js` arbitrary JavaScript in YAML.
- Windows PTY (the product has none today).
- Electron / Tauri / VS Code extension (reserved, not shipped).

## Recommended architecture

```
┌─ Rust host (one process) ─────────────────────────────────────┐
│  dsh-kernel: Context, Fiber, services, effects, isolate        │
│  agent loop, sessions, tools, LLM, fs/shell/sandbox, persist   │
│  GUI RPC: HTTP+WS (axum) implementing today’s four quadrants   │
│  serve SPA dist + /plugins + __DSH_BOOT__                      │
│  SDK JSON-RPC stdio  (Python / TS SDK / subagent-dsh-sdk)      │
│  ACP stdio           (editors / subagent-acp)                  │
│  headless one-shot   (dsh --profile headless)                  │
└────────────┬──────────────────────┬────────────────────────────┘
             │ GUI four-quadrant    │ NDJSON JSON-RPC / ACP
             ▼                      ▼
┌─ TS UI (browser / future webview) ┐  ┌─ Python / TS SDK / ACP ┐
│  packages/client + apps/web       │  │  unchanged IPC, new bin │
│  Cordis client tree, slots, React │  └─────────────────────────┘
└───────────────────────────────────┘
```

Three wires stay distinct. Do not point Python at the GUI HTTP API. Do not make the browser speak SDK JSON-RPC.

The browser Loader is independent of the host plugin runtime. Rust must scan manifests (or a generated roster), hash bundles, serve them, and inject the boot graph. It must not run Cordis in the host.

## Keep-or-drop inventory

Each row is a recorded decision. “Keep” means preserve the **observable contract**. “Replace” means keep the contract and change the implementation. “Drop” means v1 will not ship it.

### Product identity

| Invariant | Stance | Authority |
|---|---|---|
| Everything is a plugin; no privileged core | **Replace**: in-process Rust crates + profile manifest | [architecture.md](../../../../docs/architecture.md); [microkernel event taxonomy](../../implemented/architecture/2026-06-11-microkernel-event-taxonomy.md) |
| Registrations are effects; `register()` returns a disposer | **Keep** | root `AGENTS.md` |
| Profiles + bundles; empty root; `dsh-base` first | **Keep** the layer order; patch files become a closed YAML dialect | [profile plugin bundles](../../implemented/architecture/2026-08-05-profile-plugin-bundles.md) |
| New behavior on extension points, not loop edits | **Keep** | root `AGENTS.md` |
| Waterfall listeners must call `next()` | **Keep** | [cordis-primer.md](../../../../docs/cordis-primer.md) |
| Merge-extensible `SessionEventMap` | **Replace** with a closed first-party serde enum + `ignorable` leftovers | [typed event schemas](2026-06-16-typed-event-schemas.md); [session log version](../../implemented/architecture/2026-08-10-session-log-version-mechanism.md) |
| Client plugins = Cordis DI in the browser | **Keep** (TS) | [client plugin loading](../../implemented/architecture/2026-07-23-client-plugin-loading-model.md) |

### Session / durability

| Invariant | Stance | Authority |
|---|---|---|
| Session is an append-only typed event log; `deriveMessages()` is history | **Keep** | [event-sourced sessions](../../implemented/architecture/2026-06-11-event-sourced-sessions.md) |
| Model-visible ⟺ logged | **Keep** | [reconstructable requests](../../implemented/architecture/2026-07-05-reconstructable-requests.md) |
| Canonical log is lossless and contiguous, including `assistant/chunk` | **Keep** | [session persistence](../../implemented/architecture/2026-06-14-session-persistence.md) |
| Flushed events never rewritten; crashed turns closed, never truncated | **Keep** | same |
| Header is out-of-log (`SessionHeader`) | **Keep** | same |
| Surface (`append` / `replace`) is the only history-manipulation mechanism | **Keep** | [session surface](../../implemented/architecture/2026-06-18-session-surface.md) |
| `SESSION_FORMAT_VERSION = 0`; unknown required events refuse unless `ignorable: true` | **Keep** (implement today’s reader) | [session log version](../../implemented/architecture/2026-08-10-session-log-version-mechanism.md) |
| SQLite `SCHEMA_VERSION` monotonic; non-current `user_version` rejected | **Keep** at **15** / `application_id = 0x44534850` | persistence notes |

Do not start a v1 log in the rewrite. Do not add a migration system for v0. Port the two same-version import exceptions (pre-identity messages; pre-react-loop) or fail loud on those shapes — do not invent a third compatibility layer.

### Capability seams and execution

| Invariant | Stance | Authority |
|---|---|---|
| A seam is Definition + Provider + Consumer | **Keep** as crate roles | [capability seams](../../implemented/architecture/2026-06-13-capability-seams.md) |
| `ctx.fs` + `ctx.subprocess` are one execution world | **Keep** | [portable execution world](../../implemented/architecture/2026-07-28-portable-execution-world-consumers.md) |
| `ctx.sandbox.confine` returns wrapped argv; containers/remote replace the world pair | **Keep** | [sandbox](../../implemented/feature/2026-07-06-sandbox.md) |
| Landlock is a separate restrict-then-exec helper | **Keep C binary** | [landstrip rejection](../../rejected/feature/2026-07-26-evaluate-landstrip-for-windows-sandbox-rung.md); [native/README.md](../../../../native/README.md) |
| Explicit `resolve(request): Spec` | **Keep** | root `AGENTS.md` (`dsh-shell` template) |
| Opaque branded ids | **Keep** as Rust newtypes | root `AGENTS.md` |
| No hardcoded tunables in plugins | **Keep** | root `AGENTS.md` |
| Trust typed same-process values; validate at parser/config/queued/model JSON/durable/file/worker/process/wire | **Keep** (recast: trust Rust types in-process) | root `AGENTS.md` |
| Source launch = `node --import tsx/esm` | **Drop** (host is not Node) | [source-launch tsx ESM](../../implemented/architecture/2026-07-29-dsh-source-launch-tsx-esm.md) |
| Vendored Cordis as the framework | **Replace** the implementation; keep the listed semantics | [vendor/README.md](../../../../vendor/README.md) |
| Keyless snapshot through a real runnable example | **Keep** | [testing.md](../../../../docs/testing.md) |

### Security invariants that must not regress

1. Never silently unconfine. No usable runner → `SANDBOX_UNAVAILABLE`; the command does not run.
2. Landlock/bwrap/Seatbelt: if the kernel cannot enforce, do not exec. Landlock exit 125 + `landlock-run: ` fatal line. No env-var override of which binary confines.
3. `danger-full-access` is the only unconfined path and is explicit.
4. Escalation: paired `sandbox_permissions` + non-empty `justification`; strictly wider; `ctx.approval` before execute; grant is that call only.
5. Modes claim file effects only. Do not advertise network or PID isolation.
6. One `writableRoots()` for Seatbelt and `fs-sandbox`.
7. Windows ACL reports `partial`.
8. Subprocess `argv` is never a shell string.
9. Credential scrub before child env.
10. `rg` spawn uses `--no-config` first.
11. FS fence is containment, not a kernel boundary. `FS_SANDBOX_DENIED` ≠ `FS_PERMISSION_DENIED`.
12. Observation policy: edit without prior read → `FS_NOT_OBSERVED`.
13. LSP: no `workspace/applyEdit`; no protocol escape; query source inside workspace; `processId: null`.
14. PTY: exact-Agent auth; refuse SIGKILL of the shell; cancel is real SIGINT; PID+starttime fence.
15. Tree terminate waits for tree quiescence. Host-exit sync kill must not claim quiescence.
16. Permission presets pin per session at creation.

## Crate topology

v1 is a Cargo workspace beside `packages/`, not a crate-per-TS-package dump of all 219 names. Files that change together live together. Definition / Provider / Consumer remain separate crates when those roles already evolve independently.

### Kernel (replaces Cordis on the host)

| Crate | Responsibility |
|---|---|
| `dsh-brand` | Newtypes: `SessionId`, `MessageId`, `CallId`, … |
| `dsh-kernel` | `Context`, `Fiber` state machine, service registry, effect/dispose, isolate, intercept, inject-wait |
| `dsh-events` | `emit` / `waterfall` / `parallel` / `serial` + scope filter |
| `dsh-compose` | Entry list, id-targeted patches, insert, layer order, dump-config |
| `dsh-schema` | Plugin config validation + JSON-serializable schema for Settings UI |

Drop as crates: cosmokit, Cordis HMR plugin, logger-console, Node ModuleLoader. Logging is `tracing`.

### Spine (port together; this is the minimum agent)

`dsh-scope`, `dsh-session`, `dsh-system-prompt`, `dsh-tools`, `dsh-agent`, `dsh-agent-loop`, `dsh-llm`, `dsh-llm-deepseek`, `dsh-llm-retry`, `dsh-session-persist` (seam + JSONL; SQLite later), `dsh-credentials`, `dsh-settings`, `dsh-identity`.

### Execution world (swap as a unit)

`dsh-subprocess` + OS provider, `dsh-fs` + local + sandbox fence, `dsh-sandbox` + local wrapper of existing helpers, `dsh-shell` + bash/pwsh, `dsh-tool-fs`, `dsh-tool-bash` / `dsh-tool-pwsh`. Terminal and LSP are later optional crates on the same world.

### First product surfaces

`dsh-boot`, `dsh-cli`, `dsh-headless`, `dsh-sdk-protocol`, `dsh-sdk-jsonrpc-server`. Later: `dsh-host` (axum), `dsh-acp`.

### Trait sketches (names only)

```
trait Session {
  fn id(&self) -> SessionId;
  fn append(&mut self, event: SessionEvent, surface: Option<SurfaceIntent>) -> &SessionEvent;
  fn events(&self) -> &[SessionEvent];
  fn derive_messages(&self) -> Vec<Message>;
}

trait ToolRuntime {
  fn register(&self, def: ToolDefinition) -> Dispose;
  async fn execute(&self, input: ToolExecutionInput) -> ToolExecutionResult;
}

trait LlmAdapter {
  async fn stream(&self, opts: GenerateOptions) -> impl Stream<Item = StreamChunk>;
}

trait AgentFactory {
  async fn create_agent(&self, owner: &Context, opts: CreateAgentOptions) -> AgentHandle;
  async fn resume(&self, owner: &Context, opts: ResumeAgentOptions) -> AgentHandle;
}

trait SandboxProvider {
  fn confine(&self, argv: Vec<OsString>, policy: SandboxPolicy) -> Result<ConfinedArgv, SandboxError>;
}
```

Waterfalls are listener chains that **must** call `next()`. Returning without `next()` short-circuits, matching [cordis-primer.md](../../../../docs/cordis-primer.md).

## Plugin and composition model

### v1 plugin ABI

Trusted same-process crates. A profile lists crate-backed row ids. There is no stable C ABI and no cdylib hot-load in v1. Third-party plugins, if they exist later, are workspace crates or a later wasm component story — not `node:vm`.

`dsh-tool-cordis` (inspect/define/run live JavaScript plugins) is **out of v1**. Reintroducing dynamic plugins requires its own Agent Note and a real isolation story (wasmtime or a landlock’d process). Today’s `node:vm` is not a security boundary.

### YAML dialect (breaks `!!js`)

Do not port `eval` / `with(ctx)`. Product `!!js` uses collapse into three behaviors:

1. **Closed interpolators:** `${env:VAR:-default}`, `${cwd}`, `${dshHome:sessions}`, `${platform}`.
2. **`disabled` predicates or platform overlay files** for bash vs pwsh rows.
3. **Plugins read injected services in code.** `dsh-host-webserver` reads `ctx.webStartup`; headless reads `ctx.headlessStartup.task`. YAML stays static.

Keep id-targeted **whole-config replace** and `insert` (no deep merge), the same layer order as today, and fail-loud settlement. `dsh --dump-config` prints the composed tree with interpolators unevaluated.

HMR of user `cordis.patch.yml` may return as a file watcher that recomposes layers and keeps the last-good tree on parse failure. Module HMR is not v1.

### Presets

Keep standing mounts, `agent → preset → global` shadowing, isolate for preset-owned services (`planMode`, `workflowEngine`, `compaction`), and the rule that a preset row that provides into the root realm is a load failure. Implement isolate without Proxies: explicit realm keys on the service store.

## Durable and wire formats

### Session log

Implement today’s v0 JSONL (default `$DSH_HOME/sessions/--<project>--/<id>/session.jsonl.zstd`, packed chunk rows, header line `type: "session"`) and optional SQLite (`SCHEMA_VERSION = 15`). Closed `#[serde(tag = "type")]` enum of the generated first-party types in `packages/core/session/src/known-event-types.ts` (44 types at survey time) plus an unknown leftover that is accepted only when `ignorable: true`.

Generate the Rust enum from the same catalog generator that emits `KNOWN_SESSION_EVENT_TYPES` until the TypeScript packages are deleted, then move the catalog owner to Rust. Do not make the known set composition-dependent ([session log version](../../implemented/architecture/2026-08-10-session-log-version-mechanism.md) rejected a runtime registry for first-party readers).

Use `rusqlite` for the SQLite backend (sync, pragma-heavy, no migrator). Product default remains JSONL.

Settings YAML, credentials YAML, and `.anonymous-user-id` have no version field. Implement them as-is. Users already have those files.

### Three wires

| Protocol | Consumer | v1 freeze |
|---|---|---|
| SDK JSON-RPC (`initialize`, `session/prompt`, `shutdown`; notifications `session.event`, `session.status`, `subagent.started`, `subagent.finished`) | Python, TS SDK, `subagent-dsh-sdk` | **Byte-compatible** method names, `serverInfo.name = deepseek-harness-sdk-runtime`, full `SessionEvent` envelopes |
| ACP subset (`initialize`, `authenticate`, `session/new`, `session/prompt`, `session/cancel`; `session/update` agent_message_chunk; `session/request_permission`) | ACP clients, `subagent-acp` | **Byte-compatible** advertised caps and permission option ids |
| GUI four-quadrant HTTP + WS (`RpcMethodMap` + Typert slash remotes + mux/host frames) | TypeScript browser | **Freeze if the TS UI stays**; no `protocolVersion` today because client and host ship together |

Hook stdin/stdout/exit-code dialects for the mapped Claude Code and Codex events stay compatible if those bridges ship.

MCP tool names stay `mcp__<serverName>__<rawName>` if MCP ships.

Typert-as-analyzer is not a durable format. A Rust GUI host either consumes a frozen descriptor dump or a small IDL that generates Rust types + TS client stubs. Same-process Rust plugins do not need Typert.

## Testing and dual-run

Keep the product testing policy in [testing.md](../../../../docs/testing.md):

- Crate unit tests for races, error paths, dispose, contract regressions.
- Per-file (per-module) 100% on owned Rust `src` with the same exemption discipline.
- **Keyless snapshots through real assembled examples** are the product test. Reuse committed fixtures. Port `dsh-llm-replay` + normalizers. CI: replay-only, built binary, no `.env`.
- With-key e2e self-skips without a key; CI preflights the secret.
- Built-artifact smokes of the published bin, not source-only `cargo test`.

While a TypeScript package still ships, its Vitest + coverage stay required. Snapshots and e2e run against **one** assembled product — the bin users run. When the shipping bin becomes Rust, move the snapshot **driver** and keep the same fixture directories. Do not merge two snapshot suites against two compositions.

Add a required Rust CI lane (fmt, clippy, test, llvm-cov) as soon as any crate is in the merge path. Keep existing Node 24 lanes until the last TypeScript package in that lane is gone.

## Implementation plan

> **For agentic workers:** implement each phase with a fresh subagent per task after that phase’s detailed task plan exists. This section is the program order, not a 2–5 minute task list. A phase that covers multiple independent subsystems still gets its own Agent Note plus a bite-sized plan before code.

**Global constraints (every phase inherits these)**

- Host language for new core/backend code is Rust (edition 2024 or the then-current edition the CI pin names). Web UI stays TypeScript.
- Do not rewrite `native/landlock-run` in Rust.
- Do not port `!!js` eval.
- Session reader implements format `0` and SQLite schema `15`.
- SDK JSON-RPC and ACP method names do not change in a phase that claims “compatible”.
- Snapshots stay keyless replay of existing JSONL fixtures.
- Copyleft licenses must not enter the shipped closure (existing third-party-notices gate).
- Leader agents do not explore or rewrite locally; they dispatch subagents.

### Phase 0 — Workspace and CI

**Goal:** a Cargo workspace that CI runs, with no product behavior yet.

- [x] Add `crates/` with a workspace `Cargo.toml`, `rust-toolchain.toml`, `cargo fmt`/`clippy` config.
- [x] Add a GitHub Actions Rust lane (fmt, clippy, test) that is required when any `crates/**` path changes.
- [x] Document the source-vs-artifact split for rustc (replace the tsx/lib rule for host code).
- [ ] Exit: empty `dsh-brand` crate tests green on CI; Node lanes unchanged.

### Phase 1 — Kernel

**Goal:** Cordis semantics without Cordis.

- [ ] `dsh-kernel`: Fiber states PENDING → LOADING → ACTIVE | FAILED | UNLOADING → DISPOSED; reverse-order async dispose; no new effects while UNLOADING; failed setup rolls back collected cleanup.
- [ ] `dsh-events`: emit (contained), serial, parallel, waterfall with `next()`.
- [ ] `dsh-compose`: id-targeted whole-config replace, insert, layer order; closed interpolators; `disabled` predicates; **no** `!!js`.
- [ ] `dsh-schema`: serde + JSON-serializable schema for Settings UI.
- [ ] Isolate realms sufficient for later presets.
- [ ] Exit: kernel unit tests for inject-wait, dispose order, waterfall short-circuit, patch insert-then-patch in the same list, fail-loud missing referent.

### Phase 2 — Session + spine types

**Goal:** append-only log, surface, derive, repair, inbox types.

- [ ] Closed `SessionEvent` enum generated from the persistence catalog.
- [ ] JSONL codec including zstd frames and packed chunk rows.
- [ ] `deriveMessages`, surface replace, header fold, interrupted-turn repair.
- [ ] Format-refusal tests ported from `session-format-guard.snapshot.ts` / coordinator-contract.
- [ ] Exit: round-trip `examples/headless-agent/tests/snapshots/headless-profile/session.expected.jsonl` (normalized) through the Rust codec.

### Phase 3 — Agent loop + tools + prompt + DeepSeek LLM

**Goal:** a scripted loop that reconstructs requests from the log.

- [ ] Tool pipeline: freeze args → pre-execute → approval → guards → execute → post-execute → finalize; Code Mode collapse before policy.
- [ ] System-prompt assemble + strict `{{var}}` interpolation + runtime-context snapshot identity.
- [ ] Loop phase machine: idle / maintenance / running; turn opens before first claim; empty first claim still logs a turn; sticky `max-tokens`; abort drain + `ABORTED_BEFORE_DISPATCH`.
- [ ] `dsh-llm` stream contract: usage before finish; adapter failures as terminal `finish`, not thrown; idle watchdog per-read.
- [ ] `dsh-llm-deepseek`: `POST /chat/completions` SSE, thinking/effort mapping, `""` not `null` content, identity headers, compact header.
- [ ] Credentials per-request resolve; never store the key on the adapter.
- [ ] Exit: ported loop/cancel/reconstruction/tool-calls tests plus DeepSeek serialize/SSE/translate tests against a mock server.

### Phase 4 — Local execution world

**Goal:** bash/fs tools on POSIX with fail-closed sandbox.

- [ ] `tokio` process trees with POSIX groups; Windows `taskkill /T` equivalent later in this phase or Phase 8.
- [ ] `confine()` wraps argv; spawn existing `landlock-run` / `bwrap` / `sandbox-exec`.
- [ ] fs-local + fs-sandbox + observation policy; `rg --no-config`.
- [ ] Shell `resolve` then `run`/`start`.
- [ ] Exit: sandbox unavailable fails closed; Landlock 125+fatal-line classified as launcher failure; observation `FS_NOT_OBSERVED`; search `--no-config` invariant tests.

### Phase 5 — Headless + SDK JSON-RPC (first shipping Rust product)

**Goal:** Python SDK and `dsh --profile headless` can run on the Rust bin.

- [ ] `dsh` clap launcher: `--profile`, `--patch`, `web` alias, `plugin` deferred or still TS.
- [ ] Headless: one positional task; print last assistant text; exit 0 iff final `turn/end` is `completed`.
- [ ] SDK server: NDJSON JSON-RPC 2.0; methods and notifications listed above.
- [ ] `python/sdk-runtime` points at the Rust exe; keep `DSH_CORDIS_CONFIG` or a documented Rust composition file.
- [ ] Port `examples/jsonrpc-agent` and `examples/headless-agent` snapshot **drivers** to the Rust bin; keep fixture directories.
- [ ] Exit: keyless JSON-RPC and headless snapshot suites green on the Rust bin; Python keyless SDK tests green against the Rust exe.

### Phase 6 — Interaction, compaction, context, skills, web search, jobs, in-process subagents

**Goal:** `dsh-base` / `standard` preset behavior on Rust headless.

- [ ] Approval waterfall fail-closed; permission presets pin at session creation.
- [ ] Compaction surface replace + log-only lock; token-meter heuristic parity; overflow retry only if `replaceGeneration` advanced.
- [ ] agent-instructions, optional time-context; skill catalog + `skill` tool.
- [ ] DeepSeek `web_search`; `web_fetch` remains off (SSRF).
- [ ] Jobs in-process; in-process subagent spawn/fork + continuation manager.
- [ ] Exit: headless snapshots for compaction-recovery, provider-retry, agent-instructions resume, and a subagent scenario green.

### Phase 7 — Rust GUI host (TypeScript UI unchanged)

**Goal:** `dsh web` can be the Rust host serving `apps/web` dist.

- [ ] axum: `POST /api/<method>`, `POST /api/respond`, WS `/api/events.mux` and `/api/events.host`, `GET /api/session.export`, static SPA, `/plugins/<id>/client.js`, `__DSH_BOOT__`.
- [ ] Implement `RpcMethodMap` plus Typert slash remotes used by the shipped roster (or a frozen descriptor dump).
- [ ] Loopback privilege fence for settings/credentials/preset authoring.
- [ ] Agent/Session lookup equivalence with today’s `agentFor()` (live reuse, cold resume + preset, concurrent dedupe, subagent-owned id rejected).
- [ ] Exit: `pnpm run test:web` (replay) against the Rust host on Linux.

### Phase 8 — Optional capabilities

Each item is its own follow-up note + plan. Order is not load-bearing.

- [ ] ACP server (byte-compatible subset).
- [ ] MCP client (`mcp__` names).
- [ ] Terminal + PTY (POSIX only) via `portable-pty`; keep Linux stdin-wait `/proc` behavior or document the model-visible change.
- [ ] LSP stdio host (transient open, no `applyEdit`).
- [ ] Workflow engine replacing `worker_threads` (hostile-peer JSON protocol).
- [ ] Code Mode runtime replacing `worker_threads`.
- [ ] Windows ACL via the `windows` crate (report `partial`).
- [ ] SQLite session backend and session-query FTS.
- [ ] Out-of-process subagents (ACP / dsh-sdk / Codex / Claude Code).
- [ ] Claude Code / Codex hook bridges.

### Explicitly not a phase

- Rewriting `packages/client` in Rust.
- Rewriting VitePress.
- Rewriting `landlock-run` in Rust.
- Porting E2B adapters.
- Porting `!!js` via QuickJS/Rhai with full `ctx`.
- napi-rs in-process Landlock.
- Making known session-event types depend on which crates are linked.

## Follow-up notes (write when that decision is independent)

Do **not** start from [docs/architecture.md](../../../../docs/architecture.md) or a Superpowers `docs/superpowers/` tree. After a phase ships, update `docs/architecture.md` as a short current-state map only.

| Note | When |
|---|---|
| [2026-08-14-rust-tooling-and-gates.md](../process/2026-08-14-rust-tooling-and-gates.md) | Phase 0 pin, CI path filters, and coverage follow-up |
| [2026-08-15-rust-snapshot-harness.md](../testing/2026-08-15-rust-snapshot-harness.md) | Phase 5 snapshot driver: Vitest stays; spawn the Rust bin for the Phase 5 subset |
| `proposed/architecture/…-rust-gui-host-wire.md` | Phase 7, if the four-quadrant map needs a frozen IDL |
| `proposed/architecture/…-dynamic-plugins.md` | Only if self-modification is reintroduced |

This note partially answers [typed event schemas](2026-06-16-typed-event-schemas.md) for a Rust host: first-party events are a closed enum; `ignorable` covers unknowns; a composition-dependent runtime registry stays rejected for first-party readers. That proposal remains open for the TypeScript tree until the host cutover.

Existing implemented notes listed in the keep-or-drop tables remain authoritative for the contracts this rewrite must preserve. This proposal does not supersede them.

## Alternatives considered

**A. Incremental napi-rs hybrid: keep the Node host, rewrite hot paths in Rust.** Node would still own Cordis, `!!js`, `worker_threads`, tsx, and the plugin graph. napi-rs cannot implement Landlock (restrict-self-then-exec replaces the process). The audit surface becomes two languages plus FFI. Rejected: it does not produce a Rust core/backend, and it adds a third ABI the repo already avoided for Landlock and the Win32 folder dialog.

**B. Rust host + TypeScript web UI over frozen wires (this proposal).** Matches the already-designed GUI split ([GUI layering and RPC](../../implemented/architecture/2026-07-19-gui-layering-and-rpc-protocol.md)): one protocol, swap the carrier. Python already `Popen`s a bin. Highest leverage: Phase 5 replaces the Node runtime the SDK ships without touching React. Cost: the host must reproduce `__DSH_BOOT__`, `/plugins`, and session-event fidelity; Typert must become an IDL or frozen descriptors.

**C. Greenfield Rust including a new GUI (egui/Leptos/Tauri-only).** Deletes ~72k LOC of client UI and the slot/conversation-node extension model in one program. Electron/Tauri is reserved, not shipped. Rejected for v1: it doubles product scope and delays the first shipping Rust agent. Tauri may later reuse the TS UI over IPC `doFetch` without choosing C now.

**D. Keep TypeScript and vendored Cordis.** Zero migration cost. Rejected as the answer to this request; recorded so a later revert has a named alternative.

**E. Reject current session JSONL and start format `1` on day one.** Pre-release *allows* it, but then Rust cannot dual-run or replay 114 committed `session.jsonl` fixtures. Rejected for the rewrite window. The first **structural** change after the Rust reader exists may bump to `1` with the n→n+1 upgrader the version note deferred.

**F. Runtime plugin registry for `SessionEvent` (typetag/inventory).** Closest to declaration merging. The version-mechanism note rejected composition-dependent known-sets: a leaner build would refuse a fuller build’s logs. Rejected for first-party events.

**G. QuickJS/Rhai `!!js` compatibility.** Bit-identical YAML at the cost of embedding an eval story in Rust. Rejected. Closed interpolators plus plugins reading services cover every shipped product row.

**H. In-process Landlock via a Rust `landlock` crate.** Confines the harness. Rejected: the mechanism is self-restrict-then-exec of a helper.

## Acceptance criteria

- A written keep-or-drop table exists (this note) and later implementation PRs cite it rather than re-deciding Cordis, Landlock, `!!js`, or session format.
- Phase 5 exit is observable: keyless headless and JSON-RPC snapshots pass on a Rust bin; Python keyless tests pass against that bin.
- Phase 7 exit is observable: Linux web snapshot replay passes against a Rust host serving the existing TypeScript SPA.
- Session reader refuses foreign format versions and unknown required events; accepts `ignorable: true` leftovers; round-trips packed JSONL fixtures.
- Sandbox fail-closed tests exist before any confined tool ships.
- `native/landlock-run` remains C; the Rust host only spawns it.
- No `!!js` evaluator ships in the Rust compose crate.
- TypeScript packages that still ship keep their Vitest/coverage gates until deleted in the same PR as their crate replacement.
- `docs/architecture.md` is not used as the rewrite spec; it is updated only when a phase ships, as a current-state map.

## Risks

The kernel is small in LOC and dense in invariants (fiber teardown, transactional compose, isolate, waterfall `next()`). A shallow DI crate that omits those will fail preset isolation and fail-loud settlement.

The GUI host must reproduce lookup policy (`agentFor` live/cold/dedupe/subagent fence) and two downlink streams. A “REST rewrite” of `/api` will break the TypeScript client even if unary methods look similar.

PTY idle detection on Linux uses `/proc/<pid>/mem` plus architecture syscall tables. A `portable-pty` port that only waits for prompt/silence changes model-visible send completion.

Windows ACL is partial by construction (Everyone, hard links). “Cleaning it up” in Rust can fail-open or break pwsh startup.

Dual-run can go false-green if snapshots stay on the TypeScript bin after the product bin is Rust, or if both bins run different compositions as the merge gate.

Re-Rusting Landlock or embedding `!!js` will look like simplification and is a recorded mistake.

First tagged release freezes the session-format reader floor. A rewrite that ships as that release must include refusal/degradation for newer logs; missing that behavior cannot be added to copies users already run ([session log version](../../implemented/architecture/2026-08-10-session-log-version-mechanism.md)).
