# 编译期数学

scalar_math.fl 是与 builtin.math 对应的 comptime 标量函数，通过 VM intrinsic 执行。公开入口 meta.math；不在 meta.prelude 中导出 math，以免遮蔽 runtime math。当前机器数值域与任意精度运算的边界见 ../math.md。

## 编译期全序

`ComptimeOrd :- ComptimeEq` 由 `meta` 和默认 `meta.prelude` 导出。
它与 runtime `Ord` 使用同一个 `builtin.Order`（`less`、`equal`、`greater`），
但保留独立协议及实现身份；comptime 方法查找优先采用编译期协议。
`cmp` 提供三向比较，`lt`、`le`、`gt`、`ge` 是依赖 `cmp` 的默认方法。

当前覆盖固定宽度整数、usize／isize、任意精度 Integer、bool、char 和 str。
bool 按 false < true 排序；char 按 Unicode scalar 排序；str 按 UTF-8 字节
做字典序比较，公共前缀相同时短者在前，不做 locale 或 Unicode normalization。
浮点没有默认全序实现，不能把 NaN 的普通比较当成全序。Symbol 及复合容器的
全序尚未提供，用户定义类型可显式实现协议。

```flurry
comptime fn ordered_names() -> List<str> {
    let names: List<str> = ["beta", "alpha"];
    names.sort_by(|left, right| left.lt(right))
}
```

真实 sysroot 验收：`development/library_check.py --stage sysroot-comptime-ordering`。
比较方法使用普通 comptime 调用与预算，不将比较结果作为类型相等证据。
