# Agent Note: 冻结 Rust MCP stdio 客户端与 mcp__ 公开名称

Status: proposed

[English](2026-08-17-rust-mcp-client.md) | 中文

## 问题

[Rust 重写](2026-08-14-rust-rewrite.md)第 8 阶段第 2 项是 MCP 客户端（`mcp__` 名称）。TypeScript `@deepseek-ai/dsh-mcp-client` 已经连接外部 MCP 服务器，在 `mcp__<serverName>__<rawName>` 下注册工具，并重连崩溃的 stdio 子进程。Rust 宿主没有这样的插件。若没有冻结，移植可以在 MCP stdio 上说 ACP（Agent Client Protocol）NDJSON、尽管 rustc 为 1.85 仍依赖 `rmcp`、在 `MINIMAL_YAML` 中挂载默认 MCP 服务器、把 Streamable HTTP 或 Resources 当作范围内，或增加 TypeScript 包刻意不做的快照场景。

## 提案

第 8 阶段第 2 项在 Rust 宿主上实现既有的 MCP **客户端**约定，YAML 配置项名为 `@deepseek-ai/dsh-mcp-client`。每一项通过 stdio 连接一台服务器。分帧是 LSP `Content-Length` JSON-RPC 2.0，不是 ACP NDJSON。`clientInfo.name` 是 `dsh-mcp-client`，`clientInfo.version` 是 `0.0.1`。公开工具名保持 TypeScript 的 `publicToolName` 函数，参数为 `(serverName, rawName)`。Cordis、Landlock、`!!js`、会话格式、rusqlite 与 `native/landlock-run` 的保留或放弃引用[重写笔记](2026-08-14-rust-rewrite.md)。第 8 阶段第 2 项不增加 rusqlite、不移植 `!!js`、不改写 `landlock-run`、不依赖 `rmcp`，也不编辑 [docs/architecture.md](../../../../docs/architecture.md)。

本笔记并不取代 [MCP 客户端插件](../../implemented/feature/2026-07-07-mcp-client-plugin.md) 或 [MCP 客户端自动重连](../../implemented/feature/2026-08-06-mcp-client-auto-reconnect.md)。那两篇笔记仍是 TypeScript 约定的所有者。本笔记记录 Rust stdio 子集。

已实现的线路方法是 `initialize`、`notifications/initialized`、分页的 `tools/list`、使用原始 MCP 名称的 `tools/call`、`notifications/tools/list_changed`，以及工具 abort 信号触发时的 `$/cancelRequest`。Resources、Prompts、elicitation、sampling、OAuth 与 Streamable HTTP 均不在范围内：YAML 的 `transport` 若不是 `stdio` 则加载失败。

`register_base_plugins` 注册插件类型，使显式 overlay 可以解析 `@deepseek-ai/dsh-mcp-client`。默认的 headless、ACP、web 与 jsonrpc 组合不挂载服务器配置项，且不得 spawn MCP 子进程。没有 `--profile mcp`。

## 线路冻结

stdio 帧是 `Content-Length: <byte-length>\r\n\r\n` 加上等量的 UTF-8 JSON 字节。harness 的诊断信息走 stderr。子进程的 stdin/stdout 是 MCP 流。子进程环境从 `scrubbed_parent_env()` 起步，再应用 `config.env`。

`serverName` 是匹配 `^[A-Za-z0-9_-]{1,32}$` 的本地配置，不是远端 `serverInfo.name`。同一 `tools` 运行时上第二个使用相同 `serverName` 的活跃实例会以 TypeScript 的重复命名空间句子加载失败。`tools/call` 始终发送 `rawName`。公开名称是 `mcp__<serverName>__<rawName>`，除非 DeepSeek 函数名规范化（64 个字符，`[A-Za-z0-9_-]`）改变了该字符串，此时追加 `serverName + NUL + rawName` 的 SHA-256 的 12 个十六进制字符。

重连匹配 TypeScript 监督器：有界指数退避、每次故障的 `maxAttempts`、等于 `maxDelayMs` 的稳定窗口、故障期间工具保持注册、耗尽则注销它们。`reconnect.enabled: false` 保持 v1 的手动恢复行为。Rust 宿主没有 HMR（热模块替换）。

## 第 8 阶段 MCP 子集

| 范围 | 本项 Rust | 留在 TypeScript / 后续 |
|---|---|---|
| stdio 工具 + `mcp__` 名称 + 重连 | 对着 crate 内 Content-Length fixture（测试前置数据）的 crate 测试 | |
| Streamable HTTP | 不在范围内 | `mcp-client.e2e.ts` HTTP |
| `@modelcontextprotocol/server-everything` / filesystem e2e | 不在范围内 | Node e2e |
| 具名 Vitest 快照 | 无 | 无（TypeScript 也无） |
| `examples/mcp-memory` overlay | 不在范围内（`!!js`） | Node |

## 曾考虑的替代方案

**因为 `rmcp` 3.x 是官方 Rust MCP SDK，所以依赖它。** 为第 8 阶段第 2 项否决：`rmcp` 3.1.2 声明 rustc 1.88；工作区钉住的是 1.85。此处不要抬高 rustc。

**因为 ACP 与 SDK JSON-RPC 已经使用 NDJSON，所以在 MCP stdio 上使用 NDJSON。** 否决：TypeScript MCP SDK 的 stdio 传输是 Content-Length。把 ACP 分帧混到 MCP 子进程上是协议错误。

**在 `MINIMAL_YAML` 中挂载默认 MCP 服务器。** 否决：交付默认会在每次 headless 运行时 spawn 第三方子进程。插件类型已注册；配置项是 opt-in。

**增加具名 Vitest MCP 快照。** 否决：TypeScript MCP 笔记选择了单元/e2e 且不做快照，以使系统提示词 fixture 保持稳定。

**在第 8 阶段第 2 项移植 Streamable HTTP。** 否决：stdio 崩溃恢复是运营约定；HTTP SSE（Server-Sent Events）恢复留在 TypeScript SDK 内，是后续笔记。

## 验收标准

- 重写笔记的后续表链接到本文件。
- 分帧是 Content-Length JSON-RPC 2.0；公开名称匹配锁定的 `publicToolName` 向量。
- 默认组合不 spawn MCP；第 8 阶段第 2 项新增文件中没有 YAML `!!js`。
- 不编辑 [docs/architecture.md](../../../../docs/architecture.md)。
- 本笔记并不取代 TypeScript MCP 客户端或重连笔记。

## 风险

评审者可能因为两者都是 JSON-RPC，而把 ACP NDJSON 当作 MCP stdio 编解码。帧不同。

评审者可能把 `rmcp` 当作必须。否决理由是 MSRV 冲突。

评审者可能要求具名快照。TypeScript MCP 测试没有快照。
