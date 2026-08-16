# dsh-subprocess

English | [中文](README.zh.md)

Fully specified argv spawn and credential-scrubbed child environments for the DeepSeek Harness Rust host. `argv` is never a shell string; a consumer that wants a shell passes `["bash", "-c", command]` itself.

Child env is scrubbed then overlay-merged: `child_env` starts from `scrubbed_parent_env()` (ambient credential-shaped names and `DSH_*` names dropped), then applies explicit `EnvEntry` values. `None` is a tombstone that removes an ambient key; POSIX last exact key wins.

`spawn_subprocess` starts a POSIX process-group leader (`process_group(0)`), collects bounded tails with optional spill files, and terminates with SIGTERM to `-pid` then SIGKILL after `grace_ms`. `LocalSubprocessRuntime` resolves bare names on the scrubbed `PATH` and disposes live trees by terminating each group and awaiting `wait_for_exit`.

## Known Limitations and Deferred Work

- PTY allocation is out of this crate.
- Windows `taskkill /T` tree kill is deferred; non-unix `spawn_subprocess` returns `UnsupportedPlatform`.
- `argv` is never a shell string.
