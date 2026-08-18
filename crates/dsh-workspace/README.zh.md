# dsh-workspace

[English](README.md) | 中文

DeepSeek Harness Rust 宿主的持久化 JSON 工作区注册表。一个工作区是建立在已有目录上的稳定 id、一个显示标题，以及有序的会话账本。本 crate 不提供 HTTP，也不读取会话头。

`WorkspaceRegistry::with_path` 把一份 camelCase JSON 对象持久化到该路径（`workspaceIds`、`archivedSessionIds`、`workspaces`）。文件缺失视为空注册表。损坏的 JSON 会使下一次 `list` 或变更失败。写入使用同目录临时文件再 rename。进程内调用方由 `Mutex` 串行化。

`create` 要求目录已存在（`std::fs::canonicalize`），不会 mkdir。同一规范路径返回该工作区，且 `created: false`、标题不变。新工作区插入到显示顺序的最前；默认标题是路径的 basename。创建时不同规范路径可以共用同一显示标题。Id 为 `wk-{pid}-{nanos}`。时间戳为 ISO-8601 UTC（`YYYY-MM-DDTHH:MM:SS.sssZ`）。

`rename` 会 trim；空标题是 `title-invalid`；与另一工作区标题相同是 `workspace-name-conflict`；改成当前标题是 no-op。`delete` 只取消注册（目录和会话日志保留）；未知 id 是 `workspace-not-found`。`insert_before` 是 DOM-insertBefore（省略锚点则追加到末尾）。`attach_session` 在缺失时前置，已存在则幂等成功。`archive_session` 追加到注册表级全局归档集合，不从 `sessionIds` 移除。

`plugin::register` 挂载 YAML `@deepseek-ai/dsh-workspace` 并提供 `workspaces`。配置为可选 `{ "path": "<string>" }`。省略 `path` 时，若设置了 `DSH_HOME` 则使用 `{DSH_HOME}/workspaces.json`，否则 `{DSH_SESSION_ROOT}/workspaces.json`。配置未给 `path` 且两个环境变量都未设置，或持久化文件的父目录无法创建时，加载失败。测试使用 `with_path`。

`WorkspaceError::code` 是 kebab-case 传输字符串；`rpc_code` 映射到 `dsh_rpc::RpcErrorCode`。本 crate 不发出 `session-not-found` 或 `workspace-attach-failed`。

## 已知限制与暂缓事项

- 一元 `workspace.*` RPC 与宿主插件装配不在本 crate；`dsh-base` 与 `dsh-cli` 不注册此插件。
- attach 不读取会话头，也不依赖 `dsh-session-persist`；cwd 校验属于后续宿主任务。
