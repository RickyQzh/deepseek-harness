# dsh-events

English | [中文](README.zh.md)

Re-export of the in-process event bus implemented in `dsh-kernel`. Use `Context::emit`, `serial`, `parallel`, and `waterfall`. A waterfall listener must call `next(value).await` to delegate; returning without `next` short-circuits.

The bus is not a second crate implementation. It lives in `dsh-kernel` so listener registration can be a fiber effect and so isolate/scope filters can read `Context` without a dependency cycle.

## Known Limitations and Deferred Work

- There is no TypeScript declaration-merging event map; first-party product events arrive in a later phase as a closed serde enum.
