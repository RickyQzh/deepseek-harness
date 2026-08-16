# dsh-session

[English](README.md) | 中文

Rust 宿主的仅追加会话日志类型：品牌化的 `SessionId` / `MessageId` / `CallId`、`SESSION_FORMAT_VERSION = 0`、封闭的第一方 `SessionEvent` 枚举、surface 折叠、`derive_messages`、请求头折叠、中断轮次修复，以及打包的 chunk 行编码。

`Session::set_append_sink` 安装可选观察者，在每次成功的 `append` 之后调用（`seq` 已分配）。

本 crate 不依赖 `dsh-compose` 或 `dsh-kernel`。JSONL 成帧与 zstd 位于 `dsh-session-persist`。产品 id 是包住 `dsh_brand::Branded` 的本地 newtype；`dsh-brand` 不命名它们。

## 已知限制与暂缓事项

- SQLite `SCHEMA_VERSION = 15` 是后续的持久化后端，不属于本 crate。
- `Session` 不会自动追加 `session/end-seed`；那是 store 创建行为。
