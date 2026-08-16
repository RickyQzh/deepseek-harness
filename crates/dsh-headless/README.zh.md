# dsh-headless

[English](README.md) | 中文

DeepSeek Harness Rust 宿主的一次性 headless 运行器。

`register_headless_plugins` 依次注册 YAML 名称 `headless-startup` 与 `headless-runner`。`headless-startup` 读取 `cmdlineArgs` 并 `provide` `headlessStartup`，其 task 是用空格拼接的内部 argv。`headless-runner` 注入 `agents` 与 `sessions`，从 `provide`（代码）读取 `headlessStartup`，驱动一次用户 followup，打印最后一段 assistant 文本并追加换行，且仅当所属区间最后一次 `turn/end` 的 reason kind 为 `completed` 时请求退出码 0。失败时向 stderr 写入 `dsh: {message}\n` 并请求退出码 1。启动器必须在挂载树之前 `provide` `appExit`。测试可以 `provide` `headlessIo` 以捕获流；该服务缺席时，运行器写入进程 stdio。task 文本绝不是 YAML `!!js` 字段。

## 已知限制与暂缓事项

- 本 crate 不组合 `dsh-base`。
- 不实现会话标题的 LLM（大语言模型）调用。
- 持久化路径为 `{DSH_SESSION_ROOT}/{id}/session.jsonl`（不是 `--<project>--`）。
