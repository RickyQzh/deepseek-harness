# dsh-credentials

English | [中文](README.zh.md)

Resolves POSIX-named credential references per call. Consumers must not cache the secret across operations; a changed environment or memory value is visible on the next `resolve`.

Layers are process environment, then an optional YAML string map, then memory. An empty stored value is absent at every layer. This crate does not watch files, enforce `chmod 600`, or persist writes.

## Known Limitations and Deferred Work

- The TypeScript local provider's chokidar watcher, home-directory `.credentials.yaml`, and project `.env` files are a later crate. Phase 3 needs env + memory + an injectable YAML map so DeepSeek tests can swap a key without a real `DEEPSEEK_API_KEY`.
