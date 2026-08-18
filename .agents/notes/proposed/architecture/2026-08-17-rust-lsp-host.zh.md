# Agent Note: 冻结 Rust LSP stdio 宿主与具名 ACP lsp-definition

Status: proposed

[English](2026-08-17-rust-lsp-host.md) | 中文

## 问题

[Rust 重写](2026-08-14-rust-rewrite.md)第 8 阶段第 4 项是 LSP stdio 宿主（瞬时打开，无 `workspace/applyEdit`）。TypeScript 已经交付三个包（`@deepseek-ai/dsh-lsp`、`@deepseek-ai/dsh-lsp-stdio`、`@deepseek-ai/dsh-tool-lsp`）、封闭的四操作 `lsp` 工具，以及具名 ACP（Agent Client Protocol）快照 `lsp-definition`。Rust 宿主没有 LSP crate。若没有冻结，移植可以合并这三种角色、依赖 `tower-lsp` / `async-lsp` / `lsp-types` / `rmcp`、复用 `dsh-mcp-client` 的 Content-Length 分帧、发送宿主 PID 而不是 `processId: null`、应用 `workspace/applyEdit`、用经过 CRLF 规范化的 `read_text` 供给 `didOpen`、把 LSP 挂到共享的 `rust.snapshot.cordis.yml` 上、增加 `--profile lsp`，或把 LSP 配置项放进默认 YAML。

## 提案

第 8 阶段第 4 项在 Rust 宿主上实现既有的 LSP 能力，YAML 配置项名为 `@deepseek-ai/dsh-lsp`、`@deepseek-ai/dsh-lsp-stdio` 与 `@deepseek-ai/dsh-tool-lsp`。保持三个 crate。面向模型的工具名是 `lsp`。操作是封闭联合 `goToDefinition` | `findReferences` | `goToImplementation` | `hover`。Cordis、Landlock、`!!js`、会话格式、rusqlite 与 `native/landlock-run` 的保留或放弃引用[重写笔记](2026-08-14-rust-rewrite.md)。第 8 阶段第 4 项不增加 rusqlite、不移植 `!!js`、不改写 `landlock-run`、不依赖 `tower-lsp`、`async-lsp`、`lsp-types` 或 `rmcp`，也不编辑 [docs/architecture.md](../../../../docs/architecture.md)。

本笔记并不取代 [LSP 能力缝](../../implemented/architecture/2026-07-15-lsp-capability-seam.md)。那篇笔记仍是 TypeScript 约定的所有者。本笔记记录 Rust stdio 子集与具名 Vitest 切换点。

`register_base_plugins` 注册这三种插件类型，使 `--patch` / overlay 可以挂载它们。默认的 headless、ACP、web 与 jsonrpc 组合省略 LSP 配置项，且不得 spawn 语言服务器子进程。没有 `--profile lsp`。

## 线路冻结

在 `dsh-lsp-stdio` 内复制 Content-Length JSON-RPC 分帧（`Content-Length: {n}\r\n\r\n`）。不要导入 `dsh-mcp-client` 的分帧。不要抽出 `dsh-content-length`。ACP 保持 NDJSON。MCP 的 Content-Length 留在 `dsh-mcp-client`。initialize 发送 `processId: null`（JSON null，不是省略，也不是宿主 PID）。`workspace/applyEdit` 以 JSON-RPC 错误码 `-32601` 和消息 `workspace/applyEdit is not permitted by this host` 拒绝；查询继续服务。宿主从不应用该编辑，也从不运行 `workspace/executeCommand`。`LspService` 只暴露 `register_provider` 与 `query`。`didOpen` 文本来自 `LocalFileSystem::stream_text`（原始 UTF-8 块，不改写 CRLF）。LSP 不得调用 `read_text`。快照读取不得发出 `fs/observed`。Spawn 使用 `SubprocessHandle` 的 Pipe stdin/stdout（不是第二条 tokio `Command`）。`dsh-tool-lsp` 只读取 `ToolExecution` 上的 `session_cwd`，且不得依赖 `dsh-agent`。`dsh-agent` 不得依赖任何 LSP crate。

具名 ACP `lsp-definition` 使用 overlay `examples/acp-agent/rust.lsp.snapshot.cordis.yml`（无 `!!js`、fixture 二进制 argv、`maxLocations: 1`）。共享的 `rust.snapshot.cordis.yml` 不含 LSP 配置项。

## 第 8 阶段 LSP 子集

| 范围 | 第 8 阶段第 4 项 Rust | 留在 TypeScript / 后续 |
|---|---|---|
| ACP `lsp-definition` | `DSH_RUNTIME=rust` 时的 Vitest | |
| 协议 / 分帧 / applyEdit / jail / UTF-16 / `includeDeclaration` | crate 测试 + crate 内 Content-Length fixture | |
| 真实 `typescript-language-server` e2e | 不在范围内 | TypeScript `typescript-server.e2e.ts` |
| 共享的 `rust.snapshot.cordis.yml` | 保持不含 LSP 配置项 | |
| 对着 Rust 跑完整的 `pnpm run test:snapshot` | 不在范围内 | 重写计划的退出条件 |

## 曾考虑的替代方案

**依赖 `tower-lsp`、`async-lsp` 或 `lsp-types`。** 否决：`tower-lsp` 是服务器框架；`async-lsp` 暴露 `apply_edit`；`lsp-types` 已停止维护且覆盖整份规范。工作区 rustc 是 1.85。宿主是自有的瘦客户端。

**为 JSON-RPC 依赖 `rmcp`。** 否决：`rmcp` 是 MCP 而不是 LSP，并且已经不满足 1.85 的 MSRV 钉住。

**导入 `dsh-mcp-client::rpc` 或抽出 `dsh-content-length`。** 否决：LSP 需要 MCP 没有的头与消息上限；共享 crate 会改动已交付的 MCP；MCP 错误字符串不得泄漏进 LSP 诊断。

**用 `read_text` 做 `didOpen`。** 否决：`read_text` 会把 `\r\n` 改写成 `\n`，从而在 CRLF 文件上移动 UTF-16 列。

**把 LSP 挂到共享的 `examples/acp-agent/rust.snapshot.cordis.yml` 上。** 否决：handshake 与 `text-turn` 钉住的 schema 会多出 `lsp` 工具。Overlay `rust.lsp.snapshot.cordis.yml` 才是切换文件。

**增加 `--profile lsp` 或默认 YAML 配置项。** 否决：已实现的 profile 仍是 `headless` / `web` / `acp`。交付默认会在每次 headless 运行时 spawn 语言服务器子进程。

**把三种角色合并成一个 crate。** 否决：当这些角色独立演化时，重写保持 Definition / Provider / Consumer 分离。TypeScript 已经拆开它们。

**因为有些服务器会发送 `workspace/applyEdit` 就接受它。** 否决：TypeScript 宿主以 `-32601` 拒绝该请求并继续服务。宿主绝不可应用该编辑。

## 验收标准

- 重写笔记的后续表链接到本文件。
- YAML 名是 `@deepseek-ai/dsh-lsp`、`@deepseek-ai/dsh-lsp-stdio` 与 `@deepseek-ai/dsh-tool-lsp`；操作保持四臂封闭联合。
- `processId` 是 JSON `null`；`workspace/applyEdit` 是 `-32601`；Content-Length 只存在于 `dsh-lsp-stdio`。
- `didOpen` 使用 `stream_text`；spawn 使用 Pipe 句柄；`session_cwd` 在 `ToolExecution` 上。
- 具名 Vitest ACP 子集增加 `lsp-definition`；overlay 是 `rust.lsp.snapshot.cordis.yml`；共享的 rust ACP YAML 不含 LSP 配置项；默认 YAML 省略 LSP；没有 `--profile lsp`。
- 不编辑 [docs/architecture.md](../../../../docs/architecture.md)。
- 本笔记并不取代 TypeScript LSP 能力缝笔记。

## 风险

评审者可能把 ACP NDJSON 或 MCP 分帧当作语言服务器编解码。帧格式不同；LSP 在 `dsh-lsp-stdio` 内复制 Content-Length。

评审者可能把 LSP 配置项加进共享的 `rust.snapshot.cordis.yml`。Handshake schema 会漂移。

评审者可能为了表现得像完整编辑器客户端而接受 `workspace/applyEdit`。宿主必须以 `-32601` 拒绝，且从不应用该编辑。
