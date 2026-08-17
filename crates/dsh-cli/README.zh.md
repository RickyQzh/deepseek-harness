# dsh-cli

[English](README.md) | 中文

DeepSeek Harness Rust 宿主的 `dsh` clap 启动器。

`--profile` 为必填。已实现的 profile 为 `headless`、`web` 与 `acp`。`dsh web` 是 `--profile web` 的别名。`dsh acp` 是 `--profile acp` 的别名。argv[1] 为 `plugin`，以及任何未实现的 `--profile`，都会向 stderr 打印 `dsh: {verb} is not implemented` 并以退出码 2 退出。本二进制不会 spawn Node。

无头：调用形式为 `dsh --profile headless [--patch <file> ...] <task>`。可重复的 `--patch` 文件是按 argv 顺序传给 `boot_yaml` 的 UTF-8 YAML 文档。启动器调用 `dsh_base::register_base_plugins`，再调用 `register_headless_plugins`。当 `$DSH_CORDIS_CONFIG` 已设置且非空时使用该路径的配置 YAML，否则使用 `dsh-headless` 附带的 `minimal.cordis.yml`。当且仅当 headless 运行器的 `appExit` 码为 0 时进程退出码为 0。

Web：调用 `dsh web` 或 `dsh --profile web`，可选 `--port`（默认 3080；`0` 由操作系统分配）、`--host 127.0.0.1`（省略等同）、可重复的 `--trusted-host`、`--dist` 与 `--patch`。任何其他 `--host`（包括 `0.0.0.0`）都是用法错误，其消息包含 `is intentionally not supported yet for safety: it would expose remote code execution to the network; use 127.0.0.1 instead`。dist 取 `--dist`，否则 `$DSH_WEB_DIST`，否则相对于 cwd 的 `apps/web/dist`；目录不存在时失败，消息含 `dist`。客户端包目录在 `$DSH_CLIENT_PACKAGES` 已设置且非空时用该路径，否则用空的临时目录。当 `$DSH_CORDIS_CONFIG` 已设置且非空时使用该路径的配置 YAML，否则使用 `dsh_host::WEB_YAML`。用户 `--patch` 文档先应用；最后一份生成的 overlay 按插件 `name` 设置 `@deepseek-ai/dsh-host-webserver` 的 `host`/`port`/`trustedHosts`、`@deepseek-ai/dsh-host-frontend-static` 的 `dist`，以及 `@deepseek-ai/dsh-client-modules` 的 `dir`。启动器注册 spine、execution、base 与 `register_host_plugins`，不注册 headless 插件，提供 `appExit`、`cmdlineArgs` 与 `webIo`，打印 `dsh web: http://127.0.0.1:<bound-port>`，并等待 `appExit`。

ACP：调用 `dsh acp` 或 `dsh --profile acp`，可选可重复的 `--patch`。多余的位置参数是用法错误（`acp takes no task argument`）。`--port`、`--dist`、`--host` 与 `--trusted-host` 会被忽略；ACP 不绑定 TCP 端口。当 `$DSH_CORDIS_CONFIG` 已设置且非空时使用该路径的配置 YAML，否则使用 `dsh-acp` 附带的 `ACP_YAML`（`acp.cordis.yml`）。启动器注册 spine、execution、base 与 `dsh_acp::register_acp_plugins`，不注册 headless 或 host 插件，注入 `acpServer`，并在 stdio 上提供 NDJSON JSON-RPC 直到 EOF。诊断信息写到 stderr；stdout 只承载 ACP 帧。

用法错误与未实现失败的退出码为 2；其他启动器失败向 stderr 打印 `dsh: {message}` 并以退出码 1 退出。

JSONL 持久化在 `DSH_SESSION_ROOT` 已设置且非空时使用该目录，否则使用 `{DSH_HOME}/sessions`；当两者都未设置时，启动器将 `DSH_HOME` 设为 `$HOME/.dsh`。

## 已知限制与暂缓事项

- 未实现 `standard` profile。
- 不支持 `--host 0.0.0.0`。
- 不支持 Windows。
