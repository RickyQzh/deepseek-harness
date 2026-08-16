# Agent Note: Freeze the Rust GUI host four-quadrant wire and name the Phase 7 Vitest subset

Status: proposed

English | [中文](2026-08-16-rust-gui-host-wire.zh.md)

## Problem

The [Rust rewrite](2026-08-14-rust-rewrite.md) Phase 7 goal is a Rust `dsh web` / `dsh --profile web` host that serves the existing `apps/web` dist. The four-quadrant protocol already lives in [GUI layering and the RPC protocol](../../implemented/architecture/2026-07-19-gui-layering-and-rpc-protocol.md) and the [WebSocket downlink carrier](../../implemented/architecture/2026-08-04-websocket-downlink-carrier.md). Those notes stay the protocol owners.

Without a Phase 7 freeze, a Rust host can invent a second protocol, keep network SSE as a browser fallback, port every Typert slash remote, bind `--host 0.0.0.0`, or treat full `pnpm run test:web` as the cutover. The rewrite follow-up table left a placeholder for this freeze.

## Proposal

Phase 7 implements the existing four-quadrant HTTP + WebSocket protocol on one loopback listener. Unary calls stay `POST /api/<method>` JSON `client-request`. Downlinks are WebSocket-only on `/api/events.mux` and `/api/events.host`. Network GET or HEAD on those paths returns 426 Upgrade Required with `Upgrade: websocket`. In-process SSE is not a browser fallback; that rule is already in the WebSocket downlink note.

`packages/client/*` and `apps/web` stay TypeScript. Cite the [rewrite note](2026-08-14-rust-rewrite.md) for keep-or-drop of Cordis, Landlock, `!!js`, session format, rusqlite, and `native/landlock-run` rather than restating those rows here. Phase 7 does not add rusqlite, does not port `!!js`, does not rewrite `landlock-run`, and does not edit [docs/architecture.md](../../../../docs/architecture.md).

Two RPC dialects share `/api`. Dispatch order is trust fence, then privileged re-check, then a slash interceptor when the path after `/api/` contains `/`, else the dotted `RpcMethodMap`. Unknown dotted methods are HTTP 404. Unknown slash namespaces are HTTP 404.

Phase 7 slash remotes are only `commands/list` and `commands/execute` with payload `{ args }`. `goals/*`, `pluginInventory/list`, `messageFeedback/*`, and `dsh-cordis-host-runner` remotes are HTTP 404. The UI degrades; do not stub fake success. `GET /api/session.export` ZIP is out of the Phase 7 map (HTTP 404 or business `internal`).

`--host 0.0.0.0` is refused with the TypeScript CLI safety text. Bind `127.0.0.1`. Default listen port is `3080`. Port `0` requests an OS-assigned port. `trustedHosts` still name extra authorities on loopback deployments; they do not lift the privileged loopback pin.

`host.describe.version` is `0.0.1`. `canOpenPath` is false. `web_fetch` stays off. There is no CORS; cross-site defense is `Content-Type: application/json` plus the Host/Origin/`sec-fetch-site` fence.

`POST /api/respond` returns `RpcReceipt` (`{accepted:true}` / `{accepted:false,reason}`), not an `RpcMessage`. GET `/` serves the SPA and injects `window.__DSH_BOOT__` with `<` escaped as `\u003c` inside the JSON. GET `/plugins/<id>/client.js` serves the static plugin bundle. The ready URL line is `dsh web: http://127.0.0.1:<port>` on stdout or stderr.

This note does not supersede the GUI layering note or the WebSocket downlink note. Do not archive or rewrite those TypeScript protocol notes.

Named Phase 7 web scenarios on the Rust `dsh` bin are `rust-host-smoke` and `cold-blank-session`, recorded in the [snapshot-harness note](../testing/2026-08-15-rust-snapshot-harness.md). Remaining `test:web` files stay on the Node scaffold or jsdom. Full `pnpm run test:web` against Rust is the rewrite-program exit, not the Phase 7 cutover.

## Wire freeze

| Method | Path | Behavior |
|---|---|---|
| POST | `/api/<dotted>` | JSON `client-request`; business errors HTTP 200 + `server-response` |
| POST | `/api/respond` | `RpcReceipt` (`{accepted:true}` / `{accepted:false,reason}`), not an RpcMessage |
| GET/HEAD | `/api/events.mux` or `/api/events.host` | 426 + `Upgrade: websocket` |
| WS | `/api/events.mux`, `/api/events.host` | downlink only |
| GET | `/plugins/<id>/client.js` | static plugin bundle |
| GET | `/` | SPA + `window.__DSH_BOOT__` (`<` → `\u003c` in JSON) |
| POST non-JSON | `/api/*` | 415 `content type must be application/json` |
| untrusted `/api` | | 403 |
| privileged + non-loopback | | 403 even if Host is in trustedHosts |

Privileged dotted methods are the TypeScript set in [`PRIVILEGED_METHODS`](../../../../packages/client/connection/src/index.ts) (loopback Host required even if `trustedHosts` would pass the outer fence):

```text
agentPreset.read
agentPreset.copy
agentPreset.openDocument
agentPreset.remove
host.pickDirectory
host.openPath
settings.describe
settings.openDocument
settings.update
settings.replace
settings.mutate
credentials.describe
credentials.set
credentials.unset
llm.discoverModels
```

Locked envelope examples (camelCase `rpcId`; `RpcReceipt` has no `type` field):

```text
{"type":"client-request","rpcId":"r1","method":"host.describe","payload":{}}
{"type":"server-response","rpcId":"r1","result":{"ok":true,"value":{"version":"0.0.1","cwd":"/work","attachedSessions":0,"canOpenPath":false}}}
{"type":"server-request","rpcId":"p1","method":"session/event","payload":{}}
{"type":"client-response","rpcId":"p1","result":{"ok":true,"value":{}}}
{"accepted":true}
{"accepted":false,"reason":"not-pending"}
```

## Phase 7 subset

| Scenario | Driver | Bin | Fixture dir |
|---|---|---|---|
| web `rust-host-smoke` | Vitest `apps/web/tests/rust-host-smoke.e2e.ts` | Rust when `DSH_RUNTIME=rust`; skipped otherwise | none |
| web `cold-blank-session` | Vitest `cold-blank-session.e2e.ts` | Rust when `DSH_RUNTIME=rust`; Node scaffold otherwise | `apps/web/tests/snapshots/cold-blank-session/` |
| remaining `test:web` files | existing Vitest | Node scaffold / jsdom | existing dirs |

## Alternatives considered

**Invent a second protocol or rename four-quadrant methods.** Rejected: GUI four-quadrant method names do not change. The TypeScript notes remain the protocol owners; this note records the HTTP/WS paths Phase 7 implements.

**Keep a network SSE fallback for `/api/events.*`.** Rejected by the WebSocket downlink note: network GET answers only Upgrade Required.

**Port the Typert analyzer and every slash remote.** Rejected: Phase 7 slash remotes are `commands/list` and `commands/execute` only. Unknown slash namespaces are HTTP 404.

**Allow `--host 0.0.0.0` / LAN bind.** Rejected: the TypeScript CLI already refuses it as remote RCE. Bind `127.0.0.1` only.

**Treat full `pnpm run test:web` as the Phase 7 cutover.** Rejected: that is the rewrite-program exit. Phase 7 names `rust-host-smoke` and `cold-blank-session`. Remaining web e2e stay Node, the same named-subset pattern as Phase 5/6.

**Rewrite or archive the TypeScript GUI protocol notes.** Rejected: this note cites them and does not supersede them.

## Acceptance criteria

- The rewrite note follow-up table links to this file instead of the placeholder ``proposed/architecture/…-rust-gui-host-wire.md``.
- Network GET/HEAD `/api/events.mux` and `/api/events.host` is 426 + `Upgrade: websocket`; in-process SSE is not a browser fallback.
- Phase 7 slash remotes are only `commands/list` and `commands/execute` with payload `{ args }`; `goals/*` and cordis-host-runner are 404.
- `--host 0.0.0.0` is refused; bind is `127.0.0.1`; default port is 3080; `canOpenPath` is false; `web_fetch` stays off; `host.describe.version` is `0.0.1`.
- Named Vitest web subset is `rust-host-smoke` and `cold-blank-session`; remaining `test:web` files stay Node. This note does not claim full `pnpm run test:web` on Rust.
- [docs/architecture.md](../../../../docs/architecture.md) is not edited.
- This note does not supersede the GUI layering or WebSocket downlink notes.

## Risks

A reviewer may treat the named web subset as full `pnpm run test:web` on Rust. Remaining `test:web` files stay on the Node scaffold or jsdom. Full replay against Rust remains the rewrite-program exit in the [rewrite note](2026-08-14-rust-rewrite.md).

A 404 on `goals/*` and `dsh-cordis-host-runner` degrades the TypeScript UI. Stubbing success would dual-run against a different product.

Privileged methods stay pinned to loopback even when `trustedHosts` admits a non-loopback Host. Dropping that re-check would expose settings, credentials, and preset authoring off loopback.
