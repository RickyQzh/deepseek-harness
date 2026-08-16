# dsh-subagent

[English](README.md) | 中文

面向 DeepSeek Harness Rust 宿主的 subagent Service Definition：在 `subagents` 上的具名提供方注册表（`SubagentRuntime`），以及带能力检查的一次性 `start`。YAML 名称为 `@deepseek-ai/dsh-subagent`。本 crate 不加入 `register_spine_plugins`，也不挂入 `base.cordis.yml`。同进程 spawn 与 fork 提供方在 [`dsh-subagent-in-process`](../dsh-subagent-in-process/README.md)。`dsh-agent` 不依赖本 crate。

`inject::<SubagentRuntime>()` 得到 `Arc<SubagentRuntime>`，因此 `register_provider` 与 `start` 在内部 mutex 上取 `&self`。重复的提供方名称会失败。未知名称会失败。`start` 检查已声明的启动时能力，再委派给具名提供方；它不进入继续执行。`prepare_continuable` 是每个提供方上的数据桩，一次性 `start` 不会调用它。

`subagent/descriptor` 第 2 版是仅日志会话事件（`SessionEvent::SubagentDescriptor { data }`）。内核事件 `subagent/start` 与 `subagent/end` 不是会话事件。已发出 start 的已发布运行也必须发出 end，包括子轮次在发布后失败的情况。深度默认值为 3；当父级 `delegationDepth`（缺省为 0）加一超过上限时，`assert_subagent_max_depth` 会失败。

## 配置

无。未知键会在加载时失败。

## 模型体验

通过同进程提供方和后续面向模型的工具间接影响。本注册表不贡献工具 schema。

#### KV Cache 影响

无直接失效。

## 已知限制与延后工作

- 本阶段未实现可继续子 agent 与 `startContinuable`。
- 未移植进程外提供方（ACP、Codex、Claude Code、SDK）。
- 本阶段不把该插件挂入 `base.cordis.yml`。
