# dsh-compose

English | [中文](README.zh.md)

Closed YAML compose dialect for the Rust host. Interpolators are `${env:VAR}`, `${env:VAR:-default}`, `${cwd}`, `${dshHome:rel}`, and `${platform}`. A missing referent fails loud. `!!js` in YAML is a load error and is never evaluated.

`apply_entry_patches` clones the input, indexes ids (including rows inserted earlier in the same list), whole-config-replaces by id, and fails loud on a missing id. `compose_named_layers` starts from an empty root and applies bundle layers, then the user layer, then `--patch` overlays, in that caller-supplied order. `disabled: { platform: windows }` and `disabled: { not_platform: windows }` replace `!!js process.platform` checks.

Patch apply, layer order, and `disabled` predicates live in this crate. `dump_config` prints the composed tree with interpolators left unevaluated.

## Known Limitations and Deferred Work

- This crate does not mount fibers. Loading composed rows into `dsh-kernel` is a later phase.
- Platform strings used by interpolators and `disabled` predicates are `linux`, `macos`, and `windows` (not Node's `win32` / `darwin`).
