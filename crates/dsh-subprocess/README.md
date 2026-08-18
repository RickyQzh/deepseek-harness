# dsh-subprocess

English | [中文](README.zh.md)

Fully specified argv spawn and credential-scrubbed child environments for the DeepSeek Harness Rust host. `argv` is never a shell string; a consumer that wants a shell passes `["bash", "-c", command]` itself.

Child env is scrubbed then overlay-merged: `child_env` starts from `scrubbed_parent_env()` (ambient credential-shaped names and `DSH_*` names dropped), then applies explicit `EnvEntry` values. `None` is a tombstone that removes an ambient key; POSIX last exact key wins.

`spawn_subprocess` starts a POSIX process-group leader (`process_group(0)`), collects bounded tails with optional spill files, and terminates with SIGTERM to `-pid` then SIGKILL after `grace_ms`. `Pipe` stdin and stdout dispositions return live `ChildStdin` and `ChildStdout` from one-shot `take_stdin` and `take_stdout`; a missing Pipe disposition returns `None`. After the direct child exits, collect-mode `done()` waits at most `grace_ms` for pipe EOF, then drops the collect readers so an inherited descriptor cannot hang the outcome. `LocalSubprocessRuntime` resolves bare names on the scrubbed `PATH` and disposes live trees by terminating each group and awaiting `wait_for_exit`.

`spawn_terminal` opens a POSIX PTY with `portable-pty` (`PtySize` rows/cols, pixel sizes 0), spawns `argv` with cwd and `child_env` after `env_clear`, attaches `create_process_inspector`, writes bytes to the master, and publishes UTF-8 lossy `String` chunks on a `tokio::sync::broadcast` channel. `done` resolves when the top-level PTY child exits. `inspect_foreground`, `signal_foreground`, and `terminate` are methods on `SubprocessTerminalHandle`. `inspect_foreground` returns `Ok(None)` when PGID is missing. `signal_foreground` delivers a real group signal via the inspector (never a PTY write of `\x03`) and refuses `SIGKILL` of the shell pid with `refusing to SIGKILL the terminal shell; terminate the terminal session instead`. `terminate` copies TypeScript `closeOnce`: snapshot descendants with PID+start fences, SIGTERM, wait `grace_ms` (25 ms poll), SIGKILL survivors, then SIGTERM/SIGKILL the shell; Linux zombies count as quiescent. Last-handle drop closes the master and signals the child unless `done` already recorded the exit or `terminate` already ran. `LocalSubprocessRuntime::spawn_terminal` retains clones until `dispose` awaits `terminate` on each.

`create_process_inspector` inspects Linux `/proc` (x86_64 and aarch64 syscall tables) and macOS `ps` for foreground PGID, stdin-wait, children-first process trees, and PID+start identity. `read(0)` is a stdin wait; unreadable `/proc/<pid>/mem` is not. macOS `is_stdin_waiting` is always false.

`plugin::register` mounts YAML `@deepseek-ai/dsh-subprocess-local` and provides `subprocess` as `LocalSubprocessRuntime`.

## Known Limitations and Deferred Work

- Windows ConPTY is out; non-unix `spawn_terminal` returns `UnsupportedPlatform`.
- Synchronous host-exit `terminateForHostExit` is omitted; last-handle Drop remains leak-prevention and skips kill after `done` or `terminate`.
- Windows `taskkill /T` tree kill is deferred; non-unix `spawn_subprocess` returns `UnsupportedPlatform`.
- `argv` is never a shell string.
