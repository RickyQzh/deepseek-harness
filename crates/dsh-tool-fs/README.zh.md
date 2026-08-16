# dsh-tool-fs

[English](README.md) | 中文

面向 DeepSeek Harness Rust 宿主的模型侧 `read`、`write`、`edit` 文件系统工具。

`register_fs_tools` 在 `ToolRuntime` 上注册这三个名称。此处不注册 `search`。`FsToolContext` 持有本地文件系统、`ObservationGate` 与所有者、可选沙箱策略、子进程运行时（供后续 `search` 使用），以及 `rg_binary`。

`read` 解析 `file_path`，做 stat，缺失时观察为不存在（`FS_NOT_FOUND`），拒绝非普通文件（`FS_NOT_REGULAR_FILE`），然后读取 UTF-8 文本、观察为存在，并返回 `{ path, text }`。渲染为从 1 起编号的行 `{line_no:>6}|{line}`；末尾换行会产生最后一段空行。

`write` 与 `edit` 在调用 `write_text` / `edit_text` 之前从 `ObservationGate` 取得变更意图。该所有者尚未先读就 edit，结果为 `FS_NOT_OBSERVED`。防护变更消息通过 `remediate_fs_error` 追加 ` — re-read the file, then retry`（`FS_STALE_VERSION`）或 ` — read the file, then retry`（`FS_NOT_OBSERVED`）。`FsError` 变为名为 `FsError`、带 `FS_*` 码的 `ToolError::Coded`。

当 `FsToolContext.sandbox` 为 `None` 或文件系统无围栏时，拒绝提权字段 `sandbox_permissions` 与 `justification`：`sandbox_permissions is not available in this composition (no sandboxing filesystem to escalate)`。

## 已知限制与暂缓事项

- 不注册 `search`；`rg_binary` 与子进程运行时仅为后续 crate 变更而保存。
- `read` 不做窗口、流式或行数封顶；它把整个 UTF-8 文件渲染为编号行。
- 不注册 `read_image`。
- 会校验提权参数配对，但不实现经批准的更宽模式重试；变更调用传入的是现有的 `ctx.sandbox`。
