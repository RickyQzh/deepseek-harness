# dsh-credentials

English | [中文](README.zh.md)

Resolves POSIX-named credential references per call. Consumers must not cache the secret across operations; a changed environment or memory value is visible on the next `resolve`.

Layers are process environment, then an optional YAML string map, then memory. An empty stored value is absent at every layer. `set_ref` / `unset_ref` write the file layer; when a persist path is set they rewrite that YAML string map (create the parent directory). Empty `set_ref` is unset. Env still wins on `resolve`; env-shadowed `set_ref` / `unset_ref` fail with `credential-rejected`. `describe_refs` returns `configured` / `source` / `writable` and never includes the secret. Trait `set` with an empty string remains `EmptyValue` and writes memory.

This crate does not watch files or enforce `chmod 600`.

`plugin::register` mounts YAML `@deepseek-ai/dsh-credentials` and provides the `credentials` service as `LayeredCredentials`. When `DSH_HOME` is set, the plugin uses `{DSH_HOME}/credentials.yaml`; otherwise the store is memory-only.

## Known Limitations and Deferred Work

- The TypeScript local provider's chokidar watcher, `$DSH_HOME/.credentials.yaml` filename, and project `.env` files are a later crate. This crate persists `credentials.yaml` under `DSH_HOME`.
