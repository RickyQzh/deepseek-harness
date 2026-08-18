# dsh-credentials

[English](README.md) | 中文

按次解析 POSIX 命名的凭据引用。消费者不得跨操作缓存密钥；环境或内存值的变更会在下一次 `resolve` 时可见。

层顺序为进程环境、可选的 YAML 字符串映射，然后是内存。每一层中空的已存值视为缺失。`set_ref` / `unset_ref` 写入 file 层；在设置了持久化路径时会重写该 YAML 字符串映射（必要时创建父目录）。空的 `set_ref` 视为 unset。`resolve` 时环境层仍然优先；被环境遮蔽的 `set_ref` / `unset_ref` 以 `credential-rejected` 失败。`describe_refs` 返回 `configured` / `source` / `writable`，从不包含密钥。trait `set` 遇到空字符串仍是 `EmptyValue`，并写入内存层。

本 crate 不监视文件，也不强制 `chmod 600`。

`plugin::register` 挂载 YAML `@deepseek-ai/dsh-credentials` 并以 `LayeredCredentials` 提供 `credentials` 服务。若设置了 `DSH_HOME`，插件使用 `{DSH_HOME}/credentials.yaml`；否则该存储仅为内存。

## 已知限制与暂缓事项

- TypeScript 本地提供者的 chokidar 监视器、`$DSH_HOME/.credentials.yaml` 文件名以及项目 `.env` 文件属于后续 crate。本 crate 把 `credentials.yaml` 持久化到 `DSH_HOME` 下。
