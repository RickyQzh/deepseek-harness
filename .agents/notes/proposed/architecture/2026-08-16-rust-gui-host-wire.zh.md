# Agent Note: 冻结 Rust GUI 宿主的四象限线路，并命名第 7 阶段 Vitest 子集

Status: proposed

[English](2026-08-16-rust-gui-host-wire.md) | 中文

## 问题

[Rust 重写](2026-08-14-rust-rewrite.md)第 7 阶段的目标是 Rust 的 `dsh web` / `dsh --profile web` 宿主，用来服务现有的 `apps/web` dist。四象限协议已经写在 [GUI 分层与 RPC 协议](../../implemented/architecture/2026-07-19-gui-layering-and-rpc-protocol.md) 和 [WebSocket 下行载体](../../implemented/architecture/2026-08-04-websocket-downlink-carrier.md) 中。那些笔记仍是协议的所有者。

若没有第 7 阶段冻结，Rust 宿主可以发明第二套协议、把网络 SSE（Server-Sent Events）留作浏览器回退、移植每一个 Typert 斜杠 Remote、绑定 `--host 0.0.0.0`，或把完整的 `pnpm run test:web` 当作切换点。重写笔记的后续表为此冻结留了占位符。

## 提案

第 7 阶段在一条 loopback 监听器上实现既有的四象限 HTTP + WebSocket 协议。一元调用仍是 `POST /api/<method>` 的 JSON `client-request`。下行只走 WebSocket，路径为 `/api/events.mux` 与 `/api/events.host`。对这些路径的网络 GET 或 HEAD 返回 426 Upgrade Required，并带 `Upgrade: websocket`。进程内 SSE 不是浏览器回退；该规则已在 WebSocket 下行载体笔记中。

`packages/client/*` 与 `apps/web` 仍为 TypeScript。Cordis、Landlock、`!!js`、会话格式、rusqlite 与 `native/landlock-run` 的保留或放弃引用[重写笔记](2026-08-14-rust-rewrite.md)，不要在此复述那些行。第 7 阶段不增加 rusqlite、不移植 `!!js`、不改写 `landlock-run`，也不编辑 [docs/architecture.md](../../../../docs/architecture.md)。

两套 RPC 方言共用 `/api`。分发顺序是信任围栏，然后特权再检查，若 `/api/` 之后的路径含 `/` 则走斜杠拦截器，否则走点分 `RpcMethodMap`。未知点分方法为 HTTP 404。未知斜杠命名空间为 HTTP 404。

第 7 阶段的斜杠 Remote 只有 `commands/list` 与 `commands/execute`，payload 为 `{ args }`。`goals/*`、`pluginInventory/list`、`messageFeedback/*` 与 `dsh-cordis-host-runner` Remote 为 HTTP 404。UI 降级；不要 stub 假成功。`GET /api/session.export` ZIP 不在第 7 阶段映射中（HTTP 404 或业务 `internal`）。

`--host 0.0.0.0` 按 TypeScript CLI（命令行界面）的安全文案拒绝。绑定 `127.0.0.1`。默认监听端口是 `3080`。端口 `0` 请求操作系统分配的端口。`trustedHosts` 仍可为 loopback 部署命名额外 authority；它们不能解除特权方法的 loopback 钉住。

`host.describe.version` 是 `0.0.1`。`canOpenPath` 为 false。`web_fetch` 保持关闭。没有 CORS；跨站防御是 `Content-Type: application/json` 加上 Host/Origin/`sec-fetch-site` 围栏。

`POST /api/respond` 返回 `RpcReceipt`（`{accepted:true}` / `{accepted:false,reason}`），不是 `RpcMessage`。GET `/` 服务 SPA，并注入 `window.__DSH_BOOT__`，JSON 内把 `<` 转义为 `\u003c`。GET `/plugins/<id>/client.js` 服务静态插件 bundle。就绪 URL 行是 stdout 或 stderr 上的 `dsh web: http://127.0.0.1:<port>`。

本笔记并不取代 GUI 分层笔记或 WebSocket 下行载体笔记。不要归档或改写那些 TypeScript 协议笔记。

Rust `dsh` 二进制上的第 7 阶段具名 web 场景是 `rust-host-smoke` 与 `cold-blank-session`，记录在[快照 harness 笔记](../testing/2026-08-15-rust-snapshot-harness.md)。其余 `test:web` 文件留在 Node scaffold 或 jsdom。对着 Rust 跑完整的 `pnpm run test:web` 是重写计划的退出条件，不是第 7 阶段的切换点。

## 线路冻结

| 方法 | 路径 | 行为 |
|---|---|---|
| POST | `/api/<dotted>` | JSON `client-request`；业务错误为 HTTP 200 + `server-response` |
| POST | `/api/respond` | `RpcReceipt`（`{accepted:true}` / `{accepted:false,reason}`），不是 RpcMessage |
| GET/HEAD | `/api/events.mux` 或 `/api/events.host` | 426 + `Upgrade: websocket` |
| WS | `/api/events.mux`、`/api/events.host` | 仅下行 |
| GET | `/plugins/<id>/client.js` | 静态插件 bundle |
| GET | `/` | SPA + `window.__DSH_BOOT__`（JSON 内 `<` → `\u003c`） |
| POST 非 JSON | `/api/*` | 415 `content type must be application/json` |
| 不受信任的 `/api` | | 403 |
| 特权方法 + 非 loopback | | 即使 Host 在 trustedHosts 中仍为 403 |

特权点分方法是 TypeScript 集合 [`PRIVILEGED_METHODS`](../../../../packages/client/connection/src/index.ts)（即使 `trustedHosts` 能通过外层围栏，仍要求 loopback Host）：

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

冻结的信封示例（camelCase 的 `rpcId`；`RpcReceipt` 没有 `type` 字段）：

```text
{"type":"client-request","rpcId":"r1","method":"host.describe","payload":{}}
{"type":"server-response","rpcId":"r1","result":{"ok":true,"value":{"version":"0.0.1","cwd":"/work","attachedSessions":0,"canOpenPath":false}}}
{"type":"server-request","rpcId":"p1","method":"session/event","payload":{}}
{"type":"client-response","rpcId":"p1","result":{"ok":true,"value":{}}}
{"accepted":true}
{"accepted":false,"reason":"not-pending"}
```

## 第 7 阶段子集

| 场景 | 驱动 | 二进制 | Fixture 目录 |
|---|---|---|---|
| web `rust-host-smoke` | Vitest `apps/web/tests/rust-host-smoke.e2e.ts` | `DSH_RUNTIME=rust` 时为 Rust；否则跳过 | 无 |
| web `cold-blank-session` | Vitest `cold-blank-session.e2e.ts` | `DSH_RUNTIME=rust` 时为 Rust；否则为 Node scaffold | `apps/web/tests/snapshots/cold-blank-session/` |
| 其余 `test:web` 文件 | 现有 Vitest | Node scaffold / jsdom | 现有目录 |

## 曾考虑的替代方案

**发明第二套协议或重命名四象限方法。** 否决：GUI 四象限方法名不变。TypeScript 笔记仍是协议的所有者；本笔记记录第 7 阶段实现的 HTTP/WS 路径。

**为 `/api/events.*` 保留网络 SSE 回退。** 被 WebSocket 下行载体笔记否决：网络 GET 只回答 Upgrade Required。

**移植 Typert 分析器与每一个斜杠 Remote。** 否决：第 7 阶段的斜杠 Remote 只有 `commands/list` 与 `commands/execute`。未知斜杠命名空间为 HTTP 404。

**允许 `--host 0.0.0.0` / 局域网绑定。** 否决：TypeScript CLI 已经把它当作远程 RCE 拒绝。只绑定 `127.0.0.1`。

**把完整的 `pnpm run test:web` 当作第 7 阶段切换点。** 否决：那是重写计划的退出条件。第 7 阶段命名 `rust-host-smoke` 与 `cold-blank-session`。其余 web e2e 留在 Node，与第 5/6 阶段相同的具名子集模式。

**改写或归档 TypeScript GUI 协议笔记。** 否决：本笔记引用它们，并不取代它们。

## 验收标准

- 重写笔记的后续表链接到本文件，而不是占位符 ``proposed/architecture/…-rust-gui-host-wire.md``。
- 对 `/api/events.mux` 与 `/api/events.host` 的网络 GET/HEAD 为 426 + `Upgrade: websocket`；进程内 SSE 不是浏览器回退。
- 第 7 阶段的斜杠 Remote 只有 `commands/list` 与 `commands/execute`，payload 为 `{ args }`；`goals/*` 与 cordis-host-runner 为 404。
- `--host 0.0.0.0` 被拒绝；绑定为 `127.0.0.1`；默认端口为 3080；`canOpenPath` 为 false；`web_fetch` 保持关闭；`host.describe.version` 为 `0.0.1`。
- 具名 Vitest web 子集是 `rust-host-smoke` 与 `cold-blank-session`；其余 `test:web` 文件留在 Node。本笔记不声称在 Rust 上跑完整的 `pnpm run test:web`。
- 不编辑 [docs/architecture.md](../../../../docs/architecture.md)。
- 本笔记并不取代 GUI 分层或 WebSocket 下行载体笔记。

## 风险

评审者可能把具名 web 子集当作在 Rust 上跑完整的 `pnpm run test:web`。其余 `test:web` 文件留在 Node scaffold 或 jsdom。对着 Rust 的完整回放仍是[重写笔记](2026-08-14-rust-rewrite.md)中的重写计划退出条件。

对 `goals/*` 与 `dsh-cordis-host-runner` 返回 404 会使 TypeScript UI 降级。stub 假成功会对着不同产品双跑。

即使 `trustedHosts` 承认非 loopback Host，特权方法仍钉在 loopback。去掉那次再检查会把 settings、credentials 与 preset 编写暴露到 loopback 之外。
