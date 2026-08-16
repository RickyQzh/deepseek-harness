# dsh-sandbox

English | [中文](README.zh.md)

Fail-closed sandbox modes, writable roots, and escalation for the DeepSeek Harness Rust host. `SandboxMode` is kebab-case (`read-only`, `workspace-write`, `danger-full-access`) and names file effects only.

`writable_roots` is empty under `read-only` and `danger-full-access`; under `workspace-write` it is the deduplicated canonical set of the policy workspace root, `/tmp`, and the platform temp directory. `canonical_path` uses `std::fs::canonicalize` and returns the input spelling when resolution fails.

`validate_escalation_args` requires `sandbox_permissions` and a non-empty justification together. `approve_escalation` fails closed in order: not strictly wider, no approval service, no agent, then the approver outcome. `SandboxPolicy::confined` rejects `danger-full-access`. A missing backend is `SANDBOX_UNAVAILABLE` with the TypeScript refusal paragraph (Windows ACL clause included).

Model-visible markers: `[sandbox: file access denied under {mode} mode]` and `[sandbox: escalation available — retry this exact {subject} once with sandbox_permissions (the narrowest wider mode that suffices) + justification; the approval prompt asks the user]`.

## Known Limitations and Deferred Work

- Confine wrapping, Landlock, bwrap, and Seatbelt profiles are not in this crate.
- Resolve does not fold `sandbox/mode` session events; the constructor default mode is the resolved mode.
- Darwin and Windows runners are not selected here.
