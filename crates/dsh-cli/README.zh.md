# dsh-cli

[English](README.md) | 中文

DeepSeek Harness Rust 宿主的 `dsh` clap 启动器。

`--profile` 为必填。当前仅实现 `headless`。调用形式为 `dsh --profile headless [--patch <file> ...] <task>`。可重复的 `--patch` 文件是按 argv 顺序传给 `boot_yaml` 的 UTF-8 YAML 文档。argv[1] 为 `web` 或 `plugin`，以及任何未实现的 `--profile`，都会向 stderr 打印 `dsh: {verb} is not implemented` 并以退出码 2 退出。本二进制不会 spawn Node。启动器调用 `dsh_base::register_base_plugins`，因此第 6 阶段 YAML 名称存在。当 `$DSH_CORDIS_CONFIG` 已设置且非空时使用该路径的配置 YAML，否则使用 `dsh-headless` 附带的 `minimal.cordis.yml`。用法错误与未实现失败的退出码为 2；其他启动器失败向 stderr 打印 `dsh: {message}` 并以退出码 1 退出。当且仅当 headless 运行器的 `appExit` 码为 0 时进程退出码为 0。

JSONL 持久化在 `DSH_SESSION_ROOT` 已设置且非空时使用该目录，否则使用 `{DSH_HOME}/sessions`；当两者都未设置时，启动器将 `DSH_HOME` 设为 `$HOME/.dsh`。

## 已知限制与暂缓事项

- 未实现 `standard` profile。
- 不支持 Windows。
