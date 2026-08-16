# Agent Note: Execution YAML plugins register from dsh-agent as a sibling of spine

Status: implemented

[English](2026-08-16-execution-plugins-in-dsh-agent.md) | 中文

## 问题

第 5 阶段需要按 YAML 命名的 subprocess、fs、shell 以及模型侧 fs/bash 工具。把这些 setup 折进 `register_spine_plugins` 会让每一次 spine 启动都依赖执行 crate。把这些 `register` 调用放进 `dsh-boot` 会引入产品 crate 依赖，并再次形成 [Spine YAML plugins](2026-08-16-spine-plugins-in-dsh-agent.md) 要防止的环。

## 决策

每个执行 crate 依赖 `dsh-boot` 与 `dsh-kernel`，并导出 `plugin::register`。`dsh_agent::register_execution_plugins` 是 `register_spine_plugins` 的兄弟函数：需要 bash 与 fs 工具的调用方两者都调用。`dsh-boot` 不依赖产品 crate。YAML 名称仍是 `dsh-boot` 中已有的封闭常量（`@deepseek-ai/dsh-subprocess-local`、`@deepseek-ai/dsh-fs-local`、`@deepseek-ai/dsh-shell-bash-local`、`@deepseek-ai/dsh-tool-fs`、`@deepseek-ai/dsh-tool-bash`）。

`Context::provide(name, value: T)` 存储 `T`；`inject::<T>()` 与 `get::<T>()` 返回 `Arc<T>`。执行插件提供内层值（`LocalSubprocessRuntime`、`LocalFileSystem`、`LocalBashExecutor`）并按该类型注入，因此构造函数拿到的是单层 Arc。

`@deepseek-ai/dsh-fs-local` 的 cwd 取自配置 `cwd`，否则 `DSH_CWD`，否则进程 cwd。`@deepseek-ai/dsh-shell-bash-local` 注入 `subprocess`，并提供无围栏的 `shell`（`BashConfig::default()`）。`@deepseek-ai/dsh-tool-fs` 注入 `tools`、`fs` 与 `subprocess`，然后以 `ObservationOwner(1)`、进程级 `ObservationGate`、`sandbox: None` 以及 `rg_binary: "rg"` 调用 `register_fs_tools`。`@deepseek-ai/dsh-tool-bash` 注入 `tools` 与 `shell`，并调用 `register_bash_tool(runtime, shell, None)`。`ToolRuntime::registered_names` 返回已排序的模型侧名称。

## 备选方案

**把执行插件折进 `register_spine_plugins`。** 否决：spine YAML（credentials、llm、tools、agent、sessions）必须能在不拉取 subprocess、fs、shell 或 tool crate 的情况下启动，且后续 bin 会显式调用该兄弟函数。

**在 `dsh-boot` 中放置 `register_execution_plugins`。** 否决：这与在 `dsh-boot` 中放置 spine 组合是同一类产品依赖环。

**`provide(Arc::new(T))` 再 `inject::<Arc<T>>`。** 否决：`provide` 已经把 `T` 包进 Arc，那样会存成 `Arc<Arc<T>>`，需要 `Arc<T>` 的构造函数无法通过类型检查。

**按 agent（智能体）分配 `ObservationOwner`。** 本阶段否决：TypeScript 的 actor 身份尚未挂载；进程级 `ObservationOwner(1)` 是已接受的缺口。

## 影响

需要 bash 与 fs 的 Headless 与 JSON-RPC bin 先调用 `register_spine_plugins`，再调用 `register_execution_plugins`。未知的执行 YAML 名称仍然会作为加载错误失败。第 5 阶段的 bash 无围栏。文件系统观察是进程级所有者 1。

`cargo test -p dsh-agent --offline execution_yaml_registers_bash` 会启动 spine 加上执行 YAML 列表，并断言 `registered_names` 包含 `bash` 与 `read`。

## 相关

spine 组合见 [Spine YAML plugins register from dsh-agent, not dsh-boot](2026-08-16-spine-plugins-in-dsh-agent.md)。程序级重写提案见 [Rewrite core and backend in Rust](../../proposed/architecture/2026-08-14-rust-rewrite.md)。
