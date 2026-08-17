# dsh-terminal-bash

English | [中文](README.zh.md)

Interactive bash PTY backend for the DeepSeek Harness Rust host. `register` mounts YAML `@deepseek-ai/dsh-terminal-bash` (`dsh_boot::PLUGIN_TERMINAL_BASH`), injects `terminals` as `Mutex<TerminalSessionService>`, `subprocess` as `LocalSubprocessRuntime`, and `sandboxPolicy` as `SandboxPolicyResolver`, then registers the configured backend type (default `shell`). Spawn argv is `[shellPath, ...shellArgs]` under `danger-full-access`. Any other resolved mode looks up optional `sandbox` as `LocalSandboxProvider` at spawn and wraps through `confine`. A missing provider fails before `spawn_terminal` with `terminal-bash: sandbox mode "{mode}" requires a ctx.sandbox provider in the execution world`. The backend does not depend on `dsh-agent`, `dsh-acp`, `dsh-host`, `dsh-cli`, `dsh-headless`, `dsh-mcp-client`, or `dsh-base`. This crate is not mounted in `base.cordis.yml`.

Unknown config keys fail load (`TerminalBashConfig: unknown key "{key}"`). Defaults: `backendType` `shell`, `shellPath` `/bin/bash`, `shellArgs` `--noprofile --norc -i`, `rows` 40, `cols` 160, `scrollbackLines` 10000, `scrollbackMaxBytes` 4194304, `maxReadBytes` 262144, `pollIntervalMs` 50, `exactProbeAfterMs` 150, `idleSilenceMs` 3000, `handoffGraceMs` 500, `timeoutMs` 30000, `disposeGraceMs` 3000. Empty `backendType` or `shellPath` fails with `terminal-bash: backendType must be non-empty` / `terminal-bash: shellPath must be non-empty`. A non-positive or non-integer numeric field fails with `terminal-bash: {name} must be a positive safe integer`. `maxReadBytes` above `scrollbackMaxBytes` fails with `terminal-bash: maxReadBytes must not exceed scrollbackMaxBytes`. `handoffGraceMs` below `pollIntervalMs` fails with `terminal-bash: handoffGraceMs must be at least pollIntervalMs so one readiness poll runs inside the grace window`.

Child env overlays after the subprocess scrubbed parent: `TERM=dumb`, `PAGER=cat`, `GIT_PAGER=cat`, `PS1=dsh> ` (trailing space), `PROMPT_COMMAND` OSC `133;D;` with the last exit status, `BASH_SILENCE_DEPRECATION_WARNING=1`, `DSH_SHELL=1`, `DSH_SESSION_ID=<owner>`, `DSH_PTY_SESSION_ID=<id>`. Default cwd is the sandbox policy workspace root when spawn omits `cwd`.

Readiness uses a private OSC `133;D;` marker then a printable tail equal to `dsh> `, discards pre-write evidence at each send (including initialize's empty write), and does not accept zero-output silence while unpublished. After `exactProbeAfterMs`, `inspect_foreground` `input_waiting` may settle `stdin_read` when unpublished startup already has output. The same foreground PGID must leave a pre-write wait and re-enter it; a different PGID may use the current wait. Unknown foreground is never `stdin_read`. Prompt-marker plus `dsh> ` with idle of at least `pollIntervalMs` and the captured `shellPgid` also settles `stdin_read`. Other settlement reasons are `inferred_idle`, `timeout`, and `session_exit`. Timeout rejects spawn with `PTY shell did not reach readiness before startup timeout`. Session exit during initialize is `PTY shell exited during startup`. Cancel delivers `SIGINT` to the foreground process group and never writes `\x03`.

## Model Experience

Indirectly through terminal tool consumers. This backend contributes no tool schema or prompt.

#### KV Cache effect

No direct invalidation.

## Known Limitations and Deferred Work

- Model-facing `terminal_*` tools live in `dsh-tool-terminal`.
