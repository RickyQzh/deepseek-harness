# dsh-jobs-local

[English](README.md) | 中文

进程内 [`JobRegistry`](../dsh-jobs/README.md) 提供方。`plugin::register` 以 `LocalJobRegistry` 提供 `jobs`。`inject::<LocalJobRegistry>()` 得到 `Arc<LocalJobRegistry>`，因此 `start` / `list` / `get` / `read` / `kill` / `wait` 在内部 mutex 上取 `&self`。YAML 名称为 `@deepseek-ai/dsh-jobs-local`。本 crate 不加入 `register_spine_plugins`，也不挂入 `base.cordis.yml`。

`LocalJobRegistry::new(max_concurrent_per_owner)` 设置活跃任务上限。配置 `maxConcurrentJobsPerOwner` 必须是正整数，默认值为 `10`。`start()` 在活跃计数检查、`run()` 与插入期间持有注册表 mutex，因此并发 `start` 不会超过上限。`run` 不得重入该注册表。所有无 owner 任务共用一个桶。id 为 `<kind>-N`，计数器按 kind 在进程内独立：即使已有 subagent 任务，第一个 bash 任务仍是 `bash-1`。`run` 中的 panic 不会被捕获；只有 `run` 返回后才会登记。丢弃 `wait` future 会取消等待计数，因此被取消的等待不会压制 `on_job_done`。

结算遵循首次结果优先。`on_job_done` 在记录进入终止状态之后触发。`wait` 在超时时返回当前快照并让任务继续存活。`Drop` 与 kernel dispose 会对仍在运行的任务调用 `cancel`。`cancel` 是同步且幂等的。panic 的 `done` future 记为 `failed`。

## 配置

| 键 | 默认 | 含义 |
|---|---|---|
| `maxConcurrentJobsPerOwner` | `10` | 每个 owner（或共享的无 owner 桶）中 `running` 加 `stopping` 任务的上限。 |

未知键会在加载时失败。

## 模型体验

通过生产方插件和 [`dsh-tool-jobs`](../dsh-tool-jobs/README.md) 间接影响。

#### KV Cache 影响

无直接失效。

## 已知限制与延后工作

- 一份进程全局注册表；未实现 isolate / preset 分层。
- 任务是进程内的；记录随 harness 进程结束而消失。
- 本阶段不把该插件挂入 `base.cordis.yml`。
