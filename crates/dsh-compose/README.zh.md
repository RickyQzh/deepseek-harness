# dsh-compose

[English](README.md) | 中文

Rust 宿主的封闭 YAML 组合方言。插值器为 `${env:VAR}`、`${env:VAR:-default}`、`${cwd}`、`${dshHome:rel}` 和 `${platform}`。缺失的指称失败即响。YAML 中的 `!!js` 是加载错误，不会被求值。

`apply_entry_patches` 克隆输入，为 id 建索引（包括同一列表中先插入的行），按 id 整份替换配置，并在 id 缺失时失败即响。`compose_named_layers` 从空根开始，按调用方给出的顺序应用 bundle 层、用户层、然后 `--patch` overlay。`disabled: { platform: windows }` 与 `disabled: { not_platform: windows }` 取代 `!!js process.platform` 判断。

补丁应用、层序和 `disabled` 谓词在本 crate 中。`dump_config` 打印组合后的树，插值器保持未求值。

## 已知限制与暂缓事项

- 本 crate 不挂载 fiber。把组合后的行加载进 `dsh-kernel` 是后续阶段。
- 插值器和 `disabled` 谓词使用的平台字符串是 `linux`、`macos` 和 `windows`（不是 Node 的 `win32` / `darwin`）。
