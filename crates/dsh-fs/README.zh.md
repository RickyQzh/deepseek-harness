# dsh-fs

[English](README.md) | 中文

面向 DeepSeek Harness Rust 宿主的文件系统类型、`FS_*` 错误码，以及无围栏的本地 UTF-8 后端。

`FsTargetKey` 与 `FsVersion` 是包住 `dsh_brand::Branded` 的本地 newtype。品牌化不修剪也不校验。`FsError` 携带稳定的 `FsErrorCode`；`as_str` 返回与 TypeScript 相同的 `FS_*` 字符串。

`FS_SANDBOX_DENIED` 是进程内围栏拒绝。`FS_PERMISSION_DENIED` 是内核或操作系统权限失败。二者不可互换。

`LocalFileSystem` 将相对路径接到 `cwd`（或单次调用的 cwd），对最深的已存在祖先做 realpath 再拼上剩余后缀，并用该规范字符串同时作为 `target_key` 与 `display_path`。`read_text` 只接受常规 UTF-8 文件，拒绝前 8192 字节中的 NUL 以及非法 UTF-8，并将 `\r\n` 规范为 `\n`。`write_text` 先写入同目录 `0o600` 临时文件再 `rename` 发布；省略 intent 即为无条件创建或覆盖。`edit_text` 在 LF 规范化文本上做字面替换，若原文件为 CRLF 则发布时恢复 CRLF。版本令牌为 `{dev}:{ino}:{size}:{mtime_nsec}:{ctime_nsec}`。`sandbox_mode()` 返回 `None`。`sandbox_policy` 参数被忽略。

## 已知限制与暂缓事项

- 无沙箱围栏或观察策略；`sandbox_mode()` 为 `None`，`sandbox_policy` 被忽略，直至 Task 37。
