# Agent Note: Time-context refresh uses process-local wall clock

Status: implemented

English | [中文](2026-08-16-time-context-wall-clock-refresh.zh.md)

## Problem

`crates/dsh-time-context` compared `unix_now_ms()` (epoch milliseconds) with `SessionEvent.time`. LoopAgent stamps `time` as `seq as i64`, not wall clock. A positive `refreshIntervalMs` therefore never skipped (`now - seq` is ~1.7e12 ms), and elapsed text formatted a multi-million-day duration instead of `unavailable`.

The TypeScript plugin can schedule from durable event timestamps because those values are epoch milliseconds ([durable per-step time context](../feature/2026-07-16-durable-per-step-time-context.md)). The Rust host cannot.

## Decision

`dsh-time-context` does not treat log `time` as wall clock. A plugin-local `HashMap` keyed by session id string stores the last wall-clock prepend millisecond, matching token-meter's session-id keys. The map updates only when the listener actually prepends. Omit or `0` for `refreshIntervalMs` still injects at every eligible step.

Elapsed text is `unavailable` when the preceding event is missing or its `time` is below `1_000_000_000_000` (seq-as-time and other non-Unix-ms values). Plausible Unix-ms baselines still use `format_duration`. LoopAgent's seq-as-time convention is unchanged.

## Alternatives considered

**Compare `unix_now_ms()` to `event.time`.** This is the mixed-clock defect: seq is not epoch milliseconds, so a live positive interval never skips and elapsed text is a multi-million-day duration.

**Stamp LoopAgent `time` as epoch milliseconds.** That would restore the TypeScript durable-interval contract, but seq-as-time is a host-wide convention (compaction, session append, repair). This crate does not change it.

**Add chrono or another time crate.** `SystemTime` plus manual UTC formatting already supplies `now_ms` and the timestamp line; a second calendar crate would not change the seq-as-time log.

## Consequences

A positive interval skips a second eligible step at the same mocked `now_ms` and injects again once wall clock advances by the interval, within one process and plugin instance. Resume, restart, or a new plugin instance sees an empty map and injects. Typical harness logs print `unavailable` for elapsed. Package tests pin both behaviors; `format_duration` cases are unchanged.
