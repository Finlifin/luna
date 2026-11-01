# 受检反射

`structure.fl` 为 builtin `Type` 提供 comptime 固有方法。通过真实 sysroot
加载 meta 后，可以直接在命名类型、泛型应用或局部 Type 值上调用。
已知结果域为 Type 的条件、match、代码块和调用表达式也可以作为接收者，例如
`(if flag {left} else {right}).struct_field_names()`。接收者照常求值一次，
不为选择反射方法提前执行；若执行接收者时返回，则跳过反射调用。

| 方法 | 返回值 |
| --- | --- |
| `NewtypeRepresentation()` | newtype 的直接表示 `Type` |
| `struct_fields()` | 声明顺序的具名 Type 条目 `Object` |
| `enum_variant_fields()` | 每个 variant 的字段描述 `List<Object>` |
| `struct_field_names()` | 声明顺序的 `List<Symbol>` |
| `struct_field_types()` | 对应的 `List<Type>` |
| `enum_group_paths()` | 分组路径的 `List<List<Symbol>>` |
| `enum_variant_paths()` | variant 路径的 `List<List<Symbol>>` |
| `enum_variant_shapes()` | `.unit`、`.tuple`、`.named` 的 `List<Symbol>` |
| `enum_variant_field_names()` | 每个 variant 的具名字段列表 |
| `enum_variant_field_types()` | 每个 variant 的字段 Type 列表 |

```flurry
struct Row { active: bool, count: u8 }
struct Column where T { first: T, second: T }

comptime fn Columns(subject: Type) -> Type {
    let names = subject.struct_field_names();
    let fields = subject.struct_field_types().map(|field| Column<field>);
    let builder = meta.StructBuilder.new(.name = .Columns);
    let added = builder.add_fields(names, fields);
    assumes added;
    builder.Build()
}

typealias Storage = Columns(Row);
```

struct 与 enum 操作分别检查接收者的结构种类。路径由 Symbol 片段组成，
索引只表示逻辑声明顺序，不是 discriminant 或存储编码。泛型参数在应用后
代入，字段中已有的 normal-form 头保留；反射不授予字段访问或布局权限。
这些方法不提供字段默认代码、属性、方法体或协议实现的复制能力。

`meta.struct_field_names(subject)` 等既有自由函数继续由此模块导出，复用同一
集中绑定和编译器检查。builder 的 `from_type` 使用 Type 方法，不再承载反射
接口的定义。构造完成的新 Type 可使用同样的方法观察。


## 有序字段描述

`subject.struct_fields()` 将字段名与对应 Type 组成一个有序 `Object`。
`subject.enum_variant_fields()` 按 `enum_variant_paths()` 的顺序返回描述列表：
unit 分支为空 Object，tuple 分支使用无名条目，named 分支保留字段名。
空的 tuple/named 分支也会产生空 Object，分支形状仍须从
`enum_variant_shapes()` 取得，不能仅靠字段描述区分。

```flurry
struct Source { item: u32, ready: bool }

comptime fn Selected() -> Type {
    let fields = Source.struct_fields();
    let selected = fields.filter(|entry| entry.key(0) matches .item?);
    let draft = meta.StructBuilder.from_object(
        selected.concat({active: bool}), .name = .Selected) match {
        value? => value,
        null => return bool,
    };
    draft.Build()
}
```

条目值是原始 Type 值，不是类型名字符串；泛型替换后的类型和不透明
normal-form 头保持原身份。接收者只求值一次。返回描述是普通 comptime 值，
后续编辑不会改变源声明或已保存的描述。它们不复制默认代码、属性、可见性、
存储编码或方法，也不授予访问源字段的权限；新定义仍经过 builder 的发布检查。

这两个接口使用已有反射和 Object/List 操作在库中组合实现，没有新增编译器
特判。描述构造仍受 invocation 预算约束，不承诺零拷贝。真实 sysroot 的反射、
筛选和生成定义用例见 `development/examples/sysroot-reflected-field-objects.fl`。


验证（工作目录 `/home/fin/luna`）：

- `python3 -B development/check.py --timeout 900 -- test -p luna --test sysroot_preludes shipped_reflection_objects_feed_checked_definition_builders`：通过，`/tmp/luna-tools-4vvs3v5u/result.json`。覆盖真实 sysroot、泛型与 opaque 字段、枚举字段形状、描述快照及生成结构体的使用。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test source_reflection ordered_field_objects`：通过，`/tmp/luna-tools-5y48gdzf/result.json`。错误接收者返回 Type 错误，失败后合法 invocation 可继续执行。


## newtype 的显式表示观察

`subject.NewtypeRepresentation()` 返回已完成 newtype 实例的直接表示类型。
formation 契约在观察前检查，参数按实例代入；例如 `Wrapper<bool>` 的表示
是 bool，而 `Wrapper<Other>` 的表示是 Other，不继续展开 Other。普通 struct、
enum、primitive 和 opaque normal-form 头不能冒充 newtype。

返回值是 Type 元数据，可以描述 runtime 指针，不会因此在 comptime 中创建
指针值。观察不改变类型身份、成员查找或 trait 证据。`NewtypeBuilder.from_type`
使用这一接口复制表示，随后 Build 发布新身份。
