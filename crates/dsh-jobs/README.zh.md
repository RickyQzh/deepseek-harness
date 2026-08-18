# dsh-jobs

[English](README.md) | 中文

面向 DeepSeek Harness Rust 宿主的后台任务 Service Definition：品牌 [`JobId`](src/brand.rs)、快照，以及 [`JobRegistry`](src/types.rs) trait。本 crate 不是 YAML 插件名；加载 `@deepseek-ai/dsh-jobs` 会按未知名称失败。进程内提供方是 [`dsh-jobs-local`](../dsh-jobs-local/README.md)。本 crate 不加入 `register_spine_plugins`，也不挂入 `base.cordis.yml`。

`JobId` 是包住 `dsh_brand::Branded<JobIdTag>` 的本地 newtype（`new` / `as_str`）。`JobKind` 为 `Bash`、`Subagent` 或 `PtySend`；id 前缀是 `bash`、`subagent` 与 `pty-send`。`JobStart.owner_session` 是可选的 `SessionId`，不是活的 agent（智能体）。`JobStart.run` 是同步的，且不得重入正在启动该任务的注册表。`dsh-jobs` 不依赖 `dsh-agent`。

有 owner 的访问比较 session id。`bash-1` 等 id 可预测，因此这道隔离是安全边界。`list(Some(other))` 返回空列表，不是错误。对他人 owner 的 `get` / `read` / `kill` 会失败。`caller: None` 只看见无 owner 的任务。无 owner 的任务（`owner_session: None`）对任何调用方开放。

结算遵循首次结果优先：一个终止状态、等待方只释放一次、`on_job_done` 只触发一次。`wait` 不在 trait 上；有界等待由 [`LocalJobRegistry::wait`](../dsh-jobs-local/README.md) 提供。

## 模型体验

通过生产方插件和 [`dsh-tool-jobs`](../dsh-tool-jobs/README.md) 间接影响。

#### KV Cache 影响

无直接失效。

## 已知限制与延后工作

- 未实现 isolate / preset 分层；`dsh-jobs-local` 提供一份进程全局注册表。
- 本阶段省略 `attachController` / `servesOwner`。
- 本阶段不把该 crate 挂入 `base.cordis.yml`。
