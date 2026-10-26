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

## 局部长度读取的 VM 性能

已准入的局部列表可以直接观察其长度，不再为每次 `ListLength(Local)`
复制全部元素或重复验证元素内容。普通局部值与已提升为 storage 的局部值
采用同一规则；存储使用现有的受检锁访问。读取仍检查列表头与元素类型身份，
并计入类型描述比较的工作预算，不把复杂的元素类型当成免费比较。

此路径限于 comptime 且没有 cleanup schedule 的局部读取。带计算的操作数
保留完整求值与验证；赋值更新后的局部值也读取当前长度。调用入口的参数
准入、普通参数交付和其他快照成本没有移除，不能据此声称源码中的任意
`meta.length(values)` 调用都已成为与数据规模无关的操作。

基准 `compiler/comptime_interpretor/benches/list_length.rs` 覆盖 Bool/Type
元素、0/16/4096 元素、1/64 次读取，以及局部读取与块表达式两条路径。
包构造、解析、elaboration 和 LLVM 不在计时区；每轮 invocation 的输入
构造与准入仍在计时区。每个用例预热 5 次，每进程运行 100 次，再取 5 个
独立进程的中位数。分配计数包含 alloc/realloc，字节为累计请求量而非峰值。

```sh
python3 -B development/comptime_bench.py --bench list_length --iterations 100 --repeat 5
```

4096 元素、每 invocation 读取 64 次的局部路径结果：

| 元素 | 时间，ms/invocation | 分配次数/invocation | 请求字节/invocation | VM steps/invocation |
| --- | --- | --- | --- | --- |
| Bool | 5.991 → 0.328 | 2562.04 → 642.04 | 74323456 → 2940416 | 1079382 → 29462 |
| Type | 9.273 → 0.253 | 1050.04 → 602.04 | 15719440 → 1006096 | 1332554 → 21258 |

作为对照，块表达式路径的分配次数、请求字节及 steps 前后完全一致；时间
分别为 Bool 的 5.975 → 5.847 ms、Type 的 9.301 → 9.379 ms。这里不承诺
通用路径提速，也不将合成用例的比例推广为整个编译过程的加速比例。

基线来自 `37b4b95a` 的 VM 加入同一基准后的测量。5 份结果目录为
`/tmp/luna-tools-kdbpwboo`、`/tmp/luna-tools-bqcmuhdc`、
`/tmp/luna-tools-mcqixyq0`、`/tmp/luna-tools-s9fyni20`、
`/tmp/luna-tools-qwwlpz0_`；最终版对应
`/tmp/luna-tools-3jcgyn1r`、`/tmp/luna-tools-cjx7o1tz`、
`/tmp/luna-tools-sc246r33`、`/tmp/luna-tools-avqemhia`、
`/tmp/luna-tools-aa4g0egv`。工作目录均为 `/home/fin/luna`，退出 0。

性能回归用例在基线上实际失败：额外 63 次读取的预算由 1 元素时的 2331
增至 4096 元素时的 1034271，报告 `/tmp/luna-tools-qc5tb0qf/result.json`。
优化后，这笔额外预算与元素数量无关；元素类型描述本身变大时仍增加预算。
测试同时检查空列表、storage、替换、一般表达式的赋值副作用、错误元素域
及失败后的继续调用。

最终检查命令退出 0，25 项测试通过；报告
`/tmp/luna-tools-il4htieu/result.json`。选定 Rust 文件的格式检查及
`git diff --check` 均通过。

```sh
python3 -B development/check.py --timeout 1800 -- test --release -p comptime_interpretor --test type_list_inputs --test type_list_index -p luna --test comptime_list_operations
```
