# Agent Note: Keep Vitest snapshot drivers; spawn the Rust bin for Phase 5 scenarios

Status: proposed

[English](2026-08-15-rust-snapshot-harness.md) | 中文

## 问题

[Rust 重写](../architecture/2026-08-14-rust-rewrite.md)的第 5 阶段把首个交付宿主变成 Rust 二进制（`dsh` headless 与 `dsh-jsonrpc-agent`）。[testing.md](../../../../docs/testing.md) 要求通过真实组装示例做无密钥快照，且[工具链笔记](../process/2026-08-14-rust-tooling-and-gates.md)要求一旦存在二进制，这些快照必须跑构建出的二进制。重写笔记的后续表为此决策留了占位符。若产品二进制已是 Rust 而快照仍跑 TypeScript 二进制，或两种二进制以不同组合作为合并门，双跑会假绿。

## 提案

保留 `examples/jsonrpc-agent/tests/sdk.snapshot.ts` 与 `examples/headless-agent` 中现有的 Vitest 快照驱动。不要把驱动移植到 `cargo test`，也不要复制 fixture 目录。

当 `DSH_RUNTIME=rust` 时，jsonrpc 驱动生成 `target/debug/dsh-jsonrpc-agent`（若设置了 `DSH_RUNTIME_BIN` 则用该路径），而不是 Node 的 `dsh-jsonrpc-agent` 源码启动。同一批 `examples/jsonrpc-agent/tests/snapshots/<name>/` 目录仍是回放语料：驱动仍从 `session.jsonl` 写入 `DSH_SNAPSHOT_FILE`。未设置 `DSH_RUNTIME` 时默认仍是 Node。

Rust 二进制上的第 6 阶段 jsonrpc 场景是 `text-turn`、`bash-tool` 与 `subagent-spawn-in-process`。`persistent-tools` 留在 Node 驱动，直到后续阶段交付持久 shell 与 `str_replace_editor`。

Rust `dsh` 二进制上的第 6 阶段 headless 场景是 `compaction-recovery`、`provider-retry`、`agent-instructions` 恢复（`workspace-context-resume.snapshot.ts`）与 `subagent-settlement`。`pty-tools`、`ralph-loop`、`goal-tools`、`advanced-toolchain` 与 `headless-profile` 留在 Node 二进制。

当 `DSH_RUNTIME=rust` 时，headless 驱动 spawn `target/debug/dsh`（若设置了 `DSH_RUNTIME_BIN` 则用该路径），并带上 `--profile headless` 与位置参数任务。静态 YAML 是 `DSH_CORDIS_CONFIG`，指向 `examples/headless-agent/rust.<scenario>.cordis.yml`（不要 `!!js`，不要 include 插件）。持久化路径仍为 `{DSH_SESSION_ROOT}/{sessionId}/session.jsonl`。

在 `DSH_RUNTIME=rust` 上，断言场景特定的持久化事实，外加进程退出码 0 以及最后一段 assistant / stdout。当 Rust 组合省略会话标题 LLM 或本阶段未移植的其他 `dsh-base` 行时，跳过与 Node 录制的 `stream-json.expected.jsonl` 的全量相等。Node 默认仍保持全量相等。不要重录 fixture，除非冻结线路上的字段是错的。

不要重录 `notifications.expected.jsonl`、`result.expected.json` 或 fixture 的 `session.jsonl`，除非已证明冻结的 SDK JSON-RPC 方法存在真实线路不匹配。组合更瘦的日志（无会话标题 LLM、无权限预设、无压缩、无 `dsh-base` 工具）是第 5 阶段可接受缺口，不是重写 Node fixture 的理由。在 `DSH_RUNTIME=rust` 上，断言 `result.expected.json` 的 `finalResponse`、最后一条 `session.status` 通知为 `idle`、持久化路径 `$DSH_SESSION_ROOT/<sessionId>/session.jsonl`，以及 initialize 的 `serverInfo.name = deepseek-harness-sdk-runtime`。该启动路径跳过与 Node 录制的通知 JSONL 的全量相等。Node 默认仍保持全量相等。

Cordis、Landlock、`!!js` 与会话格式的保留或放弃引用重写笔记，不要在此复述那些行。

第 7 阶段 Rust `dsh` 二进制上的具名 web 场景是 `rust-host-smoke` 与 `cold-blank-session`。其余 `test:web` 文件留在 Node scaffold 或 jsdom。对着 Rust 跑完整的 `pnpm run test:web` 是[重写笔记](../architecture/2026-08-14-rust-rewrite.md)中的重写计划退出条件，不是第 7 阶段的切换点。

当 `DSH_RUNTIME=rust` 时，那些具名 web 驱动 spawn `target/debug/dsh`（若设置了 `DSH_RUNTIME_BIN` 则用该路径），argv 为 `web --port 0 --dist <dir>`。未设置 `DSH_RUNTIME` 时保持进程内 Cordis scaffold。`built-boot.snapshot.ts` 仍为 jsdom/`FixtureApiClient`（无宿主）。

第 8 阶段第 1 项在 Rust `dsh` 二进制上的具名 ACP（Agent Client Protocol）场景是 `handshake`、`reject-extra-dirs` 与 `text-turn`。其余 `examples/acp-agent` 场景留在 Node `dsh-acp-demo` 二进制上。对着 Rust 跑完整的 `pnpm run test:snapshot` 不是第 8 阶段第 1 项的退出条件。

当 `DSH_RUNTIME=rust` 时，那些具名 ACP 驱动 spawn `target/debug/dsh`（若设置了 `DSH_RUNTIME_BIN` 则用该路径），并带上 `--profile acp`。未设置 `DSH_RUNTIME` 时保持 Node `dsh-acp-demo` 二进制。

## 第 6 阶段子集

| 场景 | 驱动 | 二进制 | Fixture 目录 |
|---|---|---|---|
| jsonrpc `text-turn` | Vitest `sdk.snapshot.ts` | `DSH_RUNTIME=rust` 时为 Rust；否则为 Node | `examples/jsonrpc-agent/tests/snapshots/text-turn/` |
| jsonrpc `bash-tool` | Vitest `sdk.snapshot.ts` | `DSH_RUNTIME=rust` 时为 Rust；否则为 Node | `examples/jsonrpc-agent/tests/snapshots/bash-tool/` |
| jsonrpc `subagent-spawn-in-process` | Vitest `sdk.snapshot.ts` | `DSH_RUNTIME=rust` 时为 Rust；否则为 Node | `examples/jsonrpc-agent/tests/snapshots/subagent-spawn-in-process/` |
| jsonrpc `persistent-tools` | Vitest `sdk.snapshot.ts` | 仅 Node | `examples/jsonrpc-agent/tests/snapshots/persistent-tools/` |
| headless `compaction-recovery` | Vitest `headless.snapshot.ts` | `DSH_RUNTIME=rust` 时为 Rust；否则为 Node | `examples/headless-agent/tests/snapshots/compaction-recovery/` |
| headless `provider-retry` | Vitest `headless.snapshot.ts` | `DSH_RUNTIME=rust` 时为 Rust；否则为 Node | `examples/headless-agent/tests/snapshots/provider-retry/` |
| headless agent-instructions 恢复 | Vitest `workspace-context-resume.snapshot.ts` | `DSH_RUNTIME=rust` 时为 Rust；否则为 Node | `examples/headless-agent/tests/workspace-context-resume-snapshots/` |
| headless `subagent-settlement` | Vitest `headless.snapshot.ts` | `DSH_RUNTIME=rust` 时为 Rust；否则为 Node | `examples/headless-agent/tests/snapshots/subagent-settlement/` |
| headless pty/ralph/goal/advanced/headless-profile | 现有 Vitest | 仅 Node | 现有目录 |

## 第 7 阶段子集

| 场景 | 驱动 | 二进制 | Fixture 目录 |
|---|---|---|---|
| web `rust-host-smoke` | Vitest `apps/web/tests/rust-host-smoke.e2e.ts` | `DSH_RUNTIME=rust` 时为 Rust；否则跳过 | 无 |
| web `cold-blank-session` | Vitest `cold-blank-session.e2e.ts` | `DSH_RUNTIME=rust` 时为 Rust；否则为 Node scaffold | `apps/web/tests/snapshots/cold-blank-session/` |
| 其余 `test:web` 文件 | 现有 Vitest | Node scaffold / jsdom | 现有目录 |

## 第 8 阶段 ACP 子集

| 场景 | 驱动 | 二进制 | Fixture 目录 |
|---|---|---|---|
| ACP `handshake` | Vitest `examples/acp-agent/tests/acp.snapshot.ts` | `DSH_RUNTIME=rust` 时为 Rust；否则为 Node | `examples/acp-agent/tests/snapshots/handshake/` |
| ACP `reject-extra-dirs` | 同上 | `DSH_RUNTIME=rust` 时为 Rust；否则为 Node | `examples/acp-agent/tests/snapshots/reject-extra-dirs/` |
| ACP `text-turn` | 同上 | `DSH_RUNTIME=rust` 时为 Rust；否则为 Node | `examples/acp-agent/tests/snapshots/text-turn/` |
| 其余 ACP 场景 | 同一套件 | 仅 Node | 现有目录 |

`DSH_RUNTIME=rust` 不得从表中丢掉场景（orphan-dir 守卫），并且必须跳过非子集的 **运行**。本笔记不声称在 Rust 上跑完整的 `pnpm run test:snapshot`。

## 曾考虑的替代方案

**把快照驱动改写成 Rust（`cargo test` 不生成进程，或 Rust NDJSON 客户端）。** 否决：产品测试是组装后的应用转录；针对不同组合的第二套测试正是重写笔记所点名的双跑失败。Vitest 已经拥有归一化、`llm-replay` 灌入与期望输出比较。

**针对第 5 阶段最小二进制重录每份 jsonrpc 期望 JSONL。** 否决，除非方法名、`serverInfo.name` 或冻结线路上的 `session.event` 封套字段是错的。组合更瘦的日志是第 5 阶段可接受缺口，不是 fixture 重写。

**丢掉 Vitest，把 crate 测试当作快照门。** 被工具链笔记否决：一旦存在二进制，声称检验交付产品的快照必须跑该二进制。

**在第 5 阶段为每一个现有 headless 与 jsonrpc 场景生成 Rust 二进制。** 对 pty/ralph/goal/advanced 仍否决：那些场景需要第 8 阶段能力。第 6 阶段将四个 headless 场景外加 jsonrpc `subagent-spawn-in-process` 定为切换范围。其余场景仍由 Node 驱动。

**在第 7 阶段为每一个 `test:web` 文件 spawn Rust 二进制。** 否决：具名子集是 `rust-host-smoke` 与 `cold-blank-session`。其余文件留在 Node。对着 Rust 跑完整的 `pnpm run test:web` 是重写计划的退出条件。

**在第 8 阶段第 1 项为每一个 `examples/acp-agent` 场景 spawn Rust 二进制。** 否决：具名子集是 `handshake`、`reject-extra-dirs` 与 `text-turn`。其余 ACP 场景留在 Node。对着 Rust 跑完整的 `pnpm run test:snapshot` 不是第 8 阶段第 1 项的退出条件。

## 验收标准

- 重写笔记的后续表链接到本文件，而不是占位符 ``proposed/testing/…-rust-snapshot-harness.md``。
- `DSH_RUNTIME=rust` 被记录为 Vitest 启动开关；未设置时保持 Node。
- 第 6 阶段将四个 headless 场景与 jsonrpc `subagent-spawn-in-process` 命名为 Rust 子集。
- Fixture 目录被复用；计划不增加并行的 `*.rust.expected.jsonl` 文件。
- 第 7 阶段将 web `rust-host-smoke` 与 `cold-blank-session` 命名为 Rust 子集；其余 `test:web` 文件留在 Node。本笔记不声称在 Rust 上跑完整的 `pnpm run test:web`。
- 第 8 阶段第 1 项将 ACP `handshake`、`reject-extra-dirs` 与 `text-turn` 命名为 Rust 子集；其余 ACP 场景留在 Node。本笔记不声称在 Rust 上跑完整的 `pnpm run test:snapshot`。
- 不编辑 `docs/architecture.md`。

## 风险

评审者可能把 Rust 路径上跳过 Node 的 `stream-json.expected.jsonl` 与通知 JSONL 全量相等视为削弱快照门。Node 默认仍钉住完整转录；Rust 路径钉住场景特定的持久化事实、进程退出码 0、最后一段 assistant / stdout，以及 jsonrpc 的 `finalResponse`、idle 状态、持久化路径和 `serverInfo.name`。第 6 阶段组合并不匹配 Node 事件流。

评审者可能把具名 web 子集当作在 Rust 上跑完整的 `pnpm run test:web`。其余 web e2e 留在 Node。

评审者可能把具名 ACP 子集当作在 Rust 上跑完整的 `pnpm run test:snapshot`。其余 ACP 场景留在 Node。
