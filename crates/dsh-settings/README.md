# dsh-settings

English | [中文](README.zh.md)

User-settings namespaces for the DeepSeek Harness Rust host. This crate exposes only `ui-onboarding` and does not serve HTTP.

`SettingsService::with_dir` persists `{dir}/{ns}.json` as `{ "revision": <u64>, "value": <object> }`. A missing `ui-onboarding.json` describes as `{ "welcomeNoticeVersion": "" }` at revision 0. The first successful write increments to 1 and writes the file. Writes use a same-directory temp file plus rename. In-process callers are serialized with a `Mutex`.

`update` merges object keys recursively (non-objects overwrite). `replace` stores the section wholesale; the section must be a JSON object. `mutate` applies ordered path ops: empty-path `set` replaces the root (must be an object); empty-path `unset` resets to the default document. `expected: Some(n)` that is not the current revision is `settings-conflict` with details `{ ns, expected, actual }`. `expected: None` does not check. An unknown namespace is `settings-not-exposed` with details `{ ns }`. A non-object patch or section is `settings-rejected`.

`plugin::register` mounts YAML `@deepseek-ai/dsh-settings` and provides `settings`. Config is optional `{ "dir": "<string>" }`. When `dir` is omitted, `{DSH_HOME}/settings` if `DSH_HOME` is set. Load fails if neither is set, or if `dir` cannot be created. Tests use `with_dir`.

`SettingsError::code` is the kebab-case wire string; `rpc_code` maps to `dsh_rpc::RpcErrorCode`. This crate does not port schemastery or secret redaction.

## Known Limitations and Deferred Work

- Unary `settings.*` RPC and host plugin wiring live outside this crate; `dsh-base` and `dsh-cli` do not register this plugin.
- Only `ui-onboarding` is exposed. Schema validation and secret-slot redaction are not implemented.
