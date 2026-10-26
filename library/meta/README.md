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
```

`Build` 经过普通定义检查后才发布类型，重名字段会被拒绝；字段 Type
保留 normal-form 头。空草稿可以发布空结构体，名称相同不代表类型身份相同。
草稿及其中的 List 数据不能成为 runtime 接口。

可以通过现有 `--invoke-comptime` 执行构造入口，单文件、显式包集合和 sysroot
路径都会安装发布服务；显式加载 meta 时仍需绑定 `--meta-list-type meta.List`。
驱动的 `compile_and_invoke_with_struct_builders` API 还会同时返回调用值和含生成
定义的检查后 NIR。初始 elaboration 中的 typealias 计算尚未安装发布服务。
字段默认代码、属性、泛型子句、方法和 impl
的构造尚未接入；这里还不是完整 SoA 示例所需的全部 builder 能力。
