# dsh-boot

English | [中文](README.zh.md)

Closed YAML-name to setup-closure registry and mount for the DeepSeek Harness Rust host. `boot_yaml` parses compose YAML, applies `--patch` documents, interpolates each row's `config`, and mounts registered plugins as kernel fibers. An unknown YAML `name` is a load error before that row is spawned. `!!js` is still rejected by compose parse and is never evaluated.

This crate does not register product plugins. Callers `register` setups on a `PluginRegistry` by YAML `name`. Spine composition is `dsh_agent::register_spine_plugins`. Execution composition is `dsh_agent::register_execution_plugins`.

## Known Limitations and Deferred Work

- Product plugin setups are not in this crate; `dsh_agent::register_spine_plugins` owns the spine YAML names and `dsh_agent::register_execution_plugins` owns the execution YAML names.
