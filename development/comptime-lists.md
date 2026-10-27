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
遍历组合子、查询、切片与区间替换的后续实现见下文。

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

## 编译期遍历、映射与折叠

meta 包提供普通的源码级组合操作：

| 接口 | 结果与调用规则 |
| --- | --- |
| `meta.foreach(values, visit)` | 按索引顺序调用 `visit`，每个元素一次，返回 void |
| `meta.map(values, transform)` | 按相同顺序构造 `List<U>`，每个元素调用一次 `transform` |
| `meta.fold(values, initial, combine)` | 从 initial 开始左折叠，按 `combine(accumulator, element)` 更新结果 |

回调类型为带环境参数的 `comptime fn`，可以传入命名函数或闭包；普通
runtime fn 不能冒充此阶段的回调。组合函数不声明 pure，因此不会把回调
的外部写入隐藏成纯计算。空输入不调用回调，map 返回具有正确元素域的
空列表，fold 返回传入的 initial；普通实参仍按源顺序求值一次。

```flurry
comptime fn total() -> Integer {
    let values: meta.List<Integer> = [1, 2, 3];
    let doubled = meta.map(values, |value| value * 2);
    meta.fold(doubled, 0, |sum, value| sum + value)
}
```

这里 Integer 使用 builtin.prelude 中的公开名称。total 返回 12。
遍历前只取得一次长度；循环中的索引保持编译期 Integer，适用已接通的
局部列表索引路径。闭包自己的值捕获保留独立状态，重复调用不重建环境；
要观察外部计数器的写入，显式捕获指针。修改私有捕获不会改写调用者的
另一个值副本，也不会改变已经交付给遍历函数的输入序列。

map 可以改变元素域。可选结果仍占据一个列表元素，不自动跳过 null；
嵌套列表不自动展平。映射 Type 的结果可以直接展开到类型工厂的变参输入，
保留 normal-form 头。回调执行失败立即终止当前调用，不访问后续元素；
同一个 VM 在失败后仍可执行其他合法调用。

map 的最初实现逐次调用不可变 concat，存在随输出规模二次增长的复制
成本；后续已改用 append 复用槽位缓冲区，详见下节。foreach 和 fold
不构造额外的结果列表。
后续已补齐编译期 List 指针读写（见下节）。Integer `/` 的舍入协议仍未
接通；失败传播测试使用已支持的列表越界操作。

本阶段在 `/home/fin/luna` 执行以下命令，退出 0，38 项测试通过；文档中的
组合示例也包含在源码验收中。报告 `/tmp/luna-tools-9f9hbu14/result.json`。

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test comptime_list_operations --test library_meta_objects --test library_protocols --no-fail-fast
```


## 编译期列表槽位读写

`ReadPointer` 和 `WritePointer` 在 comptime invocation 中识别已绑定的
List 类型，并核验完整的不可变数据准入条件；普通 runtime 指针传送的
Copy 权限规则不变。读取复制值表示，写入先验证新旧表示，再替换槽位。
类型、元素或预算检查失败不能在验证完成前修改槽位。

```flurry
comptime fn run()->bool {
    var values:meta.List<bool> = [true,false];
    let slot=values.ref;
    let before=slot.*;
    let mapped=meta.map(values,|value| {slot.* = []; not value});
    meta.length(values) == 0 and before[0] and not mapped[0] and mapped[1]
}
```

`map` 的回调可以替换调用者的列表，但其输入序列保持调用时的值；
先前的指针读取也保留独立值。这里没有可变元素视图或 runtime List
复制承诺，输出物化仍执行原有阶段检查。

本阶段在 `/home/fin/luna` 执行以下检查，退出 0，51 项测试通过，报告
`/tmp/luna-tools-u24r_czh/result.json`。覆盖 bool/Type 列表的预算扫描、
拒绝写入后的存储保持及恢复读取、runtime 拒绝、错误域和实际源码的
回调替换、快照与嵌套列表。

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test comptime_list_operations -p comptime_interpretor --test type_list_inputs --test abstract_values --no-fail-fast
```

## 短路查询与筛选

meta 根模块提供以下 comptime 操作。回调类型为 comptime 函数，可携带
有状态闭包；这些接口不声明 pure，因为回调可以通过捕获的指针写入。

| 接口 | 返回值 | 遍历停止条件 | 空输入 |
| --- | --- | --- | --- |
| `any(values, pred)` | bool | 第一个 true | false |
| `all(values, pred)` | bool | 第一个 false | true |
| `find(values, pred)` | ?T | 第一个 true，返回该元素 | null |
| `filter(values, pred)` | List<T> | 处理全部元素，保留 true 对应元素 | 空 List<T> |

参数生产者按调用规则求值一次；元素回调按索引升序执行。每次调用复用
同一闭包环境，空列表不调用回调，短路后不执行剩余回调。失败立即传播，
不会跳过出错元素继续查找，也不会把错误当成 false 或 null。

```flurry
comptime fn selected()->bool {
    let flags:meta.List<bool> = [false,true,false];
    let kept=meta.filter(flags,|flag| flag);
    let first=meta.find(flags,|flag| flag);
    meta.any(flags,|flag| flag) and not meta.all(flags,|flag| flag)
        and meta.length(kept) == 1 and kept[0]
        and (first match {value? => value,null => false})
}
```

find 的缺失只属于结果外层：若 T 为 ?Integer，找到一个值为 null 的元素
返回的是 `??Integer` 的外层有值、内层缺失，与“没有元素匹配”不同。
filter 不丢弃被保留的 null，不展平嵌套列表，筛选 Type 后仍可展开到
类型工厂的变参输入。输出保留输入 List 的声明身份和元素域。

遍历的是调用时的输入值；回调通过指针替换调用者的列表不会改变遍历
序列。filter 与 map 现已改用 append 复用输出槽位缓冲区；完整验证
仍有二次成本，不将此实现称作线性 builder。any/all/find 不构造结果列表。

本阶段在 `/home/fin/luna` 执行以下检查，退出 0，45 项测试通过。
报告 `/tmp/luna-tools-5aui1kd2/result.json`。覆盖短路及空输入、生产者
求值顺序、私有捕获状态、指针替换输入、可选值层次、Type 列表身份、
回调失败与 VM 恢复、错误阶段及非 bool 回调拒绝，并回归真实库包解析。

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test comptime_list_operations --test library_meta_objects --test library_protocols --no-fail-fast
```


## 复用列表槽位缓冲区

`meta.append(target: *List<T>, value: T)` 把单个元素追加到目标槽位。
这是现有指针读写与 list_concat 的源级组合，不增加新的 IR 节点或
runtime Copy 权限。map/filter 的输出收集改为调用这个接口。

VM 识别同一局部指针的 `target.* = list_concat(target.*, [value])`。
仅当后缀为不执行用户代码的列表构造、且没有 runtime 清理调度时，
复用目标槽位中的 Vec 缓冲区；原有快照已经是独立表示，不受追加影响。
一般后缀仍走原路径，保留“先取得左值快照，再执行右操作数”的顺序，
因此右侧回调替换目标不会改变待拼接的旧内容。

追加检查旧值、新后缀与合并后的不可变表示、元素域、声明身份及预算。
合并验证失败时，在释放存储锁前截去新后缀，保留原内容。长度上限
在追加前检查；合并表示超过结构工作上限也回滚。该过程不执行用户
回调，不暴露临时合并状态。完整验证仍逐次扫描前缀，因此总时间及
验证工作表的分配仍可能二次增长；本轮消除的是反复复制整段列表值
及其缓冲区的成本，并没有实现线性时间 builder。

本阶段在 `/home/fin/luna` 执行以下检查，退出 0，69 项测试通过，报告
`/tmp/luna-tools-d_6uayqk/result.json`。预算测试对 bool/Type 及优化/
普通路径逐一扫描 1–599 步；另覆盖表示上限失败回滚、旧快照保持、
副作用后缀的求值顺序及实际 map/filter 的库级执行。

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test comptime_list_operations --test library_meta_objects --test library_protocols -p comptime_interpretor --test type_list_inputs --no-fail-fast
```

追加基准把相同输入分别交给优化路径与一般路径；一般路径仅用一个空块
包裹右侧列表构造，强制保留原来的快照传送。计时包含每次新建空列表槽位、
取得指针、追加和结果长度校验；不包含解释器及包构造、解析或 LLVM。
每个进程预热 5 次、测量 20 次，3 个进程取时间中位数。累计分配字节是
alloc/realloc 请求量，不是峰值内存。

| 256 次追加 | 一般路径时间 ns | 复用路径时间 ns | 一般路径分配字节 | 复用路径分配字节 | 一般/复用执行步数 |
| --- | ---: | ---: | ---: | ---: | ---: |
| bool | 2,547,177 | 1,143,963 | 16,937,840 | 3,662,128 | 432,928 / 190,112 |
| Type | 4,690,511 | 2,159,787 | 7,584,568 | 1,359,112 | 584,854 / 281,622 |

执行命令为 `python3 -B development/comptime_bench.py --bench list_append --iterations 20 --repeat 3`。
构建报告 `/tmp/luna-tools-6es37tcd/result.json`，三个运行报告为
`/tmp/luna-tools-95sesa9y/result.json`、`/tmp/luna-tools-3vgvn1cn/result.json`、
`/tmp/luna-tools-5k69qd9n/result.json`，均退出 0。
这证明追加路径的局部收益，不表示整个编译器或任意 map/filter 回调都有
相同加速比，也不表示剩余验证成本已变为线性。

## 切片与区间替换

`meta.slice(values, start, end) -> ?List<T>` 与
`meta.replace_range(values, start, end, replacement) -> ?List<T>`
使用半开区间 `[start, end)`；两个边界都是精确的 comptime Integer。
合法边界满足 `0 <= start <= end <= length(values)`，不截断越界值，
也不将负索引解释为倒数位置。无效边界返回外层 null；合法空区间
返回外层有值的空 List。这与元素自身是否为可选类型无关。

```flurry
comptime fn edit_types() -> ?meta.List<Type> {
    let fields: meta.List<Type> = [bool, u32, bool];
    meta.replace_range(fields, 1, 2, [u64, bool])
}

comptime fn check_slice() -> bool {
    let values: meta.List<Integer> = [10, 20, 30, 40];
    meta.slice(values, 1, 3) match {
        selected? => meta.length(selected) == 2
            and selected[0] == 20 and selected[1] == 30,
        null => false,
    }
}
```

空替换列表用于删除，空区间用于插入；`start == end == length(values)`
在末尾插入。前缀、替换内容、后缀依次进入新列表。T 的 normal-form
身份、嵌套 List 边界和元素可选层都保持，Type 列表结果可继续作为
variadic 类型函数的输入。

这些是普通 comptime 函数：所有参数依源顺序求值一次，进入函数后才
检查边界。即使边界无效，replacement 参数表达式仍然执行；若该表达式
改写原列表变量，函数仍使用第一个参数已经取得的不可变快照。调用失败
或 VM 预算耗尽不会伪装成 null。

实现复用现有索引、append 和指针契约，没有增加特权 intrinsic、IR 节点
或隐式 List-to-runtime 转换。内部 append_range 只在边界检查后调用。
当前 append 仍检查累计结果，因此这些操作尚不承诺线性时间；它们也不
自动注册列表模式协议或提供具名 rest 的切片语义。

## Library traversal through invocation-time inline ranges

`foreach`, `fold`, `map`, `any`, `all`, `find`, `filter` and the private
`append_range` helper now use inline Integer ranges instead of manually
incrementing mutable cursors. Each operation still caches the input length,
reads the original immutable snapshot, preserves callback state/order and
retains the existing output-slot append behavior. This is not a complexity or
performance claim: immutable result validation remains unchanged.

An end-to-end public StructBuilder regression supplies two distinct reflected
subjects to one ordinary-parameter `Columns` function. Its inline range reads
field names and Types, builds two independent nominal declarations in the
requesting package, and delivers aliases usable by runtime construction and
field access. The range suite also checks return from either endpoint (skipping
later producers/body) and return from the iteration guard.

Validation in `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test comptime_list_operations --test builder_invocations --test comptime_inline_loops
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

- 90 tests passed: list operations 26, builder invocations 53, comptime ranges
  11; exit 0, `/tmp/luna-tools-_ga8gp0i/result.json`.
- Formatting: `/tmp/luna-tools-y365frgq/result.json`.
- List coverage includes empty/short-circuit behavior, callback phases,
  producer evaluation order, mutable closure state, immutable snapshots,
  optional and normal-form domains, failure recovery and budget exhaustion.
