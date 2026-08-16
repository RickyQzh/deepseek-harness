# dsh-cli

English | [中文](README.zh.md)

`dsh` clap launcher for the DeepSeek Harness Rust host.

`--profile` is required. Only `headless` is implemented. Invoke `dsh --profile headless [--patch <file> ...] <task>`. Repeatable `--patch` files are UTF-8 YAML documents passed to `boot_yaml` in argv order. `web` or `plugin` as argv[1], and any unimplemented `--profile`, print `dsh: {verb} is not implemented` to stderr and exit 2. This binary does not spawn Node. The launcher calls `dsh_base::register_base_plugins` so Phase 6 YAML names exist. Config YAML is `$DSH_CORDIS_CONFIG` when that variable is set and non-empty, otherwise the bundled `minimal.cordis.yml` from `dsh-headless`. Usage and not-implemented failures exit 2; other launcher failures print `dsh: {message}` to stderr and exit 1. Exit 0 if and only if the headless runner's `appExit` code is 0.

JSONL persist uses `DSH_SESSION_ROOT` if set and non-empty, else `{DSH_HOME}/sessions`; when neither is set the launcher sets `DSH_HOME` to `$HOME/.dsh`.

## Known Limitations and Deferred Work

- A `standard` profile is not implemented.
- Windows is not supported.
