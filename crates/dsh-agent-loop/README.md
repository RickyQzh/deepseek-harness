# dsh-agent-loop

English | [中文](README.zh.md)

Scripted agent driver for the Rust host: durable inbox splices, the idle / maintenance / running phase machine, runtime-context snapshot identity, and request reconstruction from `derive_messages` plus `request/header`.

`followup` / `steer` / `inject` / `cancel` only mutate the inbox and abort flag. `run_until_idle` is the turn driver. A turn always appends `turn/start` before the first claim; an empty first enter still logs `turn/end` completed without a step.

## Known Limitations and Deferred Work

- Factory `create` / `resume`, agent registry, and kernel plugin wrapping are Phase 5. This crate owns `LoopAgent` directly. Bash / fs tools are Phase 4; tests register mock tools.
