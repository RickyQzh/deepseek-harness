# dsh-host

[English](README.md) | 中文

DeepSeek Harness Rust 进程的 GUI 宿主 crate。本 crate 将在后续任务中拥有 axum 监听器。当前只导出 `/api` 信任围栏（`is_trusted_api_request`、`assert_trusted_authority`、`is_loopback_hostname`）与特权方法集合（`is_privileged_method`、`privileged_requires_loopback`）。

`is_loopback_hostname` 接受主机名（端口已剥离）：`localhost`、`[::1]`，或 IPv4 127/8。`is_trusted_api_request` 接受 Host 头（`host[:port]`）、Origin、`sec-fetch-site` 与 `trustedHosts`。缺失或无法解析的 Host 不受信任。Host 必须是 loopback 或已声明的受信任 authority。`sec-fetch-site: cross-site` 会被拒绝。缺少 Origin 时，在通过 Host 检查后视为受信任。Origin `"null"` 会被拒绝。Origin 的 host 必须与 Host 的 host 相等。

`assert_trusted_authority` 要求裸的 `host` 或 `host:port`，且经 WHATWG 等价解析后保持不变（按小写比较）。路径、userinfo、空白、悬空冒号，以及 `0x7f.0.0.1` 这类非规范主机名会以 `TrustError` 明确失败。

特权 dotted 方法与 TypeScript `PRIVILEGED_METHODS` 集合一致。当且仅当 Host 头解析为 loopback 主机名时，`privileged_requires_loopback` 为 true。即使 `trustedHosts` 能通过外层围栏，特权方法仍要求 loopback。

本 crate 不监听、不注册 plugin、不序列化 RPC。它不依赖 `dsh-rpc`。

## 已知限制与暂缓事项

- axum 监听器、静态 SPA、`__DSH_BOOT__`、`/plugins`、一元分发与 WebSocket 下行链路尚未进入本 crate。
- Web 组合的 plugin YAML 名称作为字符串常量放在 `dsh-boot`；本 crate 尚未注册它们。
