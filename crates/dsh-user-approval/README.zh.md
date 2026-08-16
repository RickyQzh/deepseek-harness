# dsh-user-approval

[English](README.md) | 中文

Rust 宿主上失败即关闭的一次性审批。`ApprovalService::request` 要求当前有未结束的轮次；它追加 `approval/asked`，以默认值 `Unavailable` 运行 `approval/request` waterfall（瀑布式事件），再追加 `approval/decided`。策略 `Never` 仍写入该审计对，但在不调用应答者的情况下返回 `Rejected`。空闲时的 `request` 返回 `ApprovalError::Idle` 且不追加任何事件。

YAML `@deepseek-ai/dsh-user-approval` 的配置为 `{ policy?: "ask"|"never" }`，默认 `"ask"`；未知策略字符串在加载时失败。YAML `headless-auto-approve` 注册一个终端 waterfall 监听器，不调用 `next()` 并返回 `AllowedOnce`。`plugin::register` 与 `plugin::register_auto_approve` 位于本 crate；`dsh-boot` 不调用它们。当 `ctx.get::<ApprovalService>("approval")` 存在时，agent 插件把该服务安装为 tools 的 `Approver`。

`ApprovalOutcome` 是 `dsh-tools` 中的枚举。`Rejected`、`Cancelled` 与 `Unavailable` 的拒绝文本沿用 `dsh-tools` 中已有的 TypeScript 字符串。见[审批 seam Agent Note](../../.agents/notes/implemented/feature/2026-07-06-approval-seam.md)。

## 已知限制与暂缓事项

- 请求仅在未结束的轮次内有效；持久的轮次外审批工作流暂缓。
- 只存在一次性授权：`AllowedOnce`，没有记住的规则或授权存储；会话策略只有 `Ask` / `Never`。
- 运行时上下文中的策略句子以及实时 `setPolicy` agent 通知未在本 crate 挂载。
- waterfall 载荷仅为 `ApprovalOutcome`；应答者不会以类型化值收到 `ApprovalRequest`。
- 本 crate 从不向人发出提示；`headless-auto-approve` 是无人值守授权。
