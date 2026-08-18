# Agent Note: Compose the Rust dsh web plugin graph in dsh-host

Status: implemented

[English](2026-08-17-rust-web-plugin-graph.md) | 中文

## 问题

第 7 阶段已经绑定 loopback 监听器、一元 `/api` 载体、WebSocket 下行和 `GuiHandler`。若没有捆绑的 YAML 图与宿主插件注册，就不会作为组合好的 web 应用监听，也不会打印就绪 URL。通过依赖 `dsh-headless` 来复制 `HeadlessIo`，会把 GUI 宿主耦合到一次性 runner。把 `register_host_plugins` 放到 `dsh-cli` 后面，则 `dsh-host` 无法在测试中自行启动。

## 决策

`dsh-host` 持有 `WEB_YAML`（`web.cordis.yml`）和 `register_host_plugins`。该 YAML 是第 6 阶段无头 base 行去掉 `headless-startup`、`headless-runner`、`headless-auto-approve` 与 `sdk-jsonrpc-server`，再加上 workspace、settings、commands、`@deepseek-ai/dsh-host-webserver`（`127.0.0.1:3080`）、`@deepseek-ai/dsh-host-frontend-static`、`@deepseek-ai/dsh-client-modules` 与 `web-app`（`printUrl: true`）。没有 `!!js`。用户审批 `policy` 为 `ask`。保留 mock LLM（大语言模型）行以便无密钥启动。

`register_host_plugins` 只注册这七个名字。它不调用 `register_spine_plugins`、`register_execution_plugins` 或 `register_base_plugins`。`dsh-host` 不依赖 `dsh-cli` 或 `dsh-headless`。`dsh-headless` 不依赖 `dsh-host`。`dsh-agent` 不依赖 `dsh-host`。

stdout 捕获是 `dsh-host` 中的 `WebIo`（`stdio`、`capture`、`stdout`、`take_stdout`）。`web-app` 在存在时 inject `"webIo"`，否则用 `WebIo::stdio()`。`serve` 之后，当 `printUrl` 为 true（默认）时，它恰好写入 `dsh web: http://127.0.0.1:<bound-port>\n`，并提供 `listeningHost`。host-webserver 提供 `hostBind` 与 `trustedHosts`（字符串数组；省略则为空）；主机不是 `127.0.0.1` 时加载失败。frontend-static 提供 `webDist`，当 `dist` 不是已存在的目录时加载失败。client-modules 提供 `clientPackages`；空目录得到空图。`web-app` 把 `trustedHosts` 注入 `HostState`。

点分映射仍在[第 7 阶段 GUI RpcMethodMap](2026-08-17-rust-gui-rpc-method-map.md)。HTTP/WS 线路冻结仍在[冻结 Rust GUI 宿主的四象限线路](../../proposed/architecture/2026-08-16-rust-gui-host-wire.md)。`dsh web` 不在本 crate 中。

## 测试

`web_yaml_rejects_js_tag_substring` 断言 `WEB_YAML` 不含 `!!js` 以及四个被省略的无头/SDK 名字。`web_app_prints_ready_url_and_serves_index` 用端口 `0`、`printUrl: true`、含 `index.html` 的 dist 以及空的 client-packages 目录启动 spine + execution + base + host，提供 `WebIo::capture()`，然后断言 stdout 含 `dsh web: http://127.0.0.1:`、`GET /` 为 HTTP 200，并且 `listeningHost` 关闭。持久化路径是插件的 `root` / `path` / `dir` 配置，不是进程环境变量。`cargo test -p dsh-host --offline` 保持既有宿主测试。

## 考虑过的替代方案

**依赖 `dsh-headless` 并复用 `HeadlessIo`。** 不予采用：GUI 宿主不得依赖一次性 runner，且 `dsh-headless` 不得依赖 `dsh-host`。

**让 `register_host_plugins` 同时注册 spine、execution 与 base。** 不予采用：CLI 在后续任务中持有该组装；本 crate 的注册器保持七个宿主/产品名字，以便调用方组合更小的图。

**把 `HeadlessIo` 复制进共享 util crate。** 不予采用：紧挨 `web-app` 的一个 `WebIo` 已足够，为两个短类型再加第三个包并不值得。

## 后果

`WEB_YAML` 省略 frontend-static 的 `dist` 与 client-modules 的 `dir`，因此不带这些配置启动捆绑文件会加载失败。这是明确失败，直到像[在 Rust 宿主上运行 dsh web](2026-08-17-rust-dsh-cli-web.md) 这样的调用方提供目录。workspace 与 settings 仍需要 `DSH_HOME` / `DSH_SESSION_ROOT` 或 `path` / `dir` 配置。就绪行是给 supervisor 的 stdout 约定；测试必须捕获 `WebIo`，而不是进程 stdout。
