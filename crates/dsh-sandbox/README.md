# dsh-sandbox

English | [中文](README.zh.md)

Fail-closed sandbox modes, writable roots, escalation, and local confine wrapping for the DeepSeek Harness Rust host. `SandboxMode` is kebab-case (`read-only`, `workspace-write`, `danger-full-access`) and names file effects only.

`writable_roots` is empty under `read-only` and `danger-full-access`; under `workspace-write` it is the deduplicated canonical set of the policy workspace root, `/tmp`, and the platform temp directory. `canonical_path` uses `std::fs::canonicalize` and returns the input spelling when resolution fails.

`LocalSandboxProvider::confine` wraps argv in bwrap, Landlock, or Seatbelt and never returns the original argv. Linux probes bwrap then Landlock; Darwin selects Seatbelt as its sole candidate without probing. An empty chain or every unusable probe is `SANDBOX_UNAVAILABLE`. The Landlock launcher path is resolved from the repo layout (`native/landlock-run/packages/linux-{arch}/bin/landlock-run`, then the matching `node_modules` package) and is never taken from the process environment. Windows ACL is not selected.

`validate_escalation_args` requires `sandbox_permissions` and a non-empty justification together. `approve_escalation` fails closed in order: not strictly wider, no approval service, no agent, then the approver outcome. `SandboxPolicy::confined` rejects `danger-full-access`. A missing backend is `SANDBOX_UNAVAILABLE` with the TypeScript refusal paragraph (Windows ACL clause included).

Model-visible markers: `[sandbox: file access denied under {mode} mode]` and `[sandbox: escalation available — retry this exact {subject} once with sandbox_permissions (the narrowest wider mode that suffices) + justification; the approval prompt asks the user]`.

## Known Limitations and Deferred Work

- Resolve does not fold `sandbox/mode` session events; the constructor default mode is the resolved mode.
- Windows ACL is not selected; an injected `win32` platform has an empty chain and fails closed.
- Runner selection is cached for the provider lifetime; installing or repairing a runner requires a new provider.
