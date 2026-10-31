# 编译期数据

list.fl 将列表操作 impl 到 meta.List；object.fl 提供有序 Object 查询与编辑；integer.fl 提供编译期整数操作。List 的编译器绑定 normal-form 头仍在包根，避免更改公开 meta.List 身份。

## 可选值与错误限定值

`ComptimeOptionalOps` 与 `ComptimeErrorResultOps` 已采用 `pure comptime trait`，
成员无需逐项标注 phase。方法仍显式声明 `comptime fn`；接口限定不承诺无副作用。
它们与 builtin 的 runtime 协议保留相同方法名，且拥有各自独立的接口身份。

泛型约束同时提供两份接口时，comptime 正文优先选择仅限 comptime 的接口，
runtime 正文采用普通接口；显式 `.as(Trait)` 保留指定来源。具体接收者的自动
可见接口搜索已接通，可以直接调用这里的可选值与错误限定值方法；泛型 impl 的
接口实参从接收者推导，并继续检查完整实现契约。
真实 sysroot 用例见 `development/examples/sysroot-qualified-phases.fl`。

## 前缀累积

`values.scan(initial, combine)` 返回依次处理每个元素后的状态；
`values.scan_exclusive(initial, combine)` 返回处理每个元素前的状态。
两者都产生与输入等长的列表，并从左到右对每个元素调用一次 comptime 回调。
空输入返回空列表，不调用回调；exclusive 的最后一次回调也会执行，错误和副作用不会被丢弃。

```flurry
use meta.List;

comptime fn offsets() -> List<Integer> {
    let sizes: List<Integer> = [4, 8, 1];
    sizes.scan_exclusive(0, |offset, size| offset + size)
    -- [0, 4, 12]
}

comptime fn ends() -> List<Integer> {
    let sizes: List<Integer> = [4, 8, 1];
    sizes.scan(0, |offset, size| offset + size)
    -- [4, 12, 13]
}
```

累积状态的类型可以与元素类型不同，例如将 `List<Type>` 扫描为一系列逐步增长的
`List<Type>`。输入在开始遍历时取值快照；回调改写调用者的列表不会改变本轮遍历。
输出按普通值快照规则保存，替换累积值不会改写之前的结果；不额外深拷贝指针指向的存储。`Type` 元素保留其
normal-form 身份，不展开生成函数的头。

当前实现复用 `append`，每次追加仍进行完整表示检查；不承诺线性构造成本。

## 相邻项去重与有序性检查

`values.dedup_by(equivalent)` 保留每个相邻组的第一个元素。比较器接收
“最后保留的元素、当前候选元素”，每个候选恰好比较一次；被移除的元素不会
替换组代表。非相邻的重复项仍保留，需要全局去重时可先按相应顺序排序。
比较器也可以表达非传递的分组条件，此时结果严格遵循上述代表选择规则。

`values.is_sorted_by(less)` 使用与 `sort_by` 相同的严格比较器，依次检查
`less(当前项, 前一项)`，第一次返回 true 时立即判定无序，不再调用后续比较。
相等元素可以连续出现。空列表、单元素列表不会调用比较器，且都视为有序。

```flurry
comptime fn distinct_sizes() -> List<Integer> {
    let sizes: List<Integer> = [8, 4, 8, 4, 1];
    let sorted = sizes.sort_by(|left, right| left < right);
    sorted.dedup_by(|left, right| left == right)
    -- [1, 4, 8]
}
```

两种操作都先取得输入快照，回调副作用不改变本次遍历范围。错误原样传播，
不会作为比较结果使用；特别是 Type 相等性求解得到 Unknown 时，不得把它
当作不等而继续去重。保留的 Type 元素不会展开 normal-form 头。

`is_sorted_by` 至多比较 n−1 次；`dedup_by` 对非空输入恰好比较 n−1 次。
去重输出仍复用现有 `append` 的完整表示检查，不承诺线性构造成本。
真实 sysroot 用例位于 `development/examples/sysroot-meta-lists.fl`，同时覆盖
有状态比较器、输入快照、排序组合与不调用回调的边界。

## Object 区间编辑

`object.slice(start, end)` 返回半开区间 `[start, end)` 内的条目。
`object.replace_range(start, end, replacement)` 用 replacement 的全部条目替换该区间。
索引是精确 Integer，必须满足 `0 <= start <= end <= object.length()`；
负数、逆序和越界都会使 invocation 失败，与现有 Object 单项编辑一致。
长度位置上的空区间合法；空区间替换用于插入，空 Object 替换用于删除。

```flurry
comptime fn edit_fields() -> Object {
    let fields: Object = {name: char, enabled: bool, name: Symbol};
    let selected = fields.slice(1, 3);
    selected.replace_range(0, 1, {count: usize, char})
    -- 顺序为 count: usize、无名 char、name: Symbol。
}
```

条目的名字、声明顺序、完整值与类型身份一起保留。重复键不会合并；`get`
仍只检查最后一个同名条目，不因其类型不匹配而回看此前条目。可选值的层次、
Type 的 normal-form 头和嵌套 Object 也不会因区间编辑而改变。
两个输入均保持快照，失败不会部分写回；实参按普通调用规则求值一次，
提前 return 跳过编辑，已执行的实参副作用不回滚。

切片使用受预算约束的 Object 操作，替换由切片与 concat 组合实现；结果是值，
不是可写视图，不承诺常数时间或零拷贝。真实 sysroot 用例在
`development/examples/sysroot-meta-object-methods.fl`。

## 精确 Integer 运算

任意精度 `Integer` 的操作直接定义为固有 comptime 方法，不经机器整数转换：

- `value.div_trunc(divisor)`、`value.rem_trunc(divisor)` 向零截断，余数与被除数同号或为零。
- `value.div_euclid(divisor)`、`value.rem_euclid(divisor)` 满足
  `value == quotient * divisor + remainder` 与 `0 <= remainder < divisor.abs()`，
  除数可正可负。两种除法的零除数都使 invocation 失败。
- `value.abs()` 精确取绝对值，没有固定宽度整数最小值溢出问题。
- `value.gcd(other)` 返回非负最大公约数，`gcd(0, 0) == 0`。
- `value.lcm(other)` 返回非负最小公倍数，任一输入为零时返回零。
- `value.to_usize()` 和 `Integer.from_usize(value)` 使用所选目标的位宽；
  转窄仍拒绝负数和越界值，不截断、不使用编译器宿主位宽。

```flurry
pure comptime fn padded_size(size: Integer, alignment: Integer) -> Integer {
    -- Explicit ceiling division for a positive alignment, including negative size.
    -(-size).div_euclid(alignment) * alignment
}

pure comptime fn common_period(a: Integer, b: Integer) -> Integer {
    a.lcm(b)
}
```

`meta.div_trunc`、`meta.rem_trunc`、`meta.to_usize` 和 `meta.from_usize`
保留为既有入口。这些方法不改变 `/` 或 `%` 的舍入规则，也不把 Integer
隐式降为固定宽度值。gcd 使用普通 comptime 循环，lcm 先除后乘；它们与其它
库函数一样受工作预算约束。完整 sysroot 示例见
`development/examples/sysroot-meta-integers.fl`。


## Object 逐条遍历与变换

Object 是有序的异构条目集合。`foreach`、`fold`、`filter` 和 `flat_map`
的回调每次接收一个长度为 1 的 Object，保留该条目的可选名字、完整值域和值。
可以继续使用 `entry.key(0)`、`entry.EntryType(0)`、`entry.value<T>(0)`
观察它；不会把所有值抹成 Any，也不会按键合并重复条目。

- `object.foreach(visit)` 按顺序访问每个条目，丢弃回调的 void 结果。
- `object.fold(initial, combine)` 将上次累积值和当前条目交给 combine，返回最终累积值。
- `object.filter(pred)` 保留 pred 返回 true 的完整条目，保持顺序。
- `object.flat_map(transform)` 按顺序拼接每次返回的 Object，允许删除、替换或展开条目。

```flurry
comptime fn fields_only(description: Object) -> Object {
    description.filter(|entry| {
        (entry.key(0) matches _?) and entry.EntryType(0) == Type
    })
}

comptime fn Schema() -> Type {
    let fields = fields_only({ready: bool, note: "metadata", count: u32});
    let draft = meta.StructBuilder.from_object(fields, .name = .Packet) match {
        value? => value,
        null => return bool,
    };
    draft.Build()
}
```

每个条目调用回调一次；空输入不调用回调，fold 原样返回初始累积值。
遍历开始时固定输入快照，因此回调重新赋值调用者的 Object 不改变本轮范围。
回调环境在各次调用间保留，副作用按条目顺序发生；异常、预算耗尽和非法查询
直接传播，不转成 false 或空 Object，也不回滚已发生的回调副作用。

filter 保留的是原条目，不是回调对参数局部副本的修改。flat_map 拼接的是返回值，
不会去重键、重排字段或隐式拆开可选值。需要进一步筛选的重复键仍逐条可见；
生成定义是否具有合法字段名和表示由 Build 检查。

这些方法以 slice、concat 和普通 comptime 控制流实现，仍进行完整的表示检查，
不承诺线性构造成本。真实 sysroot 示例为
`development/examples/sysroot-object-traversal.fl`，包含异构数据、回调状态、
空输入、normal-form Type 身份，以及从筛选后的描述生成并使用 runtime struct。

## List 安全索引查询

`values.get(index)`、`values.first()` 和 `values.last()` 返回 `?T`，是 List
的固有 `pure comptime` 方法。get 的有效范围是 `0 <= index < length`；负数、
长度位置及更大的索引返回 null。first/last 对空列表返回 null，对非空列表返回
首／末元素。索引使用精确 Integer，超出机器整数范围的值也按边界比较处理。

```flurry
comptime fn first_size() -> Integer {
    let sizes: meta.List<Integer> = [4, 8, 16];
    sizes.first().unwrap_or(0)
}

comptime fn has_null_element() -> bool {
    let values: meta.List<?Integer> = [null];
    values.get(0) match {
        element? => element matches null,
        null => false,
    }
}
```

查询仅增加一层可选性；已有 null 元素仍是“查到了一个 null”，不会变成越界。
结果是普通值快照，修改返回的嵌套 List 不改变原列表；Type 元素保留完整的
normal-form 身份。方法不构造源码指针或可写元素视图。

接收者和索引遵循普通调用求值次序。索引表达式只求值一次；若它改写接收者
所在槽位，方法从更新后的槽位读取。索引表达式本身的错误、预算耗尽及非法表示
仍传播，只有已求得索引的越界被转换为 null，不把任意 invocation 失败吞掉。
真实 sysroot 用例 `development/examples/sysroot-meta-lists.fl` 覆盖空值层次、
超大正负索引、嵌套快照与 Type 身份。

## 编译期文本查询

`ComptimeTextBytes` 为 `str` 提供 `starts_with(prefix)`、`ends_with(suffix)`、
`find(needle)` 和 `contains(needle)`。通过 meta.prelude 默认导入，在 comptime
上下文与已有的 `byte_length`、`byte` 一样使用普通方法调用。

```flurry
pure comptime fn crab_offset() -> ?u64 {
    "é🦀a".find("🦀")
    -- 2：UTF-8 字节偏移，不是字符序号。
}

pure comptime fn is_directive(line: str) -> bool {
    line.starts_with("@") and line.contains(":")
}
```

find 返回第一次精确匹配的位置，未找到返回 null；空 needle 总在位置 0 匹配，
包括空接收者。空 prefix/suffix 总是匹配。操作区分大小写，不做 Unicode
规范化，因此预组字符与组合字符序列不自动相等。输入及实参沿用普通 comptime
调用和值引用语义，不构造指针，也不修改文本。

实现通过既有字节查询、普通循环和检查过的边界条件组合；find 的最坏比较次数
为 `(n - m + 1) * m`（`0 < m <= n`），不承诺线性搜索成本。循环与辅助方法
遵循正常 invocation 工作预算，失败不会伪装成“未找到”。这些新增方法本轮仅
提供 comptime 接口；runtime 文本借用及资源验证仍需独立实现。

真实 sysroot 用例 `development/examples/sysroot-qualified-methods.fl` 覆盖
默认可见接口与编译期查询结果在 native 测试中的使用。隔离测试同时比较字节码
与参考执行，覆盖 UTF-8、空输入、重叠匹配、未找到及实参仅求值一次的顺序。

本阶段验证（工作目录 `/home/fin/luna`，均退出 0）：

- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_text_search --test text_bytes --test comptime_bytecode_observations`：
  9 项通过；`/tmp/luna-tools-uvv5uysr/result.json`。
- `python3 -B development/library_check.py --stage sysroot-qualified-methods --timeout 300`：
  真实 sysroot 编译及 native 测试通过；`/tmp/luna-library-rvnsqkdf/summary.json`。
- `cargo fmt --all` 经 `development/command.py` 执行通过，
  `/tmp/luna-tools-swax6so5/result.json`；`git diff --check` 通过。
