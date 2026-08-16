# Agent Note: Phase 6 product plugins register from dsh-base, not dsh-agent

Status: implemented

[English](2026-08-16-rust-dsh-base-plugins.md) | 中文

## 问题

第 6 阶段产品插件各自导出 `register`，但不得加入 `register_spine_plugins` 或 `register_execution_plugins`。若把该聚合函数放在 `dsh-agent` 上，`dsh-agent` 就会依赖 `dsh-subagent`。若放在 `dsh-boot` 上，则会再次形成 [Spine YAML plugins](2026-08-16-spine-plugins-in-dsh-agent.md) 要防止的产品 crate 环。

未设置 `DSH_CORDIS_CONFIG` 时，默认 CLI（命令行界面）与 jsonrpc 启动必须继续使用第 5 阶段的 `MINIMAL_YAML`，以便 `headless-ok` 保持通过。

## 决策

`crates/dsh-base` 拥有 `register_base_plugins`。它注册审批（`register` 与 `register_auto_approve`）、permission presets、llm-retry 以及 `register_retry_snapshot_backend`、token-meter、compaction-basic 以及 `register_pruner`、agent-instructions、time-context、skill（技能）三件套、web 加 DeepSeek search 加 tool-web、jobs-local 加 tool-jobs、subagent 加同进程 spawn/fork，以及 `dsh_tool_subagent::plugin::register`（四个 YAML 名称）。它不调用 spine、执行、headless 或 `sdk-jsonrpc-server` 的 register 函数。

`dsh-cli` 与 `dsh-sdk-jsonrpc-server` 依赖 `dsh-base`，并在 spine 与执行之后调用 `register_base_plugins`。`dsh-headless` 不调用；仅测试用的 `dev-dependency` 启动 `BASE_YAML`。`dsh-boot` 不依赖产品 crate。`dsh-base` 不依赖 `dsh-headless`、`dsh-cli` 或 `dsh-sdk-jsonrpc-server`。来自 `register_headless_plugins` 的重复 `headless-auto-approve` 注册会覆盖同一份 setup。

静态组合树是 `crates/dsh-headless/base.cordis.yml` 与 `crates/dsh-sdk-jsonrpc-server/base.cordis.yml`（`BASE_YAML`）。它们包含第 5 阶段 spine 与执行行、`headless-auto-approve`、user-approval `policy: ask`、三行 permission-presets 表（read-only、workspace-write、danger-full-access）、token-meter、compaction-basic、llm-retry、agent-instructions `maxBytes: 65536`、skill 三件套、web `searchProvider: deepseek-official`、web-search-deepseek `apiKeyEnv: DEEPSEEK_API_KEY`、tool-web `fetch: false` 与 `searchTimeoutMs: 60000`、jobs-local 与 tool-jobs、subagent 以及 spawn/fork 的 `providerName` 行、两行 `@deepseek-ai/dsh-tool-subagent` 挂载（spawn/continuable 的 `subagent` 与 fork/one-shot 的 `subagent_fork`），然后是 control、list（`PLUGIN_TOOL_SUBAGENT_LIST`）与 report。Headless YAML 以 `headless-startup` 然后 `headless-runner` 结尾，mock 文本为 `base-ok`。Jsonrpc YAML 改为挂载 `sdk-jsonrpc-server`，mock 占用 `provider: deepseek-official`，并省略 headless-startup/runner。两份文件都不含 `!!js`。这些文件省略 time-context、pruner、retry-snapshot-backend、workflow、goal、session-title、settings-file、commands、`@deepseek-ai/dsh-compaction` 与 `@deepseek-ai/dsh-jobs`。未设置 `DSH_CORDIS_CONFIG` 时仍加载 `MINIMAL_YAML`。

## 备选方案

**把 `register_base_plugins` 放在 `dsh-agent` 上。** 否决：`dsh-agent` 不得依赖 `dsh-subagent`。

**把 `register_base_plugins` 放在 `dsh-boot` 上。** 否决：这正是 spine 组合已经否决的产品依赖环。

**在 `dsh-cli` 与 `dsh-sdk-jsonrpc-server` 中各写一份 register 列表。** 否决：两份副本会漂移；两个 bin 都调用 `dsh_base::register_base_plugins`。

**把未设置 `DSH_CORDIS_CONFIG` 时的回退从 `MINIMAL_YAML` 换成 `BASE_YAML`。** 否决：第 5 阶段的 `headless-ok` / jsonrpc mock 必须保持为默认。

**移植 `!!js` 插值器或用环境变量三元表达式决定审批策略。** 否决：审批策略是字面量 `ask`。

**在默认 base YAML 中挂载 time-context、pruner、retry-snapshot-backend 或 `@deepseek-ai/dsh-tool-subagent-control/list-agents`。** 否决：那些名称是为后续 `--patch` 注册的，或根本未注册；未知名称会大声失败。list-agents 的名称是 `@deepseek-ai/dsh-tool-subagent-list`。

## 影响

`cargo test -p dsh-base --offline` 会启动一份去掉 `headless-startup` / `headless-runner` 的 headless `base.cordis.yml` 副本，断言 `approval`、`tokenMeter`、`compaction`、`skills`、`web`、`jobs`、`subagents`，以及工具 `skill` / `web_search` / `subagent` 且没有 `web_fetch`，并且未知 YAML 名称仍然大声失败。`cargo test -p dsh-headless --offline` 保持 `MINIMAL_YAML` 通过，并钉住 `BASE_YAML` 的 stdout 为 `base-ok\n`、退出码 0。

## 相关

spine 与执行组合见 [Spine YAML plugins](2026-08-16-spine-plugins-in-dsh-agent.md) 与 [Execution YAML plugins](2026-08-16-execution-plugins-in-dsh-agent.md)。程序级重写提案见 [Rewrite core and backend in Rust](../../proposed/architecture/2026-08-14-rust-rewrite.md)。
