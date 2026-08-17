# crates

[English](README.md) | 中文

DeepSeek Harness 宿主进程的 Rust workspace 成员。布局、保留/放弃项以及 crate 边界记录在 [Rust 重写 Agent Note](../.agents/notes/proposed/architecture/2026-08-14-rust-rewrite.md)。在后续阶段切换之前，`packages/` 下的 TypeScript 包仍是已发布的宿主。

## 成员

| Crate | 职责 |
|---|---|
| [`dsh-brand`](dsh-brand/README.md) | 品牌 id 原语（`Branded<B>`）。具体产品 id 放在所属 crate。 |
| [`dsh-kernel`](dsh-kernel/README.md) | Context、Fiber、服务、effect、isolate、事件总线。 |
| [`dsh-events`](dsh-events/README.md) | 内核事件总线的再导出（`emit` / `serial` / `parallel` / `waterfall`）。 |
| [`dsh-schema`](dsh-schema/README.md) | 插件配置 schema + Settings UI 的 JSON Schema。 |
| [`dsh-compose`](dsh-compose/README.md) | 封闭 YAML 方言：插值器、补丁、层序、disabled 谓词。 |
| [`dsh-boot`](dsh-boot/README.md) | 封闭的 YAML 名称注册表，以及把 compose 行挂载进 kernel fiber。 |
| [`dsh-session`](dsh-session/README.md) | 会话 id、封闭 `SessionEvent` 枚举、surface、derive、修复、chunk 行。 |
| [`dsh-session-persist`](dsh-session-persist/README.md) | JSONL + zstd 会话编解码（SQLite 稍后）。 |
| [`dsh-tools`](dsh-tools/README.md) | 工具执行类型与无损 JSON 参数冻结。 |
| [`dsh-user-approval`](dsh-user-approval/README.md) | 失败即关闭的 `approval/request` waterfall（瀑布式事件）、每会话 ask/never 策略，以及 `headless-auto-approve`。 |
| [`dsh-permission-presets`](dsh-permission-presets/README.md) | 在会话创建时钉死 `permission/preset`、`sandbox/mode` 与 `approval/policy`。 |
| [`dsh-system-prompt`](dsh-system-prompt/README.md) | 有序系统提示词段落、运行时上下文快照，以及严格的 `{{var}}` 插值。 |
| [`dsh-credentials`](dsh-credentials/README.md) | 按次解析 POSIX 凭据引用（环境、YAML 映射、内存）以及持久化 `credentials.yaml` 写入。 |
| [`dsh-settings`](dsh-settings/README.md) | 用户 settings 命名空间（`ui-onboarding`）与带 revision 的 JSON 文件。 |
| [`dsh-commands`](dsh-commands/README.md) | 进程内 slash 命令注册表：解析、按名称排序的列表与执行。 |
| [`dsh-llm`](dsh-llm/README.md) | 提供方无关的 LLM（大语言模型）流契约、块组装器与 mock 适配器。 |
| [`dsh-llm-deepseek`](dsh-llm-deepseek/README.md) | DeepSeek `POST /chat/completions` SSE（Server-Sent Events）适配器：序列化、翻译、按次密钥、空闲超时。 |
| [`dsh-token-meter`](dsh-token-meter/README.md) | 重放 token 计量：固定每 4 字符一 token 的启发式、surface fold、提供方 usage 锚点。 |
| [`dsh-compaction`](dsh-compaction/README.md) | 压缩引擎类型、检查点来源与工具配对切割辅助函数。 |
| [`dsh-compaction-basic`](dsh-compaction-basic/README.md) | 仅日志压缩锁、表层替换与溢出重试。 |
| [`dsh-agent-instructions`](dsh-agent-instructions/README.md) | 工作区 `AGENTS.md` / `CLAUDE.md` 基线注入与 JSONL 恢复。 |
| [`dsh-time-context`](dsh-time-context/README.md) | 可选的 pre-step 时钟注入；标准 UTC 时间戳与紧凑时长文本。 |
| [`dsh-skill`](dsh-skill/README.md) | skill（技能）提供方注册表、本地文件系统提供方，以及面向模型的 `skill` 工具。 |
| [`dsh-web`](dsh-web/README.md) | Web 搜索 Service Definition：提供方注册表、选择与 `search`。本阶段省略 fetch（SSRF）。 |
| [`dsh-web-search-deepseek`](dsh-web-search-deepseek/README.md) | DeepSeek Anthropic 兼容 `web_search_20250305` 搜索提供方。 |
| [`dsh-tool-web`](dsh-tool-web/README.md) | 面向模型的 `web_search` 工具。`web_fetch` 保持关闭。 |
| [`dsh-jobs`](dsh-jobs/README.md) | 后台任务 Service Definition：品牌 id、快照与 `JobRegistry`。 |
| [`dsh-jobs-local`](dsh-jobs-local/README.md) | 进程内 `jobs` 提供方（`LocalJobRegistry`）。 |
| [`dsh-tool-jobs`](dsh-tool-jobs/README.md) | 面向模型的 `job_output` / `job_list` / `job_kill` 工具。 |
| [`dsh-subagent`](dsh-subagent/README.md) | Subagent Service Definition：具名提供方、一次性 `start`，以及可继续的 `start_continuable`。 |
| [`dsh-subagent-in-process`](dsh-subagent-in-process/README.md) | 同进程 spawn/fork 一次性提供方与共享子 agent 驱动器。 |
| [`dsh-tool-subagent`](dsh-tool-subagent/README.md) | 面向模型的 `subagent`、`subagent_fork`、`send_message`、`list_agents` 与 `report`。 |
| [`dsh-agent`](dsh-agent/README.md) | 按会话 id 持有的实时 LoopAgent 注册表；Session append-sink 工厂。 |
| [`dsh-agent-loop`](dsh-agent-loop/README.md) | 脚本化循环：持久化 inbox、idle / maintenance / running、运行时上下文快照、请求重建、工具调用调度。 |
| [`dsh-subprocess`](dsh-subprocess/README.md) | 完全指定的 argv 派发与凭据擦除后的子进程环境。 |
| [`dsh-sandbox`](dsh-sandbox/README.md) | 失败即关闭的沙箱模式、可写根与提权。 |
| [`dsh-fs`](dsh-fs/README.md) | 文件系统类型与 `FS_*` 错误码。 |
| [`dsh-shell`](dsh-shell/README.md) | shell 请求/规格类型、POSIX bash 执行器与退出状态解析。 |
| [`dsh-tool-fs`](dsh-tool-fs/README.md) | 面向模型的 read/write/edit/glob/grep 文件系统工具。 |
| [`dsh-tool-bash`](dsh-tool-bash/README.md) | 面向模型的 bash 工具。 |
| [`dsh-sdk-protocol`](dsh-sdk-protocol/README.md) | SDK JSON-RPC 2.0 协议类型（`initialize`、`session/prompt`、`shutdown`、四类通知）。 |
| [`dsh-sdk-jsonrpc-server`](dsh-sdk-jsonrpc-server/README.md) | NDJSON JSON-RPC SDK 服务器与 `dsh-jsonrpc-agent` stdio bin。 |
| [`dsh-acp`](dsh-acp/README.md) | ACP（Agent Client Protocol）stdio 适配器：NDJSON JSON-RPC，不是 SDK JSON-RPC 服务器。 |
| [`dsh-mcp-client`](dsh-mcp-client/README.md) | MCP 客户端：`mcp__` 公开工具名、`extract_text`，以及 YAML `@deepseek-ai/dsh-mcp-client`。 |
| [`dsh-terminal`](dsh-terminal/README.md) | 按 owner 隔离的 PTY 注册表、品牌 id，以及内存 YAML `pty-snapshot-backend`。 |
| [`dsh-terminal-bash`](dsh-terminal-bash/README.md) | 交互式 bash PTY 后端：配置、环境覆盖，以及 prompt/silence/timeout 就绪检测。 |
| [`dsh-tool-terminal`](dsh-tool-terminal/README.md) | 六个面向模型的 `terminal_*` 工具、render，以及 `tool:pty` 指引。 |
| [`dsh-headless`](dsh-headless/README.md) | 一次性 headless 运行器：打印最后一段 assistant 文本加换行；仅当 `turn/end` 为 `completed` 时退出码 0。 |
| [`dsh-base`](dsh-base/README.md) | 第 6 阶段产品插件聚合器（`register_base_plugins`）以及 headless/jsonrpc 的静态 `base.cordis.yml`。 |
| [`dsh-cli`](dsh-cli/README.md) | `dsh` clap 启动器：`--profile headless` / `web` / `acp`、`dsh web` / `dsh acp` 别名、`--patch`。 |
| [`dsh-rpc`](dsh-rpc/README.md) | GUI 四象限信封（`RpcMessage`、`RpcResult`、`RpcReceipt`）与封闭的 kebab-case 错误码。 |
| [`dsh-workspace`](dsh-workspace/README.md) | 持久化 JSON 工作区注册表（`workspaces.json`）：路径、标题、会话账本与归档集合。 |
| [`dsh-host`](dsh-host/README.md) | GUI loopback HTTP 监听器、一元 `/api` dotted 与 slash RPC、信任围栏、SPA 与 `/plugins`。 |
