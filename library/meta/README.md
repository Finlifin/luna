# meta

Object 的查询与编辑定义在固有 `impl Object` 中，可以通过接收者或 `meta.Object` 调用；旧的 `meta.object_*` 自由函数不再导出。编辑方法返回新 Object，保留输入快照。

数据与 definition builder 模块通过 `builtin` 的公开绑定引用 `Type`、`Integer`、`Symbol` 和 `usize`，不在签名或方法体中重复访问原始类型符号。`bindings.fl` 集中定义 Object 类型与操作 intrinsic 的具名绑定；数据及 builder 模块通过调用或再导出复用它们。List、StructBuilder 与 EnumBuilder 接收者已使用值引用；其他历史 comptime 指针用例仍待迁移。

源码目录与待实现领域见 [六包结构](../STRUCTURE.md)。

[`meta.math`](math.md) 提供 `builtin.math` 的编译期对应接口，由 VM 执行；
普通 `math` 名称仍指向 runtime 数学库。

包入口只保留 `List` 类型工厂和公开导出；保持 `meta.List` 的 normal-form
声明身份。列表算法位于 `data/list.fl` 的 `impl List<T> where T`，Object 操作、
整数运算、struct builder 与 enum builder 分别位于 `data/object.fl`、`data/integer.fl`、
`definitions/structs.fl`、`definitions/enums.fl`。

列表操作使用接收者调用，例如 `values.length()`、`values.map(transform)`、
`values.fold(initial, combine)`。`append` 使用值引用 `self`，通过 `values.append(value)`
修改原列表位置，支持结构字段及跨方法转发，不需要取址或解引用。整体更新需要可重新绑定的位置；`let` 列表的查询仍可调用。带回调的遍历显式保存输入快照。需要显式元素类型时可使用
`meta.List<T>.concat(left, right)`。旧的 `meta.length(values)` 等自由函数不再导出。

`values.append_all(other)` 把整个同元素域列表追加到当前接收者。`other` 保持值快照，
`values.append_all(values)` 追加调用时取得的原内容，已有快照不变；空后缀仍遵守
实参求值与可重新绑定位置的要求。字段接收者在实参求值之后读取当前内容，
因此实参中已经发生的写入不会被旧快照覆盖。验证或表示预算失败不提交追加，
但不会回滚实参求值的副作用。`flat_map` 通过此接口追加每个回调返回的列表。
这减少输出前缀的重复复制，完整内容验证与现有构造成本限制仍然保留。

`to_usize(value: Integer) -> usize` 与 `from_usize(value: usize) -> Integer`
是 pure comptime 函数，用于编译期数据与目标尺寸接口之间的精确转换。
前者拒绝负数及超出目标 usize 范围的值，后者保留完整非负数值；不截断，
也不按位重解释。两者使用本次编译的目标宽度，缺少目标依据时调用失败，
不会采用运行编译器的机器宽度。转换操作数按普通调用规则求值一次，
失败携带调用内部转换节点的源码位置，并消耗 invocation 预算。

```flurry
comptime fn checked_size(value: Integer) -> bool {
    let count = meta.to_usize(value);
    meta.from_usize(count) == value
}
```

`div_trunc(left: Integer, right: Integer) -> Integer` 与
`rem_trunc(left: Integer, right: Integer) -> Integer` 提供显式向零截断的
任意精度除法和余数，均为 pure comptime 函数。商向零舍入，非零余数与被除数
同号，始终满足 `left == quotient * right + remainder`。除数为零使 invocation
失败并保留源码位置；不存在固定宽度的最小负数溢出或按主机位宽截断。

```flurry
comptime fn signed_division() -> bool {
    meta.div_trunc(-7, 3) == -2 and meta.rem_trunc(-7, 3) == -1
        and meta.div_trunc(7, -3) == -2 and meta.rem_trunc(7, -3) == 1
}
```

两个操作数按普通调用顺序各求值一次，大整数工作消耗 invocation 预算。
它们不为 Integer 的 `/` 或 `%` 安装默认协议，也不引入 runtime Integer 表示。
需要其他舍入方式时不能把这两个函数当作对应操作。

完整 sysroot 已为 `List<T>` 提供 `ListPatternView` 的 comptime contribution。
`[a, b]`、`[a, ..._]` 与 `[]` 可直接用于编译期列表，提取的元素保持 T 的身份；
列表自身仍没有 runtime 表示或 runtime 迭代承诺。
长度和索引经上述接口转换，使用主机 pipeline 时目标依据由驱动提供。
例子见 [sysroot-meta-list-patterns.fl](../../development/examples/sysroot-meta-list-patterns.fl)，
通过 `python3 -B development/library_check.py --release --stage sysroot-meta-patterns` 检查。

需要直接调用模式协议方法时，用实现视角明确指定来源：

```flurry
comptime fn first() -> Integer {
    let values: meta.List<Integer> = [10, 20];
    values.as(ListPatternView).list_pattern_view_get_element_comptime(0)
}
```

这里的索引是目标平台的 usize。空列表或越界索引使 invocation 失败；普通方法
调用不会先匹配长度。仅导入 trait 不等于在当前命题上下文中声明该能力，因此
不能把省略 `.as(ListPatternView)` 的拼写当作自动搜索所有 trait 的承诺。
方法查找保留 List 的 normal-form 头，不要求将其展开成底层结构；实现适用性、
参数及结果域检查仍然执行，接收者生成表达式仍只求值一次。

`Object` 是编译期异构对象的公开类型别名，经 `meta.prelude` 导出。
用户代码无需直接引用编译器内置符号：

```flurry
use meta.prelude.*;

pure comptime fn marker() -> Object {
    let value = { enabled: true, nested: { tag: .example } };
    value
}

^marker()
fn example() {}
```

对象保存条目的源码顺序、各自的类型和值，允许普通项、命名项、重复键和嵌套对象。
构造对象不会自动合并重复键。它可以作为 comptime 函数的参数和结果，不能通过这个别名进入 runtime 接口。

`--emit attributes` 可以检查上述元数据；任意属性键的执行语义仍需要对应消费者，
不能由对象构造器自行赋予。按条目位置读取的接口如下，索引使用 Integer：

| 接口 | 结果 |
| --- | --- |
| `Object.length(object)` | 条目总数，包括重复键和普通项 |
| `Object.key(object, index)` | 命名项返回 `?Symbol` 的存在值，普通项返回 null |
| `Object.EntryType(object, index)` | 条目原始 Type 身份 |
| `Object.value<T>(object, index)` | 类型与 T 相同时返回存在的值，否则返回 null |

```flurry
comptime fn enabled() -> bool {
    let config = { flag: true, 'x', flag: false };
    let first = meta.Object.value<bool>(config, 0);
    first match { value? => value, null => false }
}
```

返回 true。查询不会合并两个 flag，也不会根据条目值进行隐式类型转换。
typed read 匹配条目保存的声明域身份，不作为一般子类型或类型命题查询。
所有索引查询都检查边界；负数或越界索引导致 invocation 失败，不与类型不符或
无键条目混为一谈。读取 `?bool` 条目时，返回类型是 `??bool`，存在但为空的
payload 保留其独立层。输入按普通调用顺序各求值一次，查询仍受 invocation
预算限制。当前查询沿用普通值复制路径，不承诺常数时间访问大型对象。
由可冻结条目构成的 Object 也能作为 normal-form 和普通函数的 clause 参数。

按名称读取使用 `Object.get<T>(object, key: Symbol) -> ?T`。它选择最后一个同名
条目，忽略普通项；缺失 key 或最后一项的域与 T 不同，均返回 null，不向前寻找
类型相符的旧值。它不改写 Object，也不合并重复键。

```flurry
comptime fn configured() -> bool {
    let settings = { enabled: true, enabled: false };
    meta.Object.get<bool>(settings, .enabled) matches false?
}
```

查询检查所有条目的类型和值，包括被重复键遮住的条目；非法数据导致 invocation
失败，不能伪装成 key 缺失。返回值保留原始 Type 身份与 optional 层，不做隐式
转换或 normal-form 展开。Object 接收者和 key 依次求值一次。接收者为位置时保留值引用；key 的计算若
替换该位置，方法读取更新后的 Object。需要固定旧输入时先保存独立快照，再对
快照调用查询。计算得到的临时 Object 只求值一次。查询和结果复制计入预算。
该实现一次扫描 Object，仍沿用普通参数复制路径，不承诺常数时间查找。

`Object.set(index, value)` 通过 comptime 值引用修改接收者，保留该项的 key 和位置，
允许替换为不同域的值。`replace` 仍返回新 Object，不修改原值。直接修改 Object
绑定需使用 `var`；已有快照保持原值。越界索引使 invocation 失败，不插入条目。

```flurry
comptime fn configured() -> bool {
    var draft = {enabled: false};
    let saved = draft;
    draft.set(0, true);
    (saved.get<bool>(.enabled) matches false?)
        and (draft.get<bool>(.enabled) matches true?)
}
```

嵌套接收者支持 `holder.entries.set(index, value)`。实参按调用规则求值一次；
如果实参计算更换了 `holder.entries`，方法读取并更新更换后的 Object，不覆盖回
旧快照。替换完成检查后才写回，失败不回滚此前实参求值的副作用。内部值引用不
成为源码指针，也不授予 Object runtime 表示或全局 Copy。真实 sysroot 用例已
纳入 `sysroot-meta-object` 检查。
Object 和 List 的值引用路径支持普通名义结构体、空 optional 与空 List 的
comptime 快照，不要求这些 payload 实现 runtime Copy／Clone。源码回归
`comptime_object_storage` 与 sysroot 用例覆盖这些域；旧指针兼容路径的资源
限制不构成 comptime 值引用的契约。

`List.append` 通过内部值引用更新接收者，不要求 runtime Copy／Clone。
读取列表值产生独立快照，字段路径在写入时重新检查；预算或路径失败不提交该次写入。
纯 Type 描述保持 comptime 数据身份，不因这些检查展开 normal-form 头。
值引用接收者的 `self = list_concat(self, [value])` 现可复用原列表缓冲区，
包括调用者字段上的接收者；已有快照不受影响。该优化只接受不执行用户代码的
追加后缀，验证失败时撤销后缀。它仍执行完整的类型与表示预算检查，因此减少
列表复制与分配，但不保证连续追加的总成本为线性；一般拼接仍按快照顺序求值。
普通值引用参数在调用入口借用验证 List 内容，不再仅为验证复制列表缓冲区；
每次调用仍检查当前内容，依赖输入与显式契约快照继续按原规则物化。
上面的 Object 指针示例仍是待迁移的旧实现记录，不是有效的 comptime 语言用法。

CMTT 生成器也可以保存和查询局部 Object，再据此选择 quoted code。其物化
保留每个条目的类型、顺序与键，不把对象伪装成 normal term；嵌套 Object
使用同一路径。对象中的活动存储指针不能借此成为可物化的数据快照。
原生示例见 [host-object-queries.fl](../../development/examples/host-object-queries.fl)。

编辑接口返回新的 Object，保留源对象快照：

| 接口 | 行为 |
| --- | --- |
| `Object.concat(left, right)` | 按左、右顺序拼接，保留重复键 |
| `Object.append(object, value, .key = null)` | 追加普通项；提供 Symbol key 时追加命名项 |
| `Object.replace(object, index, value)` | 保留原位置和 key，改用新值及其类型 |
| `Object.remove(object, index)` | 删除指定项，后续条目向前移动 |

```flurry
comptime fn configured() -> bool {
    let source = { flag: true, 'x' };
    let extended = meta.Object.append(source, false, .key = .flag);
    let replaced = meta.Object.replace(extended, 1, true);
    let result = meta.Object.remove(replaced, 2);
    meta.Object.length(result) == 2
        and meta.Object.EntryType(source, 1) == char
        and meta.Object.EntryType(result, 1) == bool
}
```

替换和删除的负数、越界索引使 invocation 失败。替换可改变条目类型，
不会将新值转换到旧类型；后续 typed read 使用新类型。编辑沿用普通参数
求值及复制规则，容器搬移、类型标签复制和动态 key 分配计入预算。
CMTT 与原生 O0/O3 示例见
[host-object-edits.fl](../../development/examples/host-object-edits.fl)。

Object 的稳定输入编码保留每一项的 key、原始 Type 和值，因此普通项、空 key、
重复 key、条目顺序及嵌套 optional 层都不会丢失。指针和活动存储不能进入此编码；
冻结与重建都受预算约束，重建会检查每一项的 payload 域。

```flurry
pure comptime fn Tagged<metadata: meta.Object> -> Type { bool }

comptime fn enabled() -> bool where metadata: meta.Object {
    meta.Object.value<bool>(metadata, 0) match { value? => value, null => false }
}
```

`Tagged<metadata>` 保留 normal-form 的头和完整参数，不因函数体返回 bool 而
展开为 bool。编码的表示相等不是通用类型判定：不同输入的表示不自行证明
normal-form 结果不等，也不赋予 head 单射性质。当前普通函数 clause 调用支持
已完成的常量输入；将普通 comptime 局部变量传给 clause 的通用接线仍未完成，
见 [复现](../../development/repros/comptime-local-clause-input.fl)。完整使用示例见
[host-object-inputs.fl](../../development/examples/host-object-inputs.fl)。

`StructBuilder` 提供具体结构体的字段构造接口。它是 comptime 草稿，
`new()` 默认使用名称 `.Generated`，也可以通过 `.name` 指定诊断名称。
`add_field` 按调用顺序保存字段名与 Type；返回 Type 的方法名是 `Build`。
`from_type(T, .name = .Generated)` 将已实例化结构体的字段复制成可编辑草稿，
也支持当前 comptime invocation 中刚生成的结构体。复制保留字段顺序和 Type 身份，
不复制默认值、属性、方法或协议实现；`Build` 仍创建新的名义类型。

```flurry
struct Source { first: bool, second: bool }

comptime fn Mirror() -> Type {
    let builder = meta.StructBuilder.from_type(Source, .name = .Mirror);
    builder.Build()
}

typealias Mirrored = Mirror();

impl Mirrored {
    fn first_value(self) -> bool { self.first }
}

fn example() -> bool {
    let value = Mirrored { first: true, second: false };
    value.first_value()
}
```

草稿也支持按声明顺序编辑字段，索引为 comptime `Integer`：

| 方法 | 行为 |
| --- | --- |
| `add_fields(names, types)` | 按列表顺序追加整批字段，名称与类型数量必须相同 |
| `replace_fields(start, end, names, types)` | 将半开区间 `[start, end)` 替换为整批字段；允许空区间插入和空列表删除 |
| `insert_field(index, name, field_type)` | 在 index 前插入；允许在字段数处追加 |
| `replace_field(index, name, field_type)` | 同时替换已有字段的名称和 Type |
| `remove_field(index)` | 删除已有字段，后面的字段保持顺序向前移动 |

合法位置返回 true；负数或越界返回 false，草稿不变。空草稿只允许在 0 插入。
字段名和 Type 的新列表都准备好后才写回；此前保存的草稿快照不受后续编辑影响。
批量接口还检查名称与 Type 数量相等，不匹配时返回 false，草稿不变。
这些方法检查编辑位置和列表数量，重名、不可存储 Type 等仍由 `Build` 检查，编辑成功不
代表定义已经合法。接口不复制或编辑尚未接通的字段默认表达式和属性。

```flurry
struct Source { old: bool, removed: bool }

comptime fn Edited() -> Type {
    var builder = meta.StructBuilder.from_type(Source, .name = .Edited);
    let replaced = builder.replace_field(0, .value, u8);
    let removed = builder.remove_field(1);
    let inserted = builder.insert_field(0, .ready, bool);
    assumes replaced and removed and inserted;
    builder.Build()
}

typealias Record = Edited();
```

生成的 Record 字段依次为 `ready: bool`、`value: u8`。完整 sysroot 的原生构造、
字段读取和反射测试见
[sysroot-edited-struct.fl](../../development/examples/sysroot-edited-struct.fl)，通过
`python3 -B development/library_check.py --release --stage sysroot-builders` 执行。

`Build` 检查字段 Type 的形成与有限存储表示后才发布类型，重名字段会被拒绝；字段 Type
保留 normal-form 头。空草稿可以发布空结构体，名称相同不代表类型身份相同。
草稿及其中的 List 数据不能成为 runtime 接口。

草稿输入错误保留结构化诊断及构造调用的源码范围。重名字段报告
`duplicate-field`（包含所属定义或 variant 与字段名），重名 variant 报告
`duplicate-variant`；字段数量不匹配显示 names/types 数量，错误 payload 形状
显示 variant 名、shape 和数量，分组路径的空片段或内嵌点号报告 `invalid-path`。
名称按字符串转义，诊断可按 S-expression 阅读。这些错误在发布前拒绝，不会
将半成品定义交给后续代码；修正草稿后可再次构造。其他类型形成、预算与发布
错误仍按各自检查路径处理，不保证所有失败都已有专门的草稿诊断。


可以通过现有 `--invoke-comptime` 执行构造入口，单文件、显式包集合和 sysroot
路径都会安装发布服务；显式加载 meta 时仍需绑定 `--meta-list-type meta.List`。
驱动的 `compile_and_invoke_with_struct_builders` API 还会同时返回调用值和含生成
定义的检查后 NIR。直接的 typealias 初始化、常量计算和即时必需 comptime
调用也已接入；生成类型可以进入后续调用、构造表达式和运行时字段访问。
生成声明的编号边界随 NIR 保存，后续显式生成不会覆盖初始计算产生的类型。
延迟标注、分阶段默认参数和计算得到的构造器也已接入发布服务。
延迟标注可以反射调用方在原始快照之后生成的类型；恢复计算时使用已完成的输入，
不重新执行实参。省略默认类型参数时执行其生成器，显式提供该参数时跳过默认计算。
生成类型的声明身份也保留在后续类型约束检查中。
生成类型可以作为普通固有 impl 和 trait impl 的目标；准备 impl 头时，也可以调用
`StructBuilder` 并反射源码结构体。固有方法选择检查完整的同名候选集合，
不会因准备顺序忽略后面的竞争候选。依赖 trait 选择的计算仍须等待完整实现集合。
`Once`、`Clone`、`Copy` 等协议仍由普通 impl 提供，生成或反射类型不会自动授予它们。
早期反射也准备嵌套字段及泛型实参所引用的类型描述，并保留递归指针字段的身份。
可运行的泛型源类型示例见
[host-generated-nested-struct.fl](../../development/examples/host-generated-nested-struct.fl)。
默认类型参数不为任意 `T: Type` 提供字段证据，泛型函数体仍须满足其声明约束。
已可组合反射、List 映射与批量字段构造，将每个字段 `T` 转换为已声明的泛型
容器 `Column<T>`。生成类型保持这些应用的 normal-form 头，运行时展开存储布局，
支持嵌套字段访问及显式固有 impl：

```flurry
struct Column where T { first: T, second: T }

comptime fn Columns(subject: Type) -> Type {
    let names = meta.struct_field_names(subject);
    let types = meta.struct_field_types(subject).map(|field| Column<field>);
    var builder = meta.StructBuilder.new(.name = .Columns);
    let added = builder.add_fields(names, types);
    assumes added;
    builder.Build()
}

struct Row { active: bool, count: u8 }
typealias ColumnStore = Columns(Row);
```

完整原生用例见 `development/examples/sysroot-edited-struct.fl` 的
`transformed_fields`；`sysroot-builders` 检查入口同时覆盖字段编辑、反射、批量构造
和类型转换后的运行时访问。这里的 Column 是两元素示例容器，不是完整动态列库。
字段默认代码、属性、泛型子句、方法和 impl
的构造尚未接入；这里还不是完整 SoA 示例所需的全部 builder 能力。

`EnumBuilder` 构造普通具体 enum，`new(.name = ...)` 指定诊断名称，
`add_unit`、`add_tuple`、`add_named` 分别保留无 payload、位置 payload 和具名 payload
的形状。`Build` 返回新鲜的 nominal Type；payload Type 的身份不会被展开替换。
普通 `StructBuilder`／`EnumBuilder` 的发布回调必须忠实返回请求描述：不能额外
附加泛型输入或约束；普通 enum 也不能被改成 intrinsic、固定宽度或 Symbol 编码。
描述符不匹配在调用发布 commit 前拒绝，并保留构造表达式的源码位置；拒绝后可以
在同一 invocation 中重试合法构造。这不限制以后通过专门构造接口支持这些定义。

```flurry
comptime fn EventType() -> Type {
    var builder = meta.EnumBuilder.new(.name = .Event);
    builder.add_unit(.idle);
    builder.add_tuple(.data, [bool]);
    builder.add_named(.record, [.flag], [bool]);
    builder.Build()
}

typealias Event = EventType();

fn read(value: Event) -> bool {
    value match {
        .idle => false,
        .data(flag) => flag,
        .record { flag: flag } => flag,
    }
}

fn example() -> bool {
    read(Event.data(true)) and read(Event.record { flag: true })
}
```

重名 variant、重名 payload 字段、字段名与 Type 数量不一致，以及无限大小的
按值递归表示都会被拒绝。struct 与 enum 生成共用声明编号和检查状态，可以将前一个
生成类型作为后一个生成类型的 payload 或字段。显式 invocation 返回的 NIR 同时保留
这两种生成声明。`add_group` 声明分组，`.parent = [.net]` 指定父组路径；variant 方法用
`.group = [.net, .nested]` 指定所属组。路径片段按顺序保存，不改变 nominal Type。
父组必须声明；重复路径、group 与 variant 同名都会被拒绝。
泛型参数、字段默认值和属性的 enum 构造尚未接通。固定宽度构造支持下文的固定掩码布局，不包含一般 guard。

可运行的 native 示例见 [host-generated-enum.fl](../../development/examples/host-generated-enum.fl)。

分组示例（在 `Build` 前添加）：

```flurry
builder.add_group(.net);
builder.add_group(.nested, .parent = [.net]);
builder.add_tuple(.received, [bool], .group = [.net, .nested]);
```

生成后使用 `Event.net.nested.received(true)` 构造；pattern `.net` 匹配该组的全部
后代，但不会匹配名为 `.network` 的相邻组。空分组仍保留在描述中，不增加 variant。

Enum 的逻辑结构可以在 comptime 反射，结果保持声明顺序：

| 接口 | 返回值 |
| --- | --- |
| `enum_group_paths(T)` | `List<List<Symbol>>`，每组的路径片段 |
| `enum_variant_paths(T)` | `List<List<Symbol>>`，每个 variant 的路径片段 |
| `enum_variant_shapes(T)` | `List<Symbol>`，`.unit`、`.tuple` 或 `.named` |
| `enum_variant_field_names(T)` | `List<List<Symbol>>`，具名 payload 的字段名；其他形状为空列表 |
| `enum_variant_field_types(T)` | `List<List<Type>>`，实例化后的 payload Type |

`EnumBuilder.from_type(T, .name = .Generated)` 将这些逻辑信息复制为可编辑草稿。
它接受具体泛型应用和本轮 invocation 刚生成的 enum，`Build` 产生独立的 nominal
身份。该操作不复制编码 ABI、属性、默认表达式或 trait 实现，反射顺序也不表示判别值。

```flurry
enum Source where T { data(T), stopped }

comptime fn Mirror() -> Type {
    var builder = meta.EnumBuilder.from_type(Source<bool>, .name = .Mirrored);
    builder.add_unit(.extra);
    builder.Build()
}

typealias Mirrored = Mirror();
```

完整 native 示例见 [host-mirrored-enum.fl](../../development/examples/host-mirrored-enum.fl)。

EnumBuilder 同时支持编辑已有的变体和分组：

| 接口 | 行为 |
| --- | --- |
| `insert_unit(index, name, .group = [])` | 在逻辑索引前插入 unit 变体 |
| `insert_tuple(index, name, fields, .group = [])` | 插入位置 payload 变体 |
| `insert_named(index, name, names, fields, .group = [])` | 插入具名 payload 变体 |
| `replace_unit` / `replace_tuple` / `replace_named` | 参数同对应 insert；替换已有变体，可改变 payload 形状 |
| `remove_variant(index)` | 删除一个变体及其 payload 描述 |
| `insert_group(index, name, .parent = [])` | 在分组列表中插入路径 |
| `replace_group(index, name, .parent = [])` | 替换已有分组路径 |
| `remove_group(index)` | 删除一个分组声明 |

这些方法返回 bool。插入允许 `index == length`；替换和删除要求索引指向已有
条目。负索引、越界和超大 Integer 索引返回 false，草稿不变。变体编辑先准备
全部新的平行列表，再写入各列表槽位。它使用 List 的编译期不可变数据传送，
不为整个 builder 授予隐式 Copy。

索引来自反射的逻辑声明顺序，与编码判别值无关。组与变体列表分别编辑：
重命名或删除组不会自动移动后代，须显式更新后代的路径。允许暂时不完整的
草稿；重复路径、缺失父组、payload 名称/类型不匹配仍由 Build 拒绝。
保存的草稿值和已经生成的 Type 不随后续编辑改变。编辑不展开 payload Type
的 normal-form 头，也不复制源类型的 ABI、属性或 trait 实现。

```flurry
enum Source { old.{data(bool)}, stopped }

comptime fn Revised() -> Type {
    var builder = meta.EnumBuilder.from_type(Source);
    let renamed = builder.replace_group(0, .net);
    let moved = builder.replace_tuple(0, .data, [u8], .group = [.net]);
    let inserted = builder.insert_named(1, .record, [.ready], [bool], .group = [.net]);
    assumes renamed and moved and inserted;
    builder.Build()
}
```

完整 sysroot 示例见 [sysroot-edited-enum.fl](../../development/examples/sysroot-edited-enum.fl)，
运行 `python3 -B development/library_check.py --release --stage sysroot-enum-builder`。


### 转换后收集列表

`values.filter_map(transform)` 要求 comptime 回调返回 `?U`，按输入顺序收集
存在的结果，返回 `List<U>`。它只去掉外层可选性：回调返回 `??T` 时，存在
但 payload 为 null 的结果仍作为 `?T` 元素保留。

`values.flat_map(transform)` 要求 comptime 回调返回 `List<U>`，按外层输入
顺序、内层列表顺序收集元素，返回 `List<U>`。空的内层列表不产生元素。

```flurry
comptime fn select(value: Integer) -> ?Integer {
    if value == 2 { null } else { value * 10 }
}

comptime fn expand(value: Integer) -> meta.List<Integer> {
    if value == 2 { [] } else { [value, value + 10] }
}

comptime fn example() -> bool {
    let input: meta.List<Integer> = [1, 2, 3];
    let selected = input.filter_map(select);
    let expanded = input.flat_map(expand);
    selected.length() == 2 and selected[0] == 10 and selected[1] == 30
        and expanded.length() == 4 and expanded[2] == 3
}
```

两个操作都对输入快照的每个元素调用一次回调，空输入不调用回调。回调捕获状态
在调用间保留；即使回调替换调用者持有的原列表，正在遍历的快照仍保持不变。
回调失败立即中止当前收集；不会跳过失败元素继续执行。元素的 Type 身份不变，
例如作为元素的 `Identity<bool>` 不会因收集而展开为 bool。接口只用于 comptime，
不授予列表运行时表示，也不承诺线性时间构造；现有不可变数据校验仍然生效。

完整 sysroot 用例见
[sysroot-meta-lists.fl](../../development/examples/sysroot-meta-lists.fl)，运行
`python3 -B development/library_check.py --release --stage sysroot-meta`。
该阶段在 host 测试中通过 inline if 消费 comptime 结果，检查器要求实际执行
一项测试且没有失败，不将仅成功编译视为通过。

### 首项归约与转换后查找

`values.reduce(combine) -> ?T` 使用首项作为累积值，对后续元素按顺序调用
`combine(accumulator, item) -> T`。空输入返回 null，单项输入不调用回调；
非空输入调用次数为元素数减一，不要求调用方构造单位元。

`values.find_map(transform) -> ?U` 按顺序执行返回 `?U` 的 comptime 回调，
遇到第一个存在结果立即返回，不执行后续回调。所有结果均为空或输入为空时返回 null。
两个接口均保留嵌套 optional 层：归约的首项为 null，或查找到的存在结果内含 null，
都不会变成外层的缺失结果。

```flurry
comptime fn first_positive(value: Integer) -> ?Integer {
    if value > 0 { value } else { null }
}

comptime fn example() -> bool {
    let values: meta.List<Integer> = [1, 2, 3];
    let reduced = values.reduce(|left, right| left * 10 + right);
    let found = values.find_map(first_positive);
    (reduced match { value? => value == 123, null => false })
        and (found match { value? => value == 1, null => false })
}
```

遍历固定输入快照；回调修改调用方保存的列表不会改变本次遍历。回调失败立即中止，
不会跳过失败项继续处理。返回的 Type 元素保留原身份，不展开 normal-form 头。
这两个接口只用于 comptime；现有不可变数据检查和 invocation 预算仍然生效。
完整 sysroot 用例已纳入 `sysroot-meta` 检查入口。

### 稳定排序

`values.sort_by(less) -> List<T>` 返回新列表，不修改输入快照。
`less` 是接收两个 T 并返回 bool 的 comptime 回调，表示严格先后关系；
等价元素保留原来的相对顺序。空列表与单元素列表不调用比较器，但仍检查其
参数、结果和执行阶段。Type 元素保持原有身份，不展开 normal-form 头。

```flurry
comptime fn sorted_numbers() -> meta.List<Integer> {
    let input: meta.List<Integer> = [3, 1, 2];
    input.sort_by(|a, b| a < b)
}
```

实现从宽度 1 开始逐轮归并相邻段，按左到右顺序处理段。每次比较调用
`less(右段候选, 左段候选)`；仅 true 选择右项，false 选择左项。某段耗尽后
直接追加另一段的剩余内容。比较器应提供一致的严格弱序；带副作用的比较器
仍按上述顺序执行，异常立即停止，不继续调用后续比较。

比较次数为 O(n log n)，但当前 VM 的列表复制、参数传递及资源检查仍有额外
成本，因此不把整个 invocation 的时间或预算消耗宣称为 O(n log n)。所有工作
消耗通常的 invocation 预算。失败不会发布部分排序结果，也不会回滚比较器
已经执行的显式副作用。来源变量被比较器替换时，已传入的输入快照仍然有效。

真实 sysroot 示例见
[sysroot-meta-lists.fl](../../development/examples/sysroot-meta-lists.fl)；运行
`python3 -B development/library_check.py --release --stage sysroot-meta`。

### 分区点与二分边界查找

`values.partition_point(pred) -> Integer` 返回 true 前缀之后第一个 false
元素的下标；全为 true 返回列表长度，全为 false 或空列表返回 0。调用者负责
保证谓词把列表分为 true 前缀、false 后缀；实现不会先线性扫描验证这个前提。

`values.lower_bound(needle, less)` 返回第一个不严格小于 needle 的位置；
`values.upper_bound(needle, less)` 返回第一个严格大于 needle 的位置。
列表必须按同一个严格弱序比较器排序。相等元素的区间为 `[lower, upper)`，
缺失时两者给出相同的插入位置。降序或按字段排序同样由 less 定义。

```flurry
comptime fn duplicate_range() -> bool {
    let values: meta.List<Integer> = [1, 2, 2, 2, 4];
    values.lower_bound(2, |a, b| a < b) == 1
        and values.upper_bound(2, |a, b| a < b) == 4
}
```

这些函数都是 comptime 函数，位置使用目标宽度无关的 Integer。每一步检查
当前半开区间的向下取整中点；lower 的比较参数为 `(候选, needle)`，upper
为 `(needle, 候选)`。比较器必须返回 bool，空列表也检查回调类型与阶段，但
不调用它。非空列表最多调用 `floor(log2(n)) + 1` 次回调；不承诺包含输入复制
和资源检查的整个 invocation 也是对数成本。

输入、needle 和回调按普通调用顺序求值；搜索读取固定输入快照。回调失败立即
中止，已发生的显式副作用不回滚。非单调谓词或与列表不相容的比较器不保证
得到有意义的边界，接口不会把它们当成已验证的排序证明。

### 显式生成 Symbol 编码 enum

`EnumBuilder.BuildSymbol(codes: List<Symbol>) -> Type` 按叶变体声明顺序指定编码。
分组不占据编码位置。变体必须是 unit 形状；即使空的 tuple/named payload 也不能
用这个入口构造。编码数量必须匹配，重复 Symbol 在发布前拒绝。空变体列表与空编码
列表可以构造无可用变体的 enum。

```flurry
comptime fn Protocol() -> Type {
    var builder = meta.EnumBuilder.new(.name = .Protocol);
    builder.add_group(.net);
    builder.add_unit(.ready, .group = [.net]);
    builder.add_unit(.closed);
    builder.BuildSymbol([.wire_ready, .wire_closed])
}

typealias Message = Protocol();

fn is_ready(message: Message) -> bool {
    message match { .net.ready => true, .closed => false }
}
```

生成类型有独立 nominal 身份，编码不会成为类型名称或 variant 的声明名称；同一草稿
调用 `Build()` 仍生成普通 enum。`from_type` 仍只复制逻辑结构；要生成 Symbol 编码
类型，必须显式给出 `BuildSymbol` 的编码列表，不能把它当成 ABI 克隆。
编码表在 invocation 发布后保留到 NIR/IIR，并复用既有 Symbol enum 的构造与匹配。
它没有额外整数判别值，也不承诺 FFI 存储形式；任意 runtime Symbol 到 enum 的受检
转换仍未接通。

真实 sysroot 示例：
[sysroot-symbol-enum.fl](../../development/examples/sysroot-symbol-enum.fl)。
运行 `python3 -B development/library_check.py --release --stage sysroot-symbol-enum`。

### 生成固定宽度 enum

`EnumBuilder.BuildBits(width, masks, values, offsets, .base_type = null)` 按叶变体
顺序指定表示。`width` 是正 Integer；`masks` 与 `values` 是 `List<Integer>`，
每个非负整数必须能完整放入 width 位，不能靠截断消去高位。mask 中的 1 表示固定位，
value 只能在这些位置取 1。`offsets` 是 `List<List<Integer>>`，每个内层列表按该
变体字段声明顺序给出最低有效位偏移。

变体可以是 unit 或具名 payload。payload 宽度由无非法位模式的定宽标量或 BitVec
字段类型确定，不能借 offsets 指定不同宽度。固定位与 payload 必须恰好覆盖整个
表示，不能重叠或遗留无来源的位；各变体的编码必须互斥。tuple payload、一般 guard
和任意类型的自定义位表示不在当前入口的范围内。

```flurry
comptime fn Packet() -> Type {
    var builder = meta.EnumBuilder.new();
    builder.add_unit(.idle);
    builder.add_named(.data, [.byte], [u8]);
    builder.BuildBits(16, [0xffff, 0xff00], [0, 0x0100], [[], [0]], .base_type = u16)
}
```

不指定 base type 时只保留整体宽度，不暗示标量对齐或 base 类型身份。指定时必须是
与 width 一致的无符号整数类型。两种表示都保留新鲜 nominal 身份，复用固定宽度
源码 enum 的构造、匹配与只读 `'bitvec` 视图；`Build()` 和 `BuildSymbol()` 不从该
次调用继承编码。空 enum 仍检查整体宽度和表示预算。

[生成 ADD/SUB 指令的完整示例](../../development/examples/sysroot-fixed-enum.fl)
通过 `python3 -B development/library_check.py --release --stage sysroot-fixed-enum`
运行，检查原始指令值、具名 payload、函数传参与返回。

该示例通过 builtin.prelude 导出的透明别名 `BitVec<n>` 使用 5 位寄存器字段，
不需要用户直接调用编译器 builtin。任意宽度整数 `u5` 尚未实现；这里没有将
BitVec 字段声称为 `u5`，也不改变两者的类型身份或运算规则。

### 带索引映射、配对与位置查找

- `values.map_indexed(transform) -> List<U>` 按输入顺序调用
  `transform(index: Integer, value: T)`，索引从零开始。
- `values.zip_with(other, combine) -> List<V>` 将两个快照同位置的元素交给
  `combine(left: T, right: U)`，结果长度为较短输入的长度，不访问未配对尾部。
- `values.position(pred) -> ?Integer` 返回第一个匹配位置，找到后立即停止。
  索引零仍是存在结果；空列表或没有匹配项返回 null。

这些都是 `List` 的固有 comptime 方法。映射回调按索引顺序调用，闭包状态
在调用之间保留；空的映射范围不执行回调。实参仍按普通调用规则求值一次，
即使最终没有配对元素也不会跳过实参求值。回调失败终止本次操作。

```flurry
comptime fn offsets() -> bool {
    let widths: meta.List<Integer> = [8, 16, 32];
    let enabled: meta.List<bool> = [true, false];
    let indexed = widths.map_indexed(|index, width| index * 100 + width);
    let selected = widths.zip_with(enabled, |width, flag| if flag {width} else {0});
    let first_wide = widths.position(|width| width >= 16);
    indexed[2] == 232 and selected.length() == 2 and selected[1] == 0
        and (first_wide match { index? => index == 1, null => false })
}
```

输入及 Type 元素保持原有身份；这些操作不展开 normal-form 头，也不引入
runtime List 或 comptime 指针。输出构造仍受不可变表示验证的现有成本限制。
