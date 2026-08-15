# dsh-credentials

[English](README.md) | 中文

按次解析 POSIX 命名的凭据引用。消费者不得跨操作缓存密钥；环境或内存值的变更会在下一次 `resolve` 时可见。

层顺序为进程环境、可选的 YAML 字符串映射，然后是内存。每一层中空的已存值视为缺失。本 crate 不监视文件、不强制 `chmod 600`，也不持久化写入。

## 已知限制与暂缓事项

- TypeScript 本地提供者的 chokidar 监视器、主目录 `.credentials.yaml` 以及项目 `.env` 文件属于后续 crate。第 3 阶段需要环境 + 内存 + 可注入的 YAML 映射，以便 DeepSeek 测试在没有真实 `DEEPSEEK_API_KEY` 的情况下替换密钥。
