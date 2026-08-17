# dsh-host

[English](README.md) | 中文

DeepSeek Harness Rust 进程的 GUI 宿主 crate。它只在 `127.0.0.1` 上绑定 axum 监听器（`HostBind`；其它 `listen_host` 均为 `HostError`），提供 SPA dist 与 `/plugins` bundle，通过 `RpcHandler` 分发一元 `POST /api/<dotted>` 的 JSON `client-request` 信封，并经 WebSocket 承载 mux/host GUI 下行。端口 `0` 由操作系统分配。`ListeningHost::local_addr` 报告实际绑定地址；`shutdown` 停止 accept 并等待进行中的请求结束。`ListeningHost::hub` / `HostState::hub` 是用于发布下行帧的可克隆 `DownlinkHub`。

每个 `/api` 请求都会用 Host、Origin、`sec-fetch-site` 与 `trustedHosts` 运行 `is_trusted_api_request`。不受信任的 `/api` 返回 HTTP 403，正文为 `forbidden`。JSON 解析之后，特权 dotted 方法（见 `is_privileged_method`）会按 `trustedHosts` 为空再检查一次：`privileged_requires_loopback` 必须为 true，因此即使 Host 列在 `trustedHosts` 中，非 loopback Host 仍是 403。没有 CORS，也没有 TLS。

`POST /api/<dotted>` 要求 `Content-Type: application/json`（忽略 `;` 之后的参数）；其它媒体类型返回 HTTP 415，正文为 `content type must be application/json`。不是 `client-request` 的 JSON，或其 `method` 与 `/api/` 之后的路径后缀不相等，返回 HTTP 400。未知 dotted 方法是 HTTP 404（载体），不是 `RpcResult::err`。已挂载处理函数的业务结果（含错误）是 HTTP 200 + `server-response`。本 crate 的 `StubHandler` 只回答 `host.describe`（`version` 为 `0.0.1`，`cwd` 来自 `DSH_CWD` 或 `current_dir`，`attachedSessions` 为 `0`，`canOpenPath` 为 `false`）。`POST /api/respond` 为 HTTP 501。

对 `/api/events.mux` 与 `/api/events.host` 的无 WebSocket Upgrade 的 `GET`/`HEAD` 返回 HTTP 426，并带 `Upgrade: websocket`。受信任的带 Upgrade 的 GET 在这两条路径上打开仅下行的 WebSocket；其它路径不接受 WebSocket。
每条文本帧是一条来自 `DownlinkHub::publish_mux` 或 `publish_host` 的 `server-request` JSON 文档（`RpcMessage::server_request`）。落后的订阅者会跳过。客户端发来的文本或二进制应用消息会关闭套接字且不回显。Ping/pong 由 tungstenite 处理。
特权方法检查不适用于升级（这些路径不是 dotted 方法）。Host/Origin/`sec-fetch-site` 检查仍然适用；不受信任的升级返回 HTTP 403。
`GET`/`HEAD` `/plugins/<id>/client.js` 使用静态辅助函数；含 `/` 的 scoped 图 id 会映射到扫描得到的目录名，不会当作单个路径段传入。其它 `GET`/`HEAD` 路径使用 `serve_spa`。这些静态路径上的其它 HTTP 方法返回 HTTP 405。

`is_loopback_hostname` 接受主机名（端口已剥离）：`localhost`、`[::1]`，或 IPv4 127/8。`assert_trusted_authority` 要求裸的 `host` 或 `host:port`，且经 WHATWG 等价解析后保持不变（按小写比较）。路径、userinfo、空白、悬空冒号，以及 `0x7f.0.0.1` 这类非规范主机名会以 `TrustError` 明确失败。

`serve_spa` 按 URL 路径从 dist 目录取文件：`..` 或 NUL 返回 403；缺失文件或目录回退为注入启动清单后的 `index.html` 200。`serve_plugin_js` 读取 `{root}/{package_name}/lib/client.js`（缺失为 404），并设置 `Cache-Control: no-cache`；`serve_plugin_source_map` 使用 `{path}.map`，规则相同。

`scan_client_packages` 读取每个 `{dir}/*/package.json`，并取嵌套的 `dsh.client` 对象（不是顶层 `"dsh.client"` 键）。没有 `dsh` 对象的包会被跳过。`platform == "web"` 时必须存在 `lib/client.js`，否则以 `HostError` 失败，错误文本包含 `pnpm run build` 或 `client bundle not found`。`immediately` 默认为 false；`inject` 是可选字符串数组。畸形的 `dsh` / `dsh.client` 会明确失败。空的 client-package 目录得到空图。

`inject_boot_manifest` 在 `</head>`（任意大小写）之前插入 `<script>window.__DSH_BOOT__ = {…}</script>`；若没有该标签则前置。JSON 序列化之后，每个 `<` 都会替换为 `\u003c`。每条记录的 `url` 是 `/plugins/<id>/client.js?rev=<hex>`。条目 `rev` 是该包 `lib/client.js` 字节的小写十六进制 SHA-256。图的 `rev` 是按 `id` 排序后，将每条记录的 `id` 再接 `rev` 做 UTF-8 拼接，再取小写十六进制 SHA-256。

本 crate 依赖 `dsh-rpc` 的信封类型与访问器（`rpc_id`、`method`、`payload`、`result`、`as_ok`）。它不依赖 `dsh-cli` 或 `dsh-headless`。`dsh-agent` 不得依赖本 crate。

## 已知限制与暂缓事项

- 测试客户端使用 tokio-tungstenite 0.29（`default-features = false`，仅 `connect` feature，`ws://127.0.0.1`，无 rustls、无 native-tls），因为 axum 0.8 的 `ws` 已锁定 0.29；第 7 阶段计划写的是 0.26，无法与该锁定统一。
- 尚未挂载 slash Typert Remote（`/api/commands/list` 与 `/api/commands/execute`）；`/api/` 之后含 `/` 的路径按未知 dotted 方法处理（HTTP 404）。
- `POST /api/respond` 返回 501。`StubHandler` 尚未实现其余 dotted `RpcMethodMap`。
- Web 组合的 plugin YAML 名称作为字符串常量放在 `dsh-boot`；本 crate 尚未注册它们。
