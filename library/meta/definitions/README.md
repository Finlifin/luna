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

描述预处理可以复用 Object 的 `filter`、`flat_map` 和 `fold`：例如先保留具名
Type 条目，再按策略展开字段，最后交给 from_object 和 Build。回调接收单条目
Object，保留其值域与重复键；筛选成功不会绕过发布时的重名和存储合法性检查。
见 [Object 遍历接口](../data/README.md#object-逐条遍历与变换)及
`development/examples/sysroot-object-traversal.fl`。

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

Type 参数也可供 requires 中的描述函数调用使用，例如先声明
`pure comptime fn Layout<fields: meta.List<Descriptor>> -> Type { bool }`，
再约束 `Layout<describe(T)> == Layout<[Descriptor {kind: T}]>`。
这里比较的是两个 Type；描述数据只是构造 Type 的输入，不会把程序状态命题
交给 PTC。不同描述实参也不能反过来证明不透明工厂的结果不等。

## Object 描述的 enum 具名 payload

`EnumBuilder` 与 `StructBuilder` 共享有序字段描述解析，不需要为 enum 手工
维护名字／Type 两份列表：

- `add_named_from_object(name, fields, .group = []) -> bool` 追加一个具名 variant。
- `insert_named_from_object(index, name, fields, .group = []) -> bool` 在声明顺序位置插入。
- `replace_named_from_object(index, name, fields, .group = []) -> bool` 替换指定 variant。

```flurry
comptime fn EventType() -> Type {
    let draft = meta.EnumBuilder.new(.name = .Event);
    draft.add_group(.net);
    let accepted = draft.add_named_from_object(.packet,
        {ready: bool, count: u32}, .group = [.net]);
    assumes accepted;
    draft.Build()
}

typealias Event = EventType();
```

所有条目必须具名且值属于 Type 域。位置项、普通值和 `?Type` 值返回 false；
非法插入／替换位置也返回 false。解析和范围检查在修改草稿前完成，失败保留
原有 variant、payload 和分组，允许随后修复；已经保存的草稿快照不受影响。
空 Object 生成空的 **named** payload，不自动改为 unit variant。

描述保留字段顺序、重复名字和 normal-form Type 身份。成功返回只表示草稿
编辑完成；Build 仍检查重复字段、分组路径和 runtime 存储合法性。不会自动
创建 `.group` 指定的分组，也不把发布错误或预算耗尽改成 false。
共享解析位于 `definitions/fields.fl`，没有另加原始 builtin 访问或编译器特判。

真实 sysroot 示例：`development/examples/sysroot-object-enum-builder.fl`。
运行 `python3 -B development/library_check.py --stage sysroot-object-enum-builder`
可检查生成类型的构造、空 named payload 和带分组的 pattern matching。
