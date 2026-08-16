# dsh-fs

[English](README.md) | 中文

面向 DeepSeek Harness Rust 宿主的文件系统类型、`FS_*` 错误码、本地 UTF-8 后端、进程内沙箱围栏，以及按所有者隔离的观察映射。

`FsTargetKey` 与 `FsVersion` 是包住 `dsh_brand::Branded` 的本地 newtype。品牌化不修剪也不校验。`FsError` 携带稳定的 `FsErrorCode`；`as_str` 返回与 TypeScript 相同的 `FS_*` 字符串。

`FS_SANDBOX_DENIED` 是进程内围栏拒绝。`FS_PERMISSION_DENIED` 是内核或操作系统权限失败。二者不可互换。

`LocalFileSystem` 将相对路径接到 `cwd`（或单次调用的 cwd），对最深的已存在祖先做 realpath 再拼上剩余后缀，并用该规范字符串同时作为 `target_key` 与 `display_path`。`read_text` 只接受常规 UTF-8 文件，拒绝前 8192 字节中的 NUL 以及非法 UTF-8，并将 `\r\n` 规范为 `\n`。`write_text` 先写入同目录 `0o600` 临时文件再 `rename` 发布；省略 intent 即为无条件创建或覆盖。`edit_text` 在 LF 规范化文本上做字面替换，若原文件为 CRLF 则发布时恢复 CRLF。版本令牌为 `{dev}:{ino}:{size}:{mtime_nsec}:{ctime_nsec}`。

`LocalFileSystem::new` 无围栏：`sandbox_mode()` 为 `None`，并忽略 `sandbox_policy`。`LocalFileSystem::sandboxed` 安装 `SandboxFence`；`sandbox_mode()` 为该策略的模式。围栏是进程内容纳检查，不是内核边界。`read-only` 以 `FS_SANDBOX_DENIED` 拒绝变更。`workspace-write` 重新解析 `display_path`，要求其落在 `dsh_sandbox::writable_roots` 的某一根下（`is_path_under`），然后对新鲜目标执行变更。`danger-full-access` 返回原始目标。读取从不围栏。`is_path_under` 在 Linux 上使用带 `MAIN_SEPARATOR` 的区分大小写词法前缀；拼写不同时沿祖先比较 `(dev, ino)`；缺失的根不算包含。

`ObservationGate` 按 `(所有者 id, target_key)` 记录每个所有者的存在或缺失，并据此派生写入/编辑意图。不同所有者不共享观察。缺失所有者从不查找。该映射未接入 `LocalFileSystem` 方法。

`plugin::register` 挂载 YAML `@deepseek-ai/dsh-fs-local`，并提供 `fs` 服务（`LocalFileSystem`）。cwd 取自配置 `cwd`，否则 `DSH_CWD`，否则进程 cwd。

## 已知限制与暂缓事项

- 沙箱围栏是进程内容纳检查，不是内核隔离；重新解析与写入系统调用之间仍有残余 TOCTOU。
- `ObservationGate` 是独立的按所有者映射；`LocalFileSystem` 不在读、写或编辑时记录观察。
