# Agent Note: Keep Vitest snapshot drivers; spawn the Rust bin for Phase 5 scenarios

Status: proposed

[English](2026-08-15-rust-snapshot-harness.md) | 中文

## 问题

[Rust 重写](../architecture/2026-08-14-rust-rewrite.md)的第 5 阶段把首个交付宿主变成 Rust 二进制（`dsh` headless 与 `dsh-jsonrpc-agent`）。[testing.md](../../../../docs/testing.md) 要求通过真实组装示例做无密钥快照，且[工具链笔记](../process/2026-08-14-rust-tooling-and-gates.md)要求一旦存在二进制，这些快照必须跑构建出的二进制。重写笔记的后续表为此决策留了占位符。若产品二进制已是 Rust 而快照仍跑 TypeScript 二进制，或两种二进制以不同组合作为合并门，双跑会假绿。

## 提案

保留 `examples/jsonrpc-agent/tests/sdk.snapshot.ts` 与 `examples/headless-agent` 中现有的 Vitest 快照驱动。不要把驱动移植到 `cargo test`，也不要复制 fixture 目录。

当 `DSH_RUNTIME=rust` 时，jsonrpc 驱动生成 `target/debug/dsh-jsonrpc-agent`（若设置了 `DSH_RUNTIME_BIN` 则用该路径），而不是 Node 的 `dsh-jsonrpc-agent` 源码启动。同一批 `examples/jsonrpc-agent/tests/snapshots/<name>/` 目录仍是回放语料：驱动仍从 `session.jsonl` 写入 `DSH_SNAPSHOT_FILE`。未设置 `DSH_RUNTIME` 时默认仍是 Node。

Rust 二进制上的第 5 阶段 jsonrpc 场景仅限 `text-turn` 与 `bash-tool`。`subagent-spawn-in-process` 与 `persistent-tools` 留在 Node 驱动，直到后续阶段。除一条新的最小无密钥路径外，现有 `examples/headless-agent` 场景留在 Node 二进制，直到第 6/8 阶段。

不要重录 `notifications.expected.jsonl`、`result.expected.json` 或 fixture 的 `session.jsonl`，除非已证明冻结的 SDK JSON-RPC 方法存在真实线路不匹配。组合更瘦的日志（无会话标题 LLM、无权限预设、无压缩、无 `dsh-base` 工具）是第 5 阶段可接受缺口，不是重写 Node fixture 的理由。在 `DSH_RUNTIME=rust` 上，断言 `result.expected.json` 的 `finalResponse`、最后一条 `session.status` 通知为 `idle`、持久化路径 `$DSH_SESSION_ROOT/<sessionId>/session.jsonl`，以及 initialize 的 `serverInfo.name = deepseek-harness-sdk-runtime`。该启动路径跳过与 Node 录制的通知 JSONL 的全量相等。Node 默认仍保持全量相等。

Cordis、Landlock、`!!js` 与会话格式的保留或放弃引用重写笔记，不要在此复述那些行。

## 第 5 阶段子集

| 场景 | 驱动 | 二进制 | Fixture 目录 |
|---|---|---|---|
| jsonrpc `text-turn` | Vitest `sdk.snapshot.ts` | `DSH_RUNTIME=rust` 时为 Rust；否则为 Node | `examples/jsonrpc-agent/tests/snapshots/text-turn/` |
| jsonrpc `bash-tool` | Vitest `sdk.snapshot.ts` | `DSH_RUNTIME=rust` 时为 Rust；否则为 Node | `examples/jsonrpc-agent/tests/snapshots/bash-tool/` |
| jsonrpc `subagent-spawn-in-process` | Vitest `sdk.snapshot.ts` | 仅 Node | `examples/jsonrpc-agent/tests/snapshots/subagent-spawn-in-process/` |
| jsonrpc `persistent-tools` | Vitest `sdk.snapshot.ts` | 仅 Node | `examples/jsonrpc-agent/tests/snapshots/persistent-tools/` |
| headless compaction-recovery、provider-retry、pty-tools、ralph-loop、goal-tools、advanced-toolchain、subagent-settlement、headless-profile | 现有 Vitest | 仅 Node | 现有目录 |
| headless 最小无密钥 | 第 5 阶段新路径（crate 测试 + 构建出的 `dsh --profile headless`） | Rust | 不要复制那些 fixture 目录 |

## 曾考虑的替代方案

**把快照驱动改写成 Rust（`cargo test` 不生成进程，或 Rust NDJSON 客户端）。** 否决：产品测试是组装后的应用转录；针对不同组合的第二套测试正是重写笔记所点名的双跑失败。Vitest 已经拥有归一化、`llm-replay` 灌入与期望输出比较。

**针对第 5 阶段最小二进制重录每份 jsonrpc 期望 JSONL。** 否决，除非方法名、`serverInfo.name` 或冻结线路上的 `session.event` 封套字段是错的。组合更瘦的日志是第 5 阶段可接受缺口，不是 fixture 重写。

**丢掉 Vitest，把 crate 测试当作快照门。** 被工具链笔记否决：一旦存在二进制，声称检验交付产品的快照必须跑该二进制。

**在第 5 阶段为每一个现有 headless 与 jsonrpc 场景生成 Rust 二进制。** 否决：那些场景需要第 6/8 阶段能力（压缩、子 agent、PTY、持久工具）。Node 仍是它们的驱动。

## 验收标准

- 重写笔记的后续表链接到本文件，而不是占位符 ``proposed/testing/…-rust-snapshot-harness.md``。
- `DSH_RUNTIME=rust` 被记录为 Vitest 启动开关；未设置时保持 Node。
- 第 5 阶段将 `text-turn` 与 `bash-tool` 命名为 jsonrpc 的 Rust 子集，并点名仍留在 Node 上的 headless 场景。
- Fixture 目录被复用；计划不增加并行的 `*.rust.expected.jsonl` 文件。
- 不编辑 `docs/architecture.md`。

## 风险

评审者可能把 Rust 路径上跳过通知 JSONL 全量相等视为削弱快照门。Node 默认仍钉住完整转录；在第 6 阶段组合能匹配 Node 事件流之前，Rust 路径钉住交付 SDK 结果、idle 状态、持久化路径和 `serverInfo.name`。
