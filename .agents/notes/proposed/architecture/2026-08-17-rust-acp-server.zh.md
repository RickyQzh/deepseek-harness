# Agent Note: 冻结 Rust ACP stdio 子集，并命名第 8 阶段 Vitest 场景

Status: proposed

[English](2026-08-17-rust-acp-server.md) | 中文

## 问题

[Rust 重写](2026-08-14-rust-rewrite.md)第 8 阶段第 1 项是 ACP（Agent Client Protocol）服务器（字节兼容子集）。TypeScript `@deepseek-ai/dsh-acp` 已经在 JSON-RPC stdio 上实现仅面向自动化的 ACP 桥接层。第 5/6 阶段交付了另一套 NDJSON JSON-RPC 运行时（`dsh-sdk-jsonrpc-server`，`serverInfo.name = deepseek-harness-sdk-runtime`）。若没有冻结，Rust 移植可以混用这两套协议、使用 LSP `Content-Length` 分帧、把 `dsh --profile acp` 当作仍未实现，或把完整的 `examples/acp-agent` 快照矩阵当作切换点。

## 提案

第 8 阶段第 1 项在 Rust `dsh` 二进制上实现既有的仅面向自动化 ACP 约定，入口为 `--profile acp` / argv1 `acp`。分帧是换行分隔的 JSON-RPC 2.0（`ndJsonStream`），不是 LSP 的 `Content-Length`。`agentInfo.name` 是 `deepseek-harness-acp`，`agentInfo.version` 是 `0.0.1`。`protocolVersion` 是 `1`。Cordis、Landlock、`!!js`、会话格式、rusqlite 与 `native/landlock-run` 的保留或放弃引用[重写笔记](2026-08-14-rust-rewrite.md)，不要在此复述那些行。第 8 阶段第 1 项不增加 rusqlite、不移植 `!!js`、不改写 `landlock-run`，也不编辑 [docs/architecture.md](../../../../docs/architecture.md)。

本笔记并不取代 [ACP 作为仅面向自动化的协议](../../implemented/simplification/2026-07-23-acp-automation-only-protocol.md)。那篇笔记仍是 TypeScript 约定的所有者。本笔记记录 Rust 子集与具名 Vitest 切换点。

已实现的 client→agent 方法是 `initialize`、`authenticate`（空操作 `{}`）、`session/new` 和 `session/prompt`。已实现的通知是 `session/cancel`。出站方法是 `session/update`（仅 `agent_message_chunk` 文本）和 `session/request_permission`（一次性 `allow-once` / `reject-once`）。其他 ACP 请求一律返回 JSON-RPC `-32601`，消息为 `"Method not found": <method>`。校验失败返回 `-32602`，带 TypeScript 的 `Invalid params: …` 详情字符串。关联的轮次错误以 `-32603` 拒绝 `session/prompt`。

第 8 阶段第 1 项把 `pnpm run demo:acp` 保持为 TypeScript 示例。具名场景在 `DSH_RUNTIME=rust` 时 spawn `target/debug/dsh --profile acp`，并记录在[快照 harness 笔记](../testing/2026-08-15-rust-snapshot-harness.md)。其余 ACP 快照场景留在 Node `dsh-acp-demo` 二进制上，并复用 [ACP 快照测试](../../implemented/testing/2026-06-19-acp-snapshot-tests.md) 的 fixture（测试前置数据）目录。对着 Rust 跑完整的 `pnpm run test:snapshot` 不是第 8 阶段第 1 项的退出条件。

## 线路冻结

传输是每行一个 UTF-8 JSON 对象。stdout 只承载这些帧。诊断信息走 stderr。没有就绪 URL 行。

`session/prompt` 阻塞到整个 agent（智能体）空闲，并返回 `{ "stopReason": … }`。token 上限导致的轮次结束结算为 `end_turn`。显式取消、dispose（资源释放）或无轮次槽位结算为 `cancelled`。编解码器把 `max-tokens` 映射为 `max_tokens` 供其他调用方使用；提示词 RPC 不使用该值。

权限答复绝不会变成持久授权。缺少 `callId`，或桥接层并不拥有该会话时，用 `next()` 继续委派。客户端错误失败关闭为 `unavailable`。未知 `optionId` 为 `rejected`。

## 第 8 阶段 ACP 子集

| 场景 | 驱动 | 二进制 | Fixture 目录 |
|---|---|---|---|
| ACP `handshake` | Vitest `examples/acp-agent/tests/acp.snapshot.ts` | `DSH_RUNTIME=rust` 时为 Rust；否则为 Node | `examples/acp-agent/tests/snapshots/handshake/` |
| ACP `reject-extra-dirs` | 同上 | `DSH_RUNTIME=rust` 时为 Rust；否则为 Node | `examples/acp-agent/tests/snapshots/reject-extra-dirs/` |
| ACP `text-turn` | 同上 | `DSH_RUNTIME=rust` 时为 Rust；否则为 Node | `examples/acp-agent/tests/snapshots/text-turn/` |
| 其余 ACP 场景 | 同一套件 | 仅 Node | 现有目录 |

## 曾考虑的替代方案

**因为公开 ACP 规范示例经常展示 LSP Content-Length，所以采用它。** 否决：本仓库的 TypeScript 服务器、快照启动器、subagent 客户端和 handshake 预期输出都是 `ndJsonStream` NDJSON。

**因为两者都是 NDJSON JSON-RPC，所以把 ACP 路由进 `dsh-sdk-jsonrpc-server`。** 否决：`session/prompt` 语义、会话生命周期、通知和进程身份都不同。共享分发器会混用线路。

**交付与 `dsh-jsonrpc-agent` 并列的 `dsh-acp-agent` 二进制。** 为第 8 阶段第 1 项否决：ACP 客户端 spawn 一条已配置命令；重写把 ACP 放在同一个 `dsh` 宿主上。Python 不会 Popen 一个 ACP 专用名字。

**把完整的 `examples/acp-agent` 快照当作切换点。** 否决：那份语料钉住后端工具、PTY、LSP、工作流、Code Mode、钩子和 subagent。第 8 阶段第 1 项命名三个协议场景。与第 5/6/7 阶段相同的具名子集模式。

**因为 `authMethods` 为空而跳过 `authenticate`。** 否决：TypeScript 实现了空操作，并且测试会调用它。

**为 continuable drain 依赖 `dsh-subagent`。** 为第 8 阶段第 1 项否决：TypeScript 用结构性的 `ctx.get` 避开对该包的依赖。进程外 subagent 是第 8 阶段的后续项。

## 验收标准

- 重写笔记的后续表链接到本文件。
- 分帧是 NDJSON JSON-RPC 2.0；`agentInfo.name` 是 `deepseek-harness-acp`。
- 已实现方法是上文列出的五项外加两个出站方法；其他请求为带 TypeScript 消息的 `-32601`。
- 具名 Vitest ACP 子集是 `handshake`、`reject-extra-dirs` 与 `text-turn`；其余 ACP 场景留在 Node。本笔记不声称在 Rust 上覆盖完整 ACP 快照矩阵。
- 不编辑 [docs/architecture.md](../../../../docs/architecture.md)。
- 本笔记并不取代仅面向自动化的 ACP 笔记。

## 风险

评审者可能把具名子集当作在 Rust 上跑完整的 `pnpm run test:snapshot`。其余场景留在 Node。

评审者可能让 Python 或 GUI 指向 ACP stdio。那些线路仍是 SDK NDJSON 与四象限 HTTP/WS。

没有 `headless-auto-approve` 的 `dsh --profile acp` 在工具审批上失败关闭，除非客户端回答 `session/request_permission`。这就是 TypeScript 约定。
