# 受检反射

`structure.fl` 为 builtin `Type` 提供 comptime 固有方法。通过真实 sysroot
加载 meta 后，可以直接在命名类型、泛型应用或局部 Type 值上调用。
已知结果域为 Type 的条件、match、代码块和调用表达式也可以作为接收者，例如
`(if flag {left} else {right}).struct_field_names()`。接收者照常求值一次，
不为选择反射方法提前执行；若执行接收者时返回，则跳过反射调用。

| 方法 | 返回值 |
| --- | --- |
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
