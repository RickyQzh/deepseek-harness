# dsh-subprocess

English | [中文](README.zh.md)

Fully specified argv spawn and credential-scrubbed child environments for the DeepSeek Harness Rust host. `argv` is never a shell string; a consumer that wants a shell passes `["bash", "-c", command]` itself.

Child env is scrubbed then overlay-merged: `child_env` starts from `scrubbed_parent_env()` (ambient credential-shaped names and `DSH_*` names dropped), then applies explicit `EnvEntry` values. `None` is a tombstone that removes an ambient key; POSIX last exact key wins.

`spawn_subprocess` starts a POSIX process-group leader (`process_group(0)`), collects bounded tails with optional spill files, and terminates with SIGTERM to `-pid` then SIGKILL after `grace_ms`. After the direct child exits, collect-mode `done()` waits at most `grace_ms` for pipe EOF, then drops the collect readers so an inherited descriptor cannot hang the outcome. `LocalSubprocessRuntime` resolves bare names on the scrubbed `PATH` and disposes live trees by terminating each group and awaiting `wait_for_exit`.

`spawn_terminal` opens a POSIX PTY with `portable-pty` (`PtySize` rows/cols, pixel sizes 0), spawns `argv` with cwd and `child_env` after `env_clear`, writes bytes to the master, and publishes UTF-8 lossy `String` chunks on a `tokio::sync::broadcast` channel. `done` resolves when the top-level PTY child exits. Last-handle drop closes the master and signals the child. `LocalSubprocessRuntime::spawn_terminal` retains clones until `dispose` drops them; `dispose` does not wait for PTY children.

`create_process_inspector` inspects Linux `/proc` (x86_64 and aarch64 syscall tables) and macOS `ps` for foreground PGID, stdin-wait, children-first process trees, and PID+start identity. `read(0)` is a stdin wait; unreadable `/proc/<pid>/mem` is not. macOS `is_stdin_waiting` is always false. Inspect, signal, and terminate are not methods on `SubprocessTerminalHandle`.

`plugin::register` mounts YAML `@deepseek-ai/dsh-subprocess-local` and provides `subprocess` as `LocalSubprocessRuntime`.

## Known Limitations and Deferred Work

- Windows ConPTY is out; non-unix `spawn_terminal` returns `UnsupportedPlatform`.
- PTY inspect, signal, and terminate are not methods on `SubprocessTerminalHandle`. `dispose` does not wait for PTY children.
- Windows `taskkill /T` tree kill is deferred; non-unix `spawn_subprocess` returns `UnsupportedPlatform`.
- `argv` is never a shell string.
