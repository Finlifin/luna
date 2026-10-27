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
不能由对象构造器自行赋予。当前尚未提供源码层的通用对象查询、修改接口，也未完成
Object 作为 normal-form 参数的冻结编码。

`StructBuilder` 提供具体结构体的字段构造接口。它是 comptime 草稿，
`new()` 默认使用名称 `.Generated`，也可以通过 `.name` 指定诊断名称。
`add_field` 按调用顺序保存字段名与 Type；返回 Type 的方法名是 `Build`。

```flurry
struct Source { first: bool, second: bool }

comptime fn Mirror() -> Type {
    let names = meta.struct_field_names(Source);
    let types = meta.struct_field_types(Source);
    let count = meta.length(names);
    var index = count - count;
    var builder = meta.StructBuilder.new(.name = .Mirror);
    while index < count {
        builder.add_field(names[index], types[index]);
        index = index + 1;
    }
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
这两种生成声明。`add_group` 保存 group 符号；当前源码 API 尚未提供组路径拼接工具。
固定宽度编码、symbol 编码、泛型参数、字段默认值和属性的 enum 构造尚未接通。

可运行的 native 示例见 [host-generated-enum.fl](../../development/examples/host-generated-enum.fl)。
