# dsh-host

[English](README.md) | 中文

DeepSeek Harness Rust 进程的 GUI 宿主 crate。本 crate 将在后续任务中拥有 axum 监听器。它导出 `/api` 信任围栏（`is_trusted_api_request`、`assert_trusted_authority`、`is_loopback_hostname`）、特权方法集合（`is_privileged_method`、`privileged_requires_loopback`），以及 SPA dist、`/plugins` bundle 与 `window.__DSH_BOOT__` 的纯文件／启动辅助函数。

`is_loopback_hostname` 接受主机名（端口已剥离）：`localhost`、`[::1]`，或 IPv4 127/8。`is_trusted_api_request` 接受 Host 头（`host[:port]`）、Origin、`sec-fetch-site` 与 `trustedHosts`。缺失或无法解析的 Host 不受信任。Host 必须是 loopback 或已声明的受信任 authority。`sec-fetch-site: cross-site` 会被拒绝。缺少 Origin 时，在通过 Host 检查后视为受信任。Origin `"null"` 会被拒绝。Origin 的 host 必须与 Host 的 host 相等。

`assert_trusted_authority` 要求裸的 `host` 或 `host:port`，且经 WHATWG 等价解析后保持不变（按小写比较）。路径、userinfo、空白、悬空冒号，以及 `0x7f.0.0.1` 这类非规范主机名会以 `TrustError` 明确失败。

特权 dotted 方法与 TypeScript `PRIVILEGED_METHODS` 集合一致。当且仅当 Host 头解析为 loopback 主机名时，`privileged_requires_loopback` 为 true。即使 `trustedHosts` 能通过外层围栏，特权方法仍要求 loopback。

`serve_spa` 按 URL 路径从 dist 目录取文件：`..` 或 NUL 返回 403；缺失文件或目录回退为注入启动清单后的 `index.html` 200。`serve_plugin_js` 读取 `{root}/{package_name}/lib/client.js`（缺失为 404），并设置 `Cache-Control: no-cache`；`serve_plugin_source_map` 使用 `{path}.map`，规则相同。

`scan_client_packages` 读取每个 `{dir}/*/package.json`，并取嵌套的 `dsh.client` 对象（不是顶层 `"dsh.client"` 键）。没有 `dsh` 对象的包会被跳过。`platform == "web"` 时必须存在 `lib/client.js`，否则以 `HostError` 失败，错误文本包含 `pnpm run build` 或 `client bundle not found`。`immediately` 默认为 false；`inject` 是可选字符串数组。畸形的 `dsh` / `dsh.client` 会明确失败。

`inject_boot_manifest` 在 `</head>`（任意大小写）之前插入 `<script>window.__DSH_BOOT__ = {…}</script>`；若没有该标签则前置。JSON 序列化之后，每个 `<` 都会替换为 `\u003c`。每条记录的 `url` 是 `/plugins/<id>/client.js?rev=<hex>`。条目 `rev` 是该包 `lib/client.js` 字节的小写十六进制 SHA-256。图的 `rev` 是按 `id` 排序后，将每条记录的 `id` 再接 `rev` 做 UTF-8 拼接，再取小写十六进制 SHA-256。

本 crate 不监听、不注册 plugin、不序列化 RPC。它不依赖 `dsh-cli`、`dsh-headless` 或 `dsh-rpc`。`dsh-agent` 不得依赖本 crate。

## 已知限制与暂缓事项

- axum 监听器、一元 `/api` 分发与 WebSocket 下行链路尚未进入本 crate。
- Web 组合的 plugin YAML 名称作为字符串常量放在 `dsh-boot`；本 crate 尚未注册它们。
