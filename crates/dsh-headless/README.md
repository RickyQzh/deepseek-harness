# dsh-headless

English | [中文](README.zh.md)

One-shot headless runner for the DeepSeek Harness Rust host.

`register_headless_plugins` registers YAML names `headless-startup` then `headless-runner`. `headless-startup` reads `cmdlineArgs` and `provide`s `headlessStartup` whose task is the inner argv joined with spaces and whose optional `resumeSessionId` comes from YAML. `headless-runner` injects `agents` and `sessions`, reads `headlessStartup` from `provide` (code), loads then `resume`s when `resumeSessionId` is set (otherwise `create`), drives one user followup, prints the last assistant text plus a newline, and requests exit 0 iff the owned interval's final `turn/end` reason kind is `completed`. Failures write `dsh: {message}\n` to stderr and request exit 1. The launcher must `provide` `appExit` before the tree mounts. Tests may `provide` `headlessIo` to capture streams; when that service is absent the runner writes process stdio. Task text is never a YAML `!!js` field.

## Known Limitations and Deferred Work

- This crate does not compose `dsh-base`.
- Session-title LLM calls are not implemented.
- Persist path is `{DSH_SESSION_ROOT}/{id}/session.jsonl` (not `--<project>--`).
