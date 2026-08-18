# dsh-settings

[English](README.md) | 中文

DeepSeek Harness Rust 宿主的用户 settings 命名空间。本 crate 只暴露 `ui-onboarding`，不提供 HTTP。

`SettingsService::with_dir` 把 `{dir}/{ns}.json` 持久化为 `{ "revision": <u64>, "value": <object> }`。缺失的 `ui-onboarding.json` 在 revision 0 描述为 `{ "welcomeNoticeVersion": "" }`。第一次成功写入把 revision 增到 1 并写文件。写入使用同目录临时文件再 rename。进程内调用方由 `Mutex` 串行化。

`update` 按对象键递归合并（非对象整值覆盖）。`replace` 整段替换分节；分节必须是 JSON 对象。`mutate` 按顺序应用路径操作：空路径 `set` 替换根对象（必须是对象）；空路径 `unset` 重置为默认文档。`expected: Some(n)` 与当前 revision 不符时是 `settings-conflict`，details 为 `{ ns, expected, actual }`。`expected: None` 不做检查。未知 namespace 是 `settings-not-exposed`，details 为 `{ ns }`。非对象的 patch 或分节是 `settings-rejected`。

`plugin::register` 挂载 YAML `@deepseek-ai/dsh-settings` 并提供 `settings`。配置为可选 `{ "dir": "<string>" }`。省略 `dir` 时，若设置了 `DSH_HOME` 则使用 `{DSH_HOME}/settings`。两者都未给出，或 `dir` 无法创建时，加载失败。测试使用 `with_dir`。

`SettingsError::code` 是 kebab-case 传输字符串；`rpc_code` 映射到 `dsh_rpc::RpcErrorCode`。本 crate 不移植 schemastery，也不做 secret 槽位脱敏。

## 已知限制与暂缓事项

- 一元 `settings.*` RPC 与宿主插件装配不在本 crate；`dsh-base` 与 `dsh-cli` 不注册此插件。
- 只暴露 `ui-onboarding`。未实现 schema 校验与 secret 槽位脱敏。
