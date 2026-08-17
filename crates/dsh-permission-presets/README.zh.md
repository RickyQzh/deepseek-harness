# dsh-permission-presets

[English](README.md) | 中文

将 `permission/preset`、`sandbox/mode` 与 `approval/policy` 钉入每个新会话，使之后的默认值变更无法改写已有日志。`PermissionPresetService::pin_initial` 会在全新未 seed 的会话（无 `seed_length` 且无 `session/end-seed`）上写入全部三个旋钮；已 seed 或部分日志保留已有旋钮，只补齐缺失项。

YAML `@deepseek-ai/dsh-permission-presets` 的配置为 `{ presets?, defaultPreset? }`。省略 `presets` 时加载 `workspace-write`（`workspace-write` + `ask`）与 `danger-full-access`（`danger-full-access` + `never`）；仅当 YAML 列出 `read-only` 时才包含它。省略 `defaultPreset` 时选用与组合后的沙箱和审批匹配的表行；没有匹配项是加载错误，而不是 `custom`。未知的 `defaultPreset` 以 `unknown preset` 失败。表键 `custom` 为保留名。`plugin::register` 位于本 crate；`dsh-boot` 不调用它。

该插件注入 `agents`、`approval` 和 `shell`，提供 `permissionPresets`，并注册 `agents.on_session_create`。组合沙箱为 `LocalBashExecutor::sandbox_mode()`；为 `None` 时使用 `workspace-write`。组合审批为 `ApprovalService` 的默认值（`ask`，除非 YAML `policy: never`）。`set_sandbox_mode` 追加事件时不查阅 PTY 状态。`set_sandbox_mode_with_terminals` / `set_sandbox_mode_in_world` 在 `has_owner_activity` 为 true 且模式不同时拒绝（`cannot change sandbox mode from "{current}" to "{next}" while persistent terminal sessions are open or being created; wait for creation to settle and close them first`）；相同模式仍会追加。`pin_initial` 跳过该围栏；`pin_initial_in_world` 将 `terminals` 查找为 `Mutex<TerminalSessionService>`。见[权限钉死不变量](../../.agents/notes/proposed/architecture/2026-08-14-rust-rewrite.md)。

## 已知限制与暂缓事项

- `/permission` 命令与 `permissions` 投影不在本 crate 中。
- `defaultPreset` 仅来自 YAML；没有面向后续会话的 settings 文件默认值。
- `LocalBashExecutor::sandbox_mode` 始终为 `None`，因此在提供具有约束能力的执行器之前，组合沙箱为 `workspace-write`。
