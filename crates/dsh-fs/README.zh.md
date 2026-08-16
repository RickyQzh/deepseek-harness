# dsh-fs

[English](README.md) | 中文

面向 DeepSeek Harness Rust 宿主的文件系统类型与 `FS_*` 错误码。

`FsTargetKey` 与 `FsVersion` 是包住 `dsh_brand::Branded` 的本地 newtype。品牌化不修剪也不校验。`FsError` 携带稳定的 `FsErrorCode`；`as_str` 返回与 TypeScript 相同的 `FS_*` 字符串。

`FS_SANDBOX_DENIED` 是进程内围栏拒绝。`FS_PERMISSION_DENIED` 是内核或操作系统权限失败。二者不可互换。

## 已知限制与暂缓事项

- 本地后端（Task 36）、沙箱围栏与观察策略（Task 37）不在本 crate。
