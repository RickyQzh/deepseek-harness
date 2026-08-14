# Agent Note: 以 Rust 重写核心与后端，Web UI 保留 TypeScript

Status: proposed

[English](2026-08-14-rust-rewrite.md) | 中文

## 问题

DeepSeek Harness 是由插件组合而成的 coding agent（编程智能体）运行时：一次运行是一棵有序的插件树，各插件向共享的 Cordis `Context` 贡献服务、带类型的事件和可撤销的副作用。没有可供打补丁的特权核心。产品今天是 **219** 个 `@deepseek-ai/dsh-*` 包（约 1,316 个 `src` 文件、约 227k LOC），外加 vendored Cordis、一个 C Landlock 启动器、一个 Python JSON-RPC SDK、一个 React Web UI，以及一个 VitePress 文档站。每一个 harness 包都是 Cordis 插件。因此换语言是一次 **插件运行时加产品** 的移植，而不是库重写。

宿主是 Node。源码启动是 `node --import tsx/esm`。工作流与 Code Mode 引擎是 `worker_threads` 加 `node:vm`。动态自修改把 JavaScript 当作活的 Cordis 插件来求值。配置 YAML 用对着插件上下文的 `eval` 插值 `!!js`。这些机制没有 Rust 等价物，必须替换，不能逐字转录。

**没有**既有 Agent Note 提出宿主语言迁移。原生 Landlock 是 C11（约 300 行，静态 musl），因为在更早的 Rust 启动器被替换之后，Rust/landstrip 审计面已被[否决](../../rejected/feature/2026-07-26-evaluate-landstrip-for-windows-sandbox-rung.md)。一次默默把该二进制再写成 Rust 的重写，会重新打开一条已记录的安全不变量。

仓库处于预发布：后端拒绝旧的磁盘格式；`SESSION_FORMAT_VERSION` 保持 `0`，没有兼容承诺。外部客户端仍然存在：PyPI `deepseek-harness-sdk`、ACP stdio、Claude Code/Codex `hooks.json`，以及已交付的 TypeScript 浏览器。真正的兼容压力是这些线路，而不是包组上的 “stable API” 标签。

本笔记是把 **核心与后端改写成 Rust**、同时允许 Web 前端保留 TypeScript 的唯一架构提案。它不是实现 PR。各阶段的任务级计划在该阶段开始时再写；本文锁定保留/放弃、crate 边界、线路冻结和切换顺序，使后续工作流不会发明第二个产品。

## 提案

交付 **一个 Rust 宿主进程**，由它拥有 agent loop（智能体循环）、会话日志、工具、LLM（大语言模型）HTTP、文件系统/subprocess/沙箱、持久化、CLI（命令行界面）、ACP（Agent Client Protocol）和 SDK JSON-RPC。`packages/client/*`、`apps/web` 和 `website/` 保留 TypeScript。浏览器继续说现有的四象限 HTTP + WebSocket 协议。Python 继续通过 NDJSON JSON-RPC 对运行时二进制做 `Popen`；改变的只是被拉起的 exe。

保留 Cordis **语义**（具名服务、inject-wait、逆序 dispose（资源释放）、waterfall（瀑布式事件）`next()`、isolate realm、按 id 整份替换配置的补丁）。丢掉 Cordis **实现**：Proxy、TypeScript 声明合并、ESM ModuleLoader，以及 `!!js` `eval`。v1 插件是写在 profile manifest（元数据清单）里的受信任同进程 Rust crate，不是热加载的 JavaScript。

保留使本 harness 成其为自身的产品不变量：仅追加会话日志、对模型可见 ⟺ 已记录、能力 seam 为 Definition / Provider / Consumer、沙箱是与宿主共享文件系统和内核的 argv 包装（不是虚拟机），以及通过真实组装示例的无密钥快照。实现今天的会话格式（`SESSION_FORMAT_VERSION = 0`、JSONL zstd 默认、SQLite `SCHEMA_VERSION = 15`），使现有 fixture（测试前置数据）可以回放。不要在重写中发明一份并行日志。

首个交付的 Rust 产品是 **headless + SDK JSON-RPC**（Python 与 Rust 二进制对话）。第二个是服务现有 TypeScript SPA 的 Rust GUI 宿主。可选能力（ACP、MCP、terminal、LSP、工作流、Windows ACL）随后。E2B、pi-ai、作为插件的 Cordis HMR（热模块替换）、作为 TypeScript 分析器的 Typert，以及 `node:vm` 自修改，均不在 v1。

## 范围与非目标

**重写计划范围内**

- 在宿主进程上替换 vendored Cordis 的内核（`dsh-kernel`、compose、events、schema）。
- 产品主干：scope、session、system-prompt、tools、agent、agent-loop、LLM DeepSeek 适配器、JSONL 持久化、credentials、settings、identity。
- 本地执行世界：`ctx.fs` + `ctx.subprocess` 作为同一个世界；bash（POSIX）/ pwsh（win32）；沙箱包装宿主 `bwrap` / C `landlock-run` / `sandbox-exec` / Rust Windows ACL；fs-sandbox 围栏。
- 交互：approval、permission preset、commands、ask-user。
- 压缩（compaction）、token meter、agent-instructions、skills 文件系统、DeepSeek web search、jobs、同进程 subagent spawn/fork。
- `dsh` CLI（clap）、headless runner、SDK JSON-RPC 服务器，以及随后的 axum GUI 宿主。
- 消费现有 `session.jsonl` fixture 的快照 harness 移植。
- 指向 Rust exe 的 Python runtime-bin。

**保留 TypeScript**

- 全部 `packages/client/*` 与 `apps/web`（React、slot、conversation node）。
- `website/` VitePress 投影器。
- `dsh-typert-generator`，直到 IDL 替换它；该生成器是 TypeScript `ts.Program` 遍历器。
- 双面包的浏览器半边（`./client` 导出）。Rust 宿主仍必须发出 `__DSH_BOOT__` 并提供 `/plugins/<id>/client.js`。

**保留 C / 宿主 CLI**

- `native/landlock-run` C11 musl 助手。Rust 宿主 **spawn** 它。不要对 agent 进程做 Landlock。没有重新打开 landstrip 否决的新 Agent Note，不要把 `main.c` 改写成 Rust。
- 宿主 `bwrap` 与 `sandbox-exec`。

**保留 Python**

- `python/sdk`（`deepseek_harness`）。它从不导入宿主语言。

**v1 范围外（产品包可留在 TypeScript 树中直到删除）**

- E2B POC（`packages/e2b/`）。
- `dsh-llm-pi-ai` 设计孪生。
- `dsh-tool-cordis` / `dsh-cordis-host-runner`（`node:vm` 活插件挂载）。
- Cordis 模块 HMR（`cordis-plugin-hmr`）；用户补丁文件监视可以稍后作为简单监视器回来。
- Typert 类型图提取器。用面向 GUI + SDK 方法的小型 IDL 替换。
- YAML 中任意 JavaScript 的 `!!js`。
- Windows PTY（产品今天没有）。
- Electron / Tauri / VS Code 扩展（已预留，未交付）。

## 推荐架构

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

三条线路保持彼此独立。不要让 Python 指向 GUI HTTP API。不要让浏览器说 SDK JSON-RPC。

浏览器 Loader 独立于宿主插件运行时。Rust 必须扫描 manifest（或生成的名册）、哈希 bundle、提供它们，并注入启动图。它不得在宿主里运行 Cordis。

## 保留或放弃清单

每一行都是已记录的决策。“保留”表示保住 **可观察约定**。“替换”表示保住约定并改实现。“放弃”表示 v1 不交付它。

### 产品身份

| 不变量 | 立场 | 依据 |
|---|---|---|
| 一切皆插件；没有特权核心 | **替换**：同进程 Rust crate + profile manifest | [architecture.md](../../../../docs/architecture.md)；[微内核事件分类](../../implemented/architecture/2026-06-11-microkernel-event-taxonomy.md) |
| 注册都是 effect；`register()` 返回 disposer | **保留** | 根 `AGENTS.md` |
| Profile + bundle；空根；`dsh-base` 最先 | **保留** 层序；补丁文件变成封闭 YAML 方言 | [profile 插件 bundle](../../implemented/architecture/2026-08-05-profile-plugin-bundles.md) |
| 新行为走扩展点，不改 loop | **保留** | 根 `AGENTS.md` |
| Waterfall 监听器必须调用 `next()` | **保留** | [cordis-primer.md](../../../../docs/cordis-primer.md) |
| 可合并扩展的 `SessionEventMap` | **替换** 为封闭的第一方 serde enum + `ignorable` 剩余项 | [带类型的事件 schema](2026-06-16-typed-event-schemas.md)；[会话日志版本](../../implemented/architecture/2026-08-10-session-log-version-mechanism.md) |
| 客户端插件 = 浏览器里的 Cordis DI | **保留**（TS） | [客户端插件加载](../../implemented/architecture/2026-07-23-client-plugin-loading-model.md) |

### 会话 / 持久化

| 不变量 | 立场 | 依据 |
|---|---|---|
| 会话是仅追加的带类型事件日志；`deriveMessages()` 就是历史 | **保留** | [事件溯源会话](../../implemented/architecture/2026-06-11-event-sourced-sessions.md) |
| 对模型可见 ⟺ 已记录 | **保留** | [可重建请求](../../implemented/architecture/2026-07-05-reconstructable-requests.md) |
| 规范日志无损且连续，包括 `assistant/chunk` | **保留** | [会话持久化](../../implemented/architecture/2026-06-14-session-persistence.md) |
| 已刷新事件永不改写；崩溃轮次被关闭，从不截断 | **保留** | 同上 |
| Header 在日志之外（`SessionHeader`） | **保留** | 同上 |
| Surface（`append` / `replace`）是唯一的历史操作机制 | **保留** | [会话 surface](../../implemented/architecture/2026-06-18-session-surface.md) |
| `SESSION_FORMAT_VERSION = 0`；未知的必读事件拒绝，除非 `ignorable: true` | **保留**（实现今天的读取器） | [会话日志版本](../../implemented/architecture/2026-08-10-session-log-version-mechanism.md) |
| SQLite `SCHEMA_VERSION` 单调；非当前 `user_version` 拒绝 | **保留** 在 **15** / `application_id = 0x44534850` | 持久化笔记 |

不要在重写中从 v1 日志起步。不要为 v0 增加迁移系统。移植两个同版本导入例外（身份化之前的消息；react-loop 之前）或对这些形状大声失败——不要发明第三层兼容。

### 能力 seam 与执行

| 不变量 | 立场 | 依据 |
|---|---|---|
| 一个 seam 是 Definition + Provider + Consumer | **保留** 为 crate 角色 | [能力 seam](../../implemented/architecture/2026-06-13-capability-seams.md) |
| `ctx.fs` + `ctx.subprocess` 是同一个执行世界 | **保留** | [可移植执行世界](../../implemented/architecture/2026-07-28-portable-execution-world-consumers.md) |
| `ctx.sandbox.confine` 返回包装后的 argv；容器/远程替换世界对 | **保留** | [沙箱](../../implemented/feature/2026-07-06-sandbox.md) |
| Landlock 是独立的 restrict-then-exec 助手 | **保留 C 二进制** | [landstrip 否决](../../rejected/feature/2026-07-26-evaluate-landstrip-for-windows-sandbox-rung.md)；[native/README.md](../../../../native/README.md) |
| 显式 `resolve(request): Spec` | **保留** | 根 `AGENTS.md`（`dsh-shell` 模板） |
| 不透明 branded id | **保留** 为 Rust newtype | 根 `AGENTS.md` |
| 插件里没有硬编码可调项 | **保留** | 根 `AGENTS.md` |
| 信任同进程带类型的值；在 parser/config/queued/model JSON/durable/file/worker/process/wire 校验 | **保留**（改写：同进程信任 Rust 类型） | 根 `AGENTS.md` |
| 源码启动 = `node --import tsx/esm` | **放弃**（宿主不是 Node） | [源码启动 tsx ESM](../../implemented/architecture/2026-07-29-dsh-source-launch-tsx-esm.md) |
| Vendored Cordis 作为框架 | **替换** 实现；保留所列语义 | [vendor/README.md](../../../../vendor/README.md) |
| 通过真实可运行示例的无密钥快照 | **保留** | [testing.md](../../../../docs/testing.md) |

### 不得回退的安全不变量

1. 永不静默解除限制。没有可用 runner → `SANDBOX_UNAVAILABLE`；命令不运行。
2. Landlock/bwrap/Seatbelt：内核无法强制时不要 exec。Landlock 退出码 125 + `landlock-run: ` 致命行。没有用环境变量覆盖由哪个二进制施加限制。
3. `danger-full-access` 是唯一未限制路径，并且是显式的。
4. 升级：成对的 `sandbox_permissions` + 非空 `justification`；严格更宽；execute 之前走 `ctx.approval`；授权仅该次调用。
5. 模式只声称文件效果。不要宣传网络或 PID 隔离。
6. Seatbelt 与 `fs-sandbox` 共用一份 `writableRoots()`。
7. Windows ACL 报告 `partial`。
8. Subprocess `argv` 永远不是 shell 字符串。
9. 子进程环境之前做凭证擦除。
10. `rg` spawn 首先使用 `--no-config`。
11. FS 围栏是包含性检查，不是内核边界。`FS_SANDBOX_DENIED` ≠ `FS_PERMISSION_DENIED`。
12. 观察策略：未经事先读取的 edit → `FS_NOT_OBSERVED`。
13. LSP：无 `workspace/applyEdit`；无协议逃生舱；查询源在工作区内；`processId: null`。
14. PTY：精确 Agent 鉴权；拒绝 SIGKILL shell；取消是真正的 SIGINT；PID+starttime 围栏。
15. 树终止等待树完全停稳。宿主退出时的同步 kill 不得声称完全停稳。
16. Permission preset 在创建时按会话钉死。

## Crate 拓扑

v1 是紧挨 `packages/` 的 Cargo workspace，不是把全部 219 个名字按 TS 包一对一倒成 crate。一起改的文件放在一起。当 Definition / Provider / Consumer 这些角色已经独立演化时，它们仍分成独立 crate。

### 内核（在宿主上替换 Cordis）

| Crate | 职责 |
|---|---|
| `dsh-brand` | Newtypes: `SessionId`, `MessageId`, `CallId`, … |
| `dsh-kernel` | `Context`, `Fiber` state machine, service registry, effect/dispose, isolate, intercept, inject-wait |
| `dsh-events` | `emit` / `waterfall` / `parallel` / `serial` + scope filter |
| `dsh-compose` | Entry list, id-targeted patches, insert, layer order, dump-config |
| `dsh-schema` | Plugin config validation + JSON-serializable schema for Settings UI |

作为 crate 丢掉：cosmokit、Cordis HMR 插件、logger-console、Node ModuleLoader。日志用 `tracing`。

### 主干（一并移植；这是最小 agent）

`dsh-scope`、`dsh-session`、`dsh-system-prompt`、`dsh-tools`、`dsh-agent`、`dsh-agent-loop`、`dsh-llm`、`dsh-llm-deepseek`、`dsh-llm-retry`、`dsh-session-persist`（seam + JSONL；SQLite 稍后）、`dsh-credentials`、`dsh-settings`、`dsh-identity`。

### 执行世界（作为整体替换）

`dsh-subprocess` + OS 提供方、`dsh-fs` + local + 沙箱围栏、`dsh-sandbox` + 现有助手的本地包装层、`dsh-shell` + bash/pwsh、`dsh-tool-fs`、`dsh-tool-bash` / `dsh-tool-pwsh`。Terminal 与 LSP 是同一世界上稍后的可选 crate。

### 首批产品入口

`dsh-boot`、`dsh-cli`、`dsh-headless`、`dsh-sdk-protocol`、`dsh-sdk-jsonrpc-server`。稍后：`dsh-host`（axum）、`dsh-acp`。

### Trait 草图（仅名称）

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

Waterfall 是 **必须** 调用 `next()` 的监听链。不调用 `next()` 就返回会短路，与 [cordis-primer.md](../../../../docs/cordis-primer.md) 一致。

## 插件与组合模型

### v1 插件 ABI

受信任的同进程 crate。profile 列出由 crate 支撑的行 id。v1 没有稳定 C ABI，也没有 cdylib 热加载。第三方插件若稍后存在，是 workspace crate 或更晚的 wasm component 故事——不是 `node:vm`。

`dsh-tool-cordis`（inspect/define/run 活的 JavaScript 插件）**不在 v1**。重新引入动态插件需要它自己的 Agent Note 和真正的隔离故事（wasmtime 或被 landlock 的进程）。今天的 `node:vm` 不是安全边界。

### YAML 方言（打破 `!!js`）

不要移植 `eval` / `with(ctx)`。产品 `!!js` 用途收成三种行为：

1. **封闭插值器：** `${env:VAR:-default}`, `${cwd}`, `${dshHome:sessions}`, `${platform}`。
2. **`disabled` 谓词或平台 overlay 文件**，用于 bash 与 pwsh 行。
3. **插件在代码里读取注入的服务。** `dsh-host-webserver` 读 `ctx.webStartup`；headless 读 `ctx.headlessStartup.task`。YAML 保持静态。

保留按 id 的 **整份配置替换** 和 `insert`（没有深合并）、与今天相同的层序，以及失败即响的结算。`dsh --dump-config` 打印组合后的树，插值器不求值。

用户 `cordis.patch.yml` 的 HMR 可以作为文件监视器回来：重新组合各层，解析失败时保住上一份好树。模块 HMR 不是 v1。

### Preset

保留常驻挂载、`agent → preset → global` 遮蔽、preset 自有服务的 isolate（`planMode`、`workflowEngine`、`compaction`），以及 preset 行向根 realm provide 即为加载失败的规则。在没有 Proxy 的情况下实现 isolate：服务存储上的显式 realm 键。

## 持久化与协议格式

### 会话日志

实现今天的 v0 JSONL（默认 `$DSH_HOME/sessions/--<project>--/<id>/session.jsonl.zstd`、packed chunk 行、头行 `type: "session"`）以及可选 SQLite（`SCHEMA_VERSION = 15`）。对 `packages/core/session/src/known-event-types.ts` 中生成的第一方类型（调查时 44 种）使用封闭 `#[serde(tag = "type")]` enum，外加仅在 `ignorable: true` 时接受的未知剩余项。

在 TypeScript 包删除之前，从发出 `KNOWN_SESSION_EVENT_TYPES` 的同一目录生成器生成 Rust enum，然后把目录所有权移到 Rust。不要让已知集合依赖组合（[会话日志版本](../../implemented/architecture/2026-08-10-session-log-version-mechanism.md) 否决了面向第一方读取器的运行时注册表）。

SQLite 后端用 `rusqlite`（同步、pragma 密集、无迁移器）。产品默认仍是 JSONL。

Settings YAML、credentials YAML 和 `.anonymous-user-id` 没有版本字段。按原样实现。用户已经有这些文件。

### 三条线路

| 协议 | 消费方 | v1 冻结 |
|---|---|---|
| SDK JSON-RPC（`initialize`、`session/prompt`、`shutdown`；通知 `session.event`、`session.status`、`subagent.started`、`subagent.finished`） | Python、TS SDK、`subagent-dsh-sdk` | **字节兼容** 方法名、`serverInfo.name = deepseek-harness-sdk-runtime`、完整 `SessionEvent` 信封 |
| ACP 子集（`initialize`、`authenticate`、`session/new`、`session/prompt`、`session/cancel`；`session/update` agent_message_chunk；`session/request_permission`） | ACP 客户端、`subagent-acp` | **字节兼容** 已声明能力与 permission 选项 id |
| GUI 四象限 HTTP + WS（`RpcMethodMap` + Typert 斜杠 Remote + mux/host 帧） | TypeScript 浏览器 | **若 TS UI 留下则冻结**；今天没有 `protocolVersion`，因为客户端与宿主一起交付 |

若这些桥交付，已映射的 Claude Code 与 Codex 事件的钩子 stdin/stdout/退出码方言保持兼容。

若 MCP 交付，MCP 工具名保持 `mcp__<serverName>__<rawName>`。

作为分析器的 Typert 不是持久化格式。Rust GUI 宿主要么消费一份冻结的描述符转储，要么用小型 IDL 生成 Rust 类型 + TS 客户端存根。同进程 Rust 插件不需要 Typert。

## 测试与双轨运行

保留 [testing.md](../../../../docs/testing.md) 中的产品测试策略：

- 针对竞态、错误路径、dispose、约定回归的 crate 单元测试。
- 对自有 Rust `src` 按文件（按模块）100%，沿用同一套豁免纪律。
- **通过真实组装示例的无密钥快照** 才是产品测试。复用已提交的 fixture。移植 `dsh-llm-replay` 与规范化器。CI：仅回放、已构建二进制、不加载 `.env`。
- 带密钥 e2e 在没有密钥时自行跳过；CI 预检该密钥。
- 对已发布 bin 做产物冒烟，而不是只跑源码 `cargo test`。

TypeScript 包仍在交付时，其 Vitest + 覆盖率仍为必需。快照与 e2e 对着 **一份** 组装产品跑——用户运行的那个 bin。当交付 bin 变成 Rust 时，移动快照 **驱动** 并保留同一套 fixture 目录。不要把两套快照套件对着两套组合当作合并门禁。

一旦任何 crate 进入合并路径，就增加一条必需的 Rust CI 车道（fmt、clippy、test、llvm-cov）。现有 Node 24 车道保留到该车道里最后一个 TypeScript 包消失。

## 实施计划

> **面向 agentic worker：** 在该阶段的详细任务计划存在之后，每个任务用一个新的 subagent 实现。本节是程序顺序，不是 2–5 分钟任务清单。覆盖多个独立子系统的阶段在写代码之前仍要有自己的 Agent Note 加一份可咬一口大小的计划。

**全局约束（每一阶段都继承这些）**

- 新的核心/后端代码的宿主语言是 Rust（edition 2024，或 CI 钉住所点名的当时现行 edition）。Web UI 保留 TypeScript。
- 不要把 `native/landlock-run` 改写成 Rust。
- 不要移植 `!!js` eval。
- 会话读取器实现格式 `0` 与 SQLite schema `15`。
- 在声称“兼容”的阶段里，SDK JSON-RPC 与 ACP 方法名不变。
- 快照保持对现有 JSONL fixture 的无密钥回放。
- Copyleft 许可证不得进入已交付闭包（现有 third-party-notices 门禁）。
- Leader agent 不在本地探索或改写；它们分派 subagent。

### 阶段 0 — 工作区与 CI

**目标：** CI 会跑的 Cargo workspace，尚无产品行为。

- [ ] 增加 `crates/` 以及 workspace `Cargo.toml`、`rust-toolchain.toml`、`cargo fmt`/`clippy` 配置。
- [ ] 增加一条 GitHub Actions Rust 车道（fmt、clippy、test），在任何 `crates/**` 路径变化时为必需。
- [ ] 为 rustc 记录源码平面与产物平面的划分（替换宿主代码的 tsx/lib 规则）。
- [ ] 退出：空的 `dsh-brand` crate 测试在 CI 上绿；Node 车道不变。

### 阶段 1 — 内核

**目标：** 没有 Cordis 的 Cordis 语义。

- [ ] `dsh-kernel`：Fiber 状态 PENDING → LOADING → ACTIVE | FAILED | UNLOADING → DISPOSED；逆序异步 dispose；UNLOADING 时没有新 effect；失败的 setup 回滚已收集的 cleanup。
- [ ] `dsh-events`：emit（被包含）、serial、parallel、带 `next()` 的 waterfall。
- [ ] `dsh-compose`：按 id 整份配置替换、insert、层序；封闭插值器；`disabled` 谓词；**没有** `!!js`。
- [ ] `dsh-schema`：serde + 供 Settings UI 使用的 JSON 可序列化 schema。
- [ ] 足够支撑后续 preset 的 isolate realm。
- [ ] 退出：内核单元测试覆盖 inject-wait、dispose 顺序、waterfall 短路、同一列表中 insert 然后 patch、缺失指称失败即响。

### 阶段 2 — 会话 + 主干类型

**目标：** 仅追加日志、surface、derive、修复、inbox 类型。

- [ ] 从持久化目录生成的封闭 `SessionEvent` enum。
- [ ] 包括 zstd 帧与 packed chunk 行的 JSONL 编解码。
- [ ] `deriveMessages`、surface replace、header fold、中断轮次修复。
- [ ] 从 `session-format-guard.snapshot.ts` / coordinator-contract 移植的格式拒绝测试。
- [ ] 退出：把 `examples/headless-agent/tests/snapshots/headless-profile/session.expected.jsonl`（规范化后）经 Rust 编解码往返。

### 阶段 3 — agent loop + 工具 + 提示词 + DeepSeek LLM

**目标：** 能从日志重建请求的脚本化循环。

- [ ] 工具流水线：冻结 args → pre-execute → approval → guards → execute → post-execute → finalize；Code Mode 在策略之前折叠。
- [ ] 系统提示词组装 + 严格 `{{var}}` 插值 + 运行时上下文快照身份。
- [ ] Loop 阶段机：idle / maintenance / running；轮次在首次 claim 之前打开；空的首次 claim 仍记录一轮；粘性 `max-tokens`；abort 排空 + `ABORTED_BEFORE_DISPATCH`。
- [ ] `dsh-llm` 流约定：usage 在 finish 之前；适配器失败是终端 `finish`，不是抛出；按读的空闲看门狗。
- [ ] `dsh-llm-deepseek`：`POST /chat/completions` SSE、thinking/effort 映射、`""` 不是 `null` content、身份头、compact 头。
- [ ] 凭证按请求解析；永不把密钥存在适配器上。
- [ ] 退出：已移植的 loop/cancel/reconstruction/tool-calls 测试，外加对着 mock 服务器的 DeepSeek serialize/SSE/translate 测试。

### 阶段 4 — 本地执行世界

**目标：** POSIX 上带失败即关沙箱的 bash/fs 工具。

- [ ] 带 POSIX 组的 `tokio` 进程树；Windows `taskkill /T` 等价物在本阶段稍后或阶段 8。
- [ ] `confine()` 包装 argv；spawn 现有的 `landlock-run` / `bwrap` / `sandbox-exec`。
- [ ] fs-local + fs-sandbox + 观察策略；`rg --no-config`。
- [ ] Shell `resolve` 然后 `run`/`start`。
- [ ] 退出：沙箱不可用失败即关；Landlock 125+致命行归类为启动器失败；观察 `FS_NOT_OBSERVED`；搜索 `--no-config` 不变量测试。

### 阶段 5 — Headless + SDK JSON-RPC（首个交付的 Rust 产品）

**目标：** Python SDK 与 `dsh --profile headless` 能在 Rust bin 上运行。

- [ ] `dsh` clap 启动器：`--profile`、`--patch`、`web` 别名，`plugin` 推迟或仍为 TS。
- [ ] Headless：一个位置参数任务；打印最后一段 assistant 文本；当且仅当最终 `turn/end` 为 `completed` 时退出 0。
- [ ] SDK 服务器：NDJSON JSON-RPC 2.0；方法与通知如上所列。
- [ ] `python/sdk-runtime` 指向 Rust exe；保留 `DSH_CORDIS_CONFIG` 或一份有文档的 Rust 组合文件。
- [ ] 把 `examples/jsonrpc-agent` 与 `examples/headless-agent` 快照 **驱动** 移植到 Rust bin；保留 fixture 目录。
- [ ] 退出：无密钥 JSON-RPC 与 headless 快照套件在 Rust bin 上绿；Python 无密钥 SDK 测试对着 Rust exe 绿。

### 阶段 6 — 交互、压缩、上下文、skill、web search、jobs、同进程 subagent

**目标：** Rust headless 上的 `dsh-base` / `standard` preset 行为。

- [ ] Approval waterfall 失败即关；permission preset 在会话创建时钉死。
- [ ] 压缩的 surface replace + 仅日志锁；token-meter 启发式对等；仅当 `replaceGeneration` 前进时才 overflow retry。
- [ ] agent-instructions、可选 time-context；skill 目录 + `skill` 工具。
- [ ] DeepSeek `web_search`；`web_fetch` 保持关闭（SSRF）。
- [ ] 同进程 jobs；同进程 subagent spawn/fork + continuation manager。
- [ ] 退出：compaction-recovery、provider-retry、agent-instructions 恢复以及一个 subagent 场景的 headless 快照绿。

### 阶段 7 — Rust GUI 宿主（TypeScript UI 不变）

**目标：** `dsh web` 可以是服务 `apps/web` dist 的 Rust 宿主。

- [ ] axum：`POST /api/<method>`、`POST /api/respond`、WS `/api/events.mux` 与 `/api/events.host`、`GET /api/session.export`、静态 SPA、`/plugins/<id>/client.js`、`__DSH_BOOT__`。
- [ ] 实现已交付名册使用的 `RpcMethodMap` 加 Typert 斜杠 Remote（或一份冻结的描述符转储）。
- [ ] settings/credentials/preset 编写的 loopback 特权围栏。
- [ ] Agent/Session 查找与今天的 `agentFor()` 等价（活实例复用、冷恢复 + preset、并发去重、拒绝 subagent 拥有的 id）。
- [ ] 退出：对着 Linux 上的 Rust 宿主跑 `pnpm run test:web`（回放）。

### 阶段 8 — 可选能力

每一项是自己的后续笔记 + 计划。顺序没有负载。

- [ ] ACP 服务器（字节兼容子集）。
- [ ] MCP 客户端（`mcp__` 名称）。
- [ ] Terminal + PTY（仅 POSIX）经由 `portable-pty`；保留 Linux stdin-wait `/proc` 行为，或记录对模型可见的变化。
- [ ] LSP stdio 宿主（短暂打开，无 `applyEdit`）。
- [ ] 替换 `worker_threads` 的工作流引擎（敌对对等 JSON 协议）。
- [ ] 替换 `worker_threads` 的 Code Mode 运行时。
- [ ] 经由 `windows` crate 的 Windows ACL（报告 `partial`）。
- [ ] SQLite 会话后端与 session-query FTS。
- [ ] 进程外 subagent（ACP / dsh-sdk / Codex / Claude Code）。
- [ ] Claude Code / Codex 钩子桥。

### 明确不是一个阶段

- 把 `packages/client` 改写成 Rust。
- 改写 VitePress。
- 把 `landlock-run` 改写成 Rust。
- 移植 E2B 适配器。
- 经由带完整 `ctx` 的 QuickJS/Rhai 移植 `!!js`。
- napi-rs 同进程 Landlock。
- 让已知会话事件类型依赖链接了哪些 crate。

## 后续笔记（该决策独立时再写）

**不要**从 [docs/architecture.md](../../../../docs/architecture.md) 或 Superpowers `docs/superpowers/` 树起步。某一阶段交付之后，只把 `docs/architecture.md` 更新为简短的当前状态图。

| 笔记 | 时机 |
|---|---|
| `proposed/process/…-rust-tooling-and-gates.md` | 阶段 0，若 CI/工具链细节超出本文件 |
| `proposed/testing/…-rust-snapshot-harness.md` | 阶段 5，若快照驱动需要自己的决策记录 |
| `proposed/architecture/…-rust-gui-host-wire.md` | 阶段 7，若四象限映射需要冻结的 IDL |
| `proposed/architecture/…-dynamic-plugins.md` | 仅当重新引入自修改时 |

本笔记在 Rust 宿主上部分回答了[带类型的事件 schema](2026-06-16-typed-event-schemas.md)：第一方事件是封闭 enum；`ignorable` 覆盖未知项；依赖组合的运行时注册表对第一方读取器仍被否决。该提案在宿主切换之前对 TypeScript 树保持开放。

保留或放弃表中列出的既有已实现笔记，对本次重写必须保住的约定仍有权威。本提案并不取代它们。

## 曾考虑的替代方案

**A. 渐进 napi-rs 混合：保留 Node 宿主，把热路径改写成 Rust。** Node 仍会拥有 Cordis、`!!js`、`worker_threads`、tsx 和插件图。napi-rs 不能实现 Landlock（restrict-self-then-exec 会替换进程）。审计面变成两种语言加 FFI。否决：它不产出 Rust 核心/后端，并且增加仓库已经为 Landlock 和 Win32 文件夹对话框避开的第三种 ABI。

**B. Rust 宿主 + 冻结线路上的 TypeScript Web UI（本提案）。** 匹配已经设计的 GUI 拆分（[GUI 分层与 RPC](../../implemented/architecture/2026-07-19-gui-layering-and-rpc-protocol.md)）：一套协议，更换载体。Python 已经对 bin 做 `Popen`。杠杆最高：阶段 5 替换 SDK 交付的 Node 运行时而不碰 React。代价：宿主必须复现 `__DSH_BOOT__`、`/plugins` 和会话事件保真；Typert 必须变成 IDL 或冻结描述符。

**C. 含新 GUI 的绿地 Rust（仅 egui/Leptos/Tauri）。** 一次程序删除约 72k LOC 的客户端 UI 以及 slot/conversation-node 扩展模型。Electron/Tauri 已预留，未交付。为 v1 否决：它把产品范围加倍，并推迟首个交付的 Rust agent。Tauri 稍后可以在 IPC `doFetch` 上复用 TS UI，而不必现在选择 C。

**D. 保留 TypeScript 与 vendored Cordis。** 零迁移成本。作为对本请求的答案被否决；记录下来，使日后回退有一个具名替代。

**E. 拒绝当前会话 JSONL，第一天就从格式 `1` 起步。** 预发布 *允许* 这样做，但那样 Rust 就不能双轨运行或回放 114 份已提交的 `session.jsonl` fixture。为重写窗口否决。Rust 读取器存在之后的第一次 **结构性** 变更可以升到 `1`，并带上版本笔记推迟的 n→n+1 升级器。

**F. `SessionEvent` 的运行时插件注册表（typetag/inventory）。** 最接近声明合并。版本机制笔记否决了依赖组合的已知集合：更瘦的构建会拒绝更全构建写出的日志。为第一方事件否决。

**G. QuickJS/Rhai `!!js` 兼容。** YAML 比特相同，代价是在 Rust 里嵌入 eval 故事。否决。封闭插值器加插件读取服务覆盖每一条已交付产品行。

**H. 经由 Rust `landlock` crate 的同进程 Landlock。** 限制 harness。否决：机制是助手的 self-restrict-then-exec。

## 验收标准

- 存在一份书面保留或放弃表（本笔记），后续实现 PR 引用它，而不是重新决定 Cordis、Landlock、`!!js` 或会话格式。
- 阶段 5 退出可观察：无密钥 headless 与 JSON-RPC 快照在 Rust bin 上通过；Python 无密钥测试对着该 bin 通过。
- 阶段 7 退出可观察：Linux web 快照回放对着服务现有 TypeScript SPA 的 Rust 宿主通过。
- 会话读取器拒绝外来格式版本和未知必读事件；接受 `ignorable: true` 剩余项；对 packed JSONL fixture 往返。
- 任何受限制工具交付之前存在沙箱失败即关测试。
- `native/landlock-run` 仍为 C；Rust 宿主只 spawn 它。
- Rust compose crate 不交付 `!!js` 求值器。
- 仍在交付的 TypeScript 包保留其 Vitest/覆盖率门禁，直到与 crate 替换在同一 PR 中删除。
- `docs/architecture.md` 不用作重写规格；仅在某一阶段交付时作为当前状态图更新。

## 风险

内核 LOC 小而不变量密（fiber 拆除、事务性 compose、isolate、waterfall `next()`）。省略这些的浅 DI crate 会在 preset 隔离和失败即响结算上失败。

GUI 宿主必须复现查找策略（`agentFor` 活/冷/去重/subagent 围栏）以及两条下行流。对 `/api` 做 “REST 重写” 会打破 TypeScript 客户端，即使一元方法看起来相似。

Linux 上的 PTY 空闲检测使用 `/proc/<pid>/mem` 加体系结构 syscall 表。只等待 prompt/silence 的 `portable-pty` 移植会改变对模型可见的发送完成。

Windows ACL 按构造就是 partial（Everyone、硬链接）。在 Rust 里“清理它”可能 fail-open 或打断 pwsh 启动。

若产品 bin 已是 Rust 而快照仍对着 TypeScript bin，或两个 bin 对着不同组合作为合并门禁，双轨运行可能假绿。

把 Landlock 再写成 Rust 或嵌入 `!!js` 看起来像简化，却是已记录的错误。

首次打标签的发布会冻结会话格式读取器地板。作为该发布交付的重写必须包含对更新日志的拒绝/降级；缺失该行为永远无法加到用户已经在跑的副本上（[会话日志版本](../../implemented/architecture/2026-08-10-session-log-version-mechanism.md)）。
