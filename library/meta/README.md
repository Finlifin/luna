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
