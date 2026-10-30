# 受检定义构造

structs.fl、enums.fl 包含已接通的 builder；结构反射接口及 Type 固有方法位于 ../reflection/structure.fl。草稿到 Type 的发布必须经过编译器检查；尚未交付的 fn/trait/impl builder 不用占位实现伪装。builder 复用 Type 的受检反射方法。

## Object 字段描述

`StructBuilder.from_object(fields, .name = .Generated) -> ?StructBuilder`
接受一个有序 Object，每个条目必须有字段名，且存储的值必须属于 Type 域。
合法输入返回草稿；无名条目或非 Type 值返回 null。空 Object 合法。

`draft.add_fields_from_object(fields) -> bool` 把同样的描述追加到现有草稿。
它先收集完整的名字／Type 列表，再修改草稿；结构不符时返回 false，草稿保持
原样。`?Type`、布尔值或包含 Type 的另一个 Object 都不会隐式解包为字段 Type。
输入仍是普通值快照，已保存的 builder 快照也不受后续追加影响。

```flurry
comptime fn Record() -> Type {
    let fields: Object = {enabled: bool, count: u32};
    let draft = meta.StructBuilder.from_object(fields, .name = .Record) match {
        value? => value,
        null => return bool,
    };
    draft.Build()
}

typealias RecordType = Record();
```

读取按条目索引进行，保留字段顺序、重复名以及完整 Type 身份，不使用“最后同名
条目”的 `get` 语义，也不展开 normal-form 头。重复名与不合法存储类型仍由
`Build()` 的受检发布拒绝；from_object 成功只说明字段描述形状有效，不代表已
发布合法定义。底层错误和预算耗尽继续传播，不转换成 false 或 null。

这是已有具体 struct 发布路径的普通库组合，不支持字段默认表达式、泛型子句、
函数体或 trait／impl 的生成。贯通示例在
`development/examples/sysroot-object-struct-builder.fl`：从 Object 生成 Type，
反射校验后构造 runtime 值并执行 native 测试。

## 泛型函数中的定义描述数据

定义描述可以先保存在普通 struct、enum、可选值或嵌套 List 中，不必为了携带
Type 而改用类型列表。显式 comptime Type 参数经过描述函数后仍保留调用者的
绑定身份；它既不会提前变成具体类型，也不会因装进聚合值而丢失依赖。

```flurry
struct Descriptor { kind: Type }

comptime fn describe(T: Type) -> meta.List<Descriptor> {
    [Descriptor {kind: T}]
}

fn preserve(comptime T: Type,
    comptime .schema: meta.List<Descriptor> = defer describe(T), value: T) -> T
{
    value
}
```

调用 `preserve(u32, 23)` 时，schema 按普通默认实参规则生成，随实例化固定，
不会成为 runtime 数据。完整 sysroot 示例为
`development/examples/sysroot-scoped-descriptors.fl`。这提供泛型描述数据的
传递能力；尚不意味着 StructBuilder 已能发布带泛型子句的定义。
