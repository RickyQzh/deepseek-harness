# dsh-session-persist

[English](README.md) | 中文

JSONL 是产品默认格式（`session.jsonl` / `session.jsonl.zstd`）。本 crate 编码 `type: "session"` 头行、事件行、打包的 chunk 行，以及可拼接的带校验和 zstd 帧。它依赖 `dsh-session`，不依赖 compose。

`JsonlSessionStore` 将未压缩、未打包的 JSONL 写入 `{root}/{sessionId}/session.jsonl`（`SessionId::as_str()`，没有 `--<project>--` 段），以便与 Python SDK 和 jsonrpc fixture（测试前置数据）一致；`flush` 可以整文件重写。`from_env` 在 `DSH_SESSION_ROOT` 已设置且非空时使用该目录，否则使用 `{DSH_HOME}/sessions`。

`plugin::register` 提供 `sessions` 服务：默认 `from_env`，当配置 `root` 为字符串时使用 `with_root`。

`parse_header_record` 在校验 `HeaderLine` 之前对已解析 JSON 运行 `refuse_foreign_format_version`，因此更新的格式会被拒绝为不受支持，而不是损坏。已退役的 `sandboxMode` / `approvalPolicy` 字段视为损坏（`session header uses retired policy baseline fields`）。`delegationDepth` 总会写出；内存中缺失的值记为 `0`。

压缩使用包装 libzstd 的 `zstd` 0.13 crate。crate `zstd` 0.13 为 MIT OR Apache-2.0；`zstd-sys` 捆绑 facebook/zstd，双重许可为 BSD-3-Clause OR GPL-2.0；本工作区采用 BSD 授权。帧通过 `Encoder::include_checksum(true)` 写出。

## 已知限制与暂缓事项

- SQLite 是后续的持久化后端；本 crate 不依赖 rusqlite。
- surface、derive 与修复仍在 `dsh-session`。
