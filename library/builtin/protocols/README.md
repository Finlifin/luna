# 基础协议

现有资源（Once/Clone/Copy/Drop）、转换、语法制导 pattern view、运算、迭代和 Hash/Default 协议归这里。协议跨 comptime/runtime 的适用范围由声明确定，不按调用阶段复制 trait。容器实现与分配策略属于 std。

## 可失败的迭代消费

`Iterator.try_fold(initial, f)` 按生产顺序将累积值和当前元素交给
`f: fn(A, Item) -> !E A`。成功值成为下一轮累积值；首个错误立即结束
这次消费，按普通错误结果 ABI 返回 `!E A`。空输入直接成功返回 initial。
`Iterator.try_foreach(f)` 接收 `fn(Item) -> !E void`，同样在首个错误处
停止，空输入成功。两个接口均支持带环境的回调。

```flurry
fn require_true(previous: bool, item: bool) -> !bool bool {
    if not item {return error false;}
    previous and item
}

fn check_items() -> !bool bool {
    let first: ?bool = true;
    let second: ?bool = false;
    first.as(Iterator).chain(second).as(Iterator).try_fold(true, require_true)
}
```

这两个方法使用 inline fn 与 emit_item handler 联合展开。错误是普通返回值，
不会作为代数效应重新派发。失败后不再调用下游回调，也不继续生产后续元素；
未生产的源资源及回调捕获仍须清理。累积值、元素和错误载荷都可为仿射值，
接口不要求它们实现 Copy 或 Clone。回调已经接收的资源遵守其普通调用责任，
返回给调用者的错误载荷不在消费方法内部提前析构。

真实 sysroot 验收入口：
`python3 -B development/library_check.py --stage sysroot-fallible-iteration --timeout 300`。
