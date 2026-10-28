# meta

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
| `object_length(object)` | 条目总数，包括重复键和普通项 |
| `object_key(object, index)` | 命名项返回 `?Symbol` 的存在值，普通项返回 null |
| `ObjectEntryType(object, index)` | 条目原始 Type 身份 |
| `object_value<T>(object, index)` | 类型与 T 相同时返回存在的值，否则返回 null |

```flurry
comptime fn enabled() -> bool {
    let config = { flag: true, 'x', flag: false };
    let first = meta.object_value<bool>(config, 0);
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

CMTT 生成器也可以保存和查询局部 Object，再据此选择 quoted code。其物化
保留每个条目的类型、顺序与键，不把对象伪装成 normal term；嵌套 Object
使用同一路径。对象中的活动存储指针不能借此成为可物化的数据快照。
原生示例见 [host-object-queries.fl](../../development/examples/host-object-queries.fl)。

编辑接口返回新的 Object，保留源对象快照：

| 接口 | 行为 |
| --- | --- |
| `object_concat(left, right)` | 按左、右顺序拼接，保留重复键 |
| `object_append(object, value, .key = null)` | 追加普通项；提供 Symbol key 时追加命名项 |
| `object_replace(object, index, value)` | 保留原位置和 key，改用新值及其类型 |
| `object_remove(object, index)` | 删除指定项，后续条目向前移动 |

```flurry
comptime fn configured() -> bool {
    let source = { flag: true, 'x' };
    let extended = meta.object_append(source, false, .key = .flag);
    let replaced = meta.object_replace(extended, 1, true);
    let result = meta.object_remove(replaced, 2);
    meta.object_length(result) == 2
        and meta.ObjectEntryType(source, 1) == char
        and meta.ObjectEntryType(result, 1) == bool
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
    meta.object_value<bool>(metadata, 0) match { value? => value, null => false }
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
| `insert_field(index, name, field_type)` | 在 index 前插入；允许在字段数处追加 |
| `replace_field(index, name, field_type)` | 同时替换已有字段的名称和 Type |
| `remove_field(index)` | 删除已有字段，后面的字段保持顺序向前移动 |

合法位置返回 true；负数或越界返回 false，草稿不变。空草稿只允许在 0 插入。
字段名和 Type 的新列表都准备好后才写回；此前保存的草稿快照不受后续编辑影响。
这些方法只检查编辑位置，重名、不可存储 Type 等仍由 `Build` 检查，编辑成功不
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
字段默认代码、属性、泛型子句、方法和 impl
的构造尚未接入；这里还不是完整 SoA 示例所需的全部 builder 能力。

`EnumBuilder` 构造普通具体 enum，`new(.name = ...)` 指定诊断名称，
`add_unit`、`add_tuple`、`add_named` 分别保留无 payload、位置 payload 和具名 payload
的形状。`Build` 返回新鲜的 nominal Type；payload Type 的身份不会被展开替换。

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
固定宽度编码、symbol 编码、泛型参数、字段默认值和属性的 enum 构造尚未接通。

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

### 转换后收集列表

`filter_map(values, transform)` 要求 comptime 回调返回 `?U`，按输入顺序收集
存在的结果，返回 `List<U>`。它只去掉外层可选性：回调返回 `??T` 时，存在
但 payload 为 null 的结果仍作为 `?T` 元素保留。

`flat_map(values, transform)` 要求 comptime 回调返回 `List<U>`，按外层输入
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
    let selected = meta.filter_map(input, select);
    let expanded = meta.flat_map(input, expand);
    meta.length(selected) == 2 and selected[0] == 10 and selected[1] == 30
        and meta.length(expanded) == 4 and expanded[2] == 3
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
