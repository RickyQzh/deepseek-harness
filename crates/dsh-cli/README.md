# dsh-cli

English | [中文](README.zh.md)

`dsh` clap launcher for the DeepSeek Harness Rust host.

`--profile` is required. Implemented profiles are `headless` and `web`. `dsh web` is an alias for `--profile web`. `plugin` as argv[1], and any unimplemented `--profile`, print `dsh: {verb} is not implemented` to stderr and exit 2. This binary does not spawn Node.

Headless: invoke `dsh --profile headless [--patch <file> ...] <task>`. Repeatable `--patch` files are UTF-8 YAML documents passed to `boot_yaml` in argv order. The launcher calls `dsh_base::register_base_plugins` then `register_headless_plugins`. Config YAML is `$DSH_CORDIS_CONFIG` when that variable is set and non-empty, otherwise the bundled `minimal.cordis.yml` from `dsh-headless`. Exit 0 if and only if the headless runner's `appExit` code is 0.

Web: invoke `dsh web` or `dsh --profile web` with optional `--port` (default 3080; `0` is OS-assigned), `--host 127.0.0.1` (omitted is the same), repeatable `--trusted-host`, `--dist`, and `--patch`. Any other `--host`, including `0.0.0.0`, is a usage error whose message contains `is intentionally not supported yet for safety: it would expose remote code execution to the network; use 127.0.0.1 instead`. Dist is `--dist`, else `$DSH_WEB_DIST`, else `apps/web/dist` relative to cwd; a missing directory fails with a message containing `dist`. Client packages are `$DSH_CLIENT_PACKAGES` when set and non-empty, otherwise an empty temporary directory. Config YAML is `$DSH_CORDIS_CONFIG` when set and non-empty, otherwise `dsh_host::WEB_YAML`. User `--patch` documents apply first; a generated overlay last sets `@deepseek-ai/dsh-host-webserver` `host`/`port`/`trustedHosts`, `@deepseek-ai/dsh-host-frontend-static` `dist`, and `@deepseek-ai/dsh-client-modules` `dir` by plugin `name`. The launcher registers spine, execution, base, and `register_host_plugins`, not headless plugins, provides `appExit`, `cmdlineArgs`, and `webIo`, prints `dsh web: http://127.0.0.1:<bound-port>`, and waits on `appExit`.

Usage and not-implemented failures exit 2; other launcher failures print `dsh: {message}` to stderr and exit 1.

JSONL persist uses `DSH_SESSION_ROOT` if set and non-empty, else `{DSH_HOME}/sessions`; when neither is set the launcher sets `DSH_HOME` to `$HOME/.dsh`.

## Known Limitations and Deferred Work

- A `standard` profile is not implemented.
- `--host 0.0.0.0` is not supported.
- Windows is not supported.
