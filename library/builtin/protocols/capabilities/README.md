# Capability 协议

通过 trait 协议与 elaboration 连接 capability，不引入函数签名 `handles` 关键字。

- `errors.HandleError<E, R>`：错误恢复能力入口 `fn handle_error(*self, value: E) -> R`，用于 `.use(handler)` 错误处理，保持错误结果 ABI。
- `effects.HandleEffect<Args, Output>`：直接效应能力入口 `fn handle_effect(*self, args: Args) -> Output`。
- `effects.HandleCapturingEffect<Args, BodyOutput>`：可捕获效应能力入口 `fn handle_capturing_effect(*self, args: Args, continuation: Continuation) -> BodyOutput`，接收仿射（非 `Copy`/`Clone`，仅 `Once`）限定续体 `builtin.Continuation`。

运行时策略与 stack pool 封装属于 `std.runtime`。
