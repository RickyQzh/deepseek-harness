# dsh-subprocess

English | [中文](README.zh.md)

Fully specified argv spawn and credential-scrubbed child environments for the DeepSeek Harness Rust host. `argv` is never a shell string; a consumer that wants a shell passes `["bash", "-c", command]` itself.

Child env is scrubbed then overlay-merged: `child_env` starts from `scrubbed_parent_env()` (ambient credential-shaped names and `DSH_*` names dropped), then applies explicit `EnvEntry` values. `None` is a tombstone that removes an ambient key; POSIX last exact key wins.

## Known Limitations and Deferred Work

- PTY allocation is out of this crate.
- POSIX process groups land in Task 32.
- `argv` is never a shell string.
- Windows `taskkill` is out.
