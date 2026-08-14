# dsh-kernel

[English](README.md) | 中文

为 Rust 宿主提供 Context、Fiber 生命周期、具名服务、effect、isolate realm 以及进程内事件总线。本 crate 保留 Cordis 语义（具名服务、inject 等待、逆序 dispose、waterfall 的 `next()`、isolate realm），不实现 Proxy、声明合并或 `!!js`。

Fiber 状态为 PENDING → LOADING → ACTIVE | FAILED | UNLOADING → DISPOSED。`Context::effect` 登记异步 disposer；卸载与失败的 setup 按登记的逆序运行清理，fiber 处于 `Unloading` 时拒绝新的 `effect`。`Context::plugin` 挂载子 fiber；`await_ready` 在 Active 上结束，或返回 setup 错误。`Context::provide` 以 `(realm, name)` 发布具名服务并登记移除该槽的清理；同一 realm 再次 `provide` 同名返回 `ServiceAlreadyProvided`。`get` 在服务已存在且类型匹配时返回该值；`inject` 等到该读取成功，或本 fiber 进入 `Failed`、`Unloading` 或 `Disposed`。`plugin_injecting` 在所列名称全部就绪前保持 `Pending`，然后运行 setup。事件总线在本 crate 实现；`dsh-events` 对其再导出。

## 已知限制与暂缓事项

- v1 插件是 profile manifest 列出的受信任同进程 Rust crate；本 crate 不加载 cdylib。
- 日志由调用方负责；本 crate 不依赖 `tracing`。
