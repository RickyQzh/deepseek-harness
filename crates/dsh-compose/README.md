# dsh-compose

English | [中文](README.zh.md)

Closed YAML compose dialect for the Rust host. Interpolators are `${env:VAR}`, `${env:VAR:-default}`, `${cwd}`, `${dshHome:rel}`, and `${platform}`. A missing referent fails loud. `!!js` in YAML is a load error and is never evaluated.

Patch apply, layer order, and `disabled` predicates live in this crate. `dump_config` prints the composed tree with interpolators left unevaluated.

## Known Limitations and Deferred Work

- This crate does not mount fibers. Loading composed rows into `dsh-kernel` is a later phase.
- Platform strings used by interpolators and `disabled` predicates are `linux`, `macos`, and `windows` (not Node's `win32` / `darwin`).
