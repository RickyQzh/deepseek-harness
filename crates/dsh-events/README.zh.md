# dsh-events

[English](README.md) | 中文

对 `dsh-kernel` 中进程内事件总线的再导出。使用 `Context::emit`、`serial`、`parallel` 和 `waterfall`。waterfall 监听器必须调用 `next(value).await` 才能把控制交给下一环；不调用 `next` 即短路。

总线不是第二套实现。它放在 `dsh-kernel` 里，以便把监听器登记做成 fiber effect，并让 isolate/作用域过滤读取 `Context`，同时避免循环依赖。

## 已知限制与暂缓事项

- 没有 TypeScript 声明合并事件表；第一方产品事件在后续阶段以封闭的 serde 枚举落地。
