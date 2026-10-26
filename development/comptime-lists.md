# 编译期列表操作

`meta.List<T>` 是保留 normal-form 头的不可变编译期集合，不以结构列表
替代类型身份。`meta.length` 与 `meta.concat` 是 meta 包内的普通
`pure comptime fn`；普通包经公开接口调用，直接内置符号仍受包边界限制。

```flurry
comptime fn ready() -> bool {
    let first: meta.List<bool> = [true, false];
    let joined = meta.concat(first, [true]);
    meta.length(joined) == 3 and joined[0] and not joined[1] and joined[2]
}
```

`length` 返回编译期 Integer。`concat` 接受相同 `List<T>` 类型的两个值，
先求值左实参，再求值右实参，各一次；返回值按左、右的元素顺序连接。
空列表不引入特殊元素类型转换；嵌套列表和 `List<Type>` 遵循同一接口。
返回、错误或预算失败遵循原有求值路径，不为计算长度跳过实参的副作用。
已知 `List<bool>` 可以为参数 `List<T>` 提出 `T = bool` 候选；候选仍须
满足原始类型等式。此元数据只用于受信任 List 头的调用推断，不给 PTC
增加单射公理，也不从普通 opaque 工厂的结果反推输入。两个无上下文的
字面量可显式写 `meta.concat<bool>([true], [false])`。

NIR 分别使用 `(list-length ...)`、`(list-concat ... ...)`，遍历、替换、
复制预算和阶段检查保留子表达式。VM 检查集合身份、元素域与输出类型，
连接前核对总长度及工作预算；连接结果遵守现有 100,000 元素表示上限，也受嵌套结构描述工作量上限约束。
已经准入的两个输入在连接后超过后者时，报告表示大小限制，而非类型错误。
IIR 的稳定值折叠只接受已完成的列表数据，保留其完整类型，并计入折叠预算。

源码验收位于 `compiler/luna/tests/comptime_list_operations.rs`；低层边界
分别在 `comptime_interpretor/tests/type_list_inputs.rs` 和
`iir/tests/comptime_lists.rs`。本能力不代表完整 sysroot 或 List 库已经完成；
切片、迭代与更多集合接口仍需后续实现。

## 本阶段验收

工作目录 `/home/fin/luna`。以下命令退出 0，290 项测试通过；
报告为 `/tmp/luna-tools-phghacti/result.json`。

```sh
python3 -B development/check.py --timeout 1800 -- test --release -p nir --test phases --test type_rewrite --test copy_budget -p iir --test types --test rewrite --test comptime_lists -p comptime_interpretor --test type_list_inputs --test type_list_index -p elaboration --test calls --test ptc -p luna --test comptime_list_operations --test library_protocols --test library_meta_objects --no-fail-fast
```

公开 package 用例通过 `--invoke-comptime` 调用。另一源码用例显式绑定
Splice 后在运行时使用已计算的 bool；没有为列表操作跳过跨阶段交付协议。
