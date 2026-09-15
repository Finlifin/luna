# 数据结构

数据声明、构造和表示的完整规则见类型系统各篇：

| 内容 | 入口 |
|---|---|
| 命名字段、默认值和声明顺序初始化 | [struct](../type/user-defined-types/structs.md) |
| 封闭变体、载荷与模式编码 | [enum](../type/user-defined-types/enums.md) |
| 共享存储与 C ABI | [union](../type/user-defined-types/unions.md) |
| 独立身份与别名 | [newtype / typealias](../type/user-defined-types/newtype-and-typealias.md) |
| 结构查询、大小和字段位置 | [反射与布局](../type/reflection-and-layout.md) |
| 形状与类型计算 | [静态形状张量](../comptime/static-shaped-tensors.md) |
| 索引与共享存储环境 | [运行时依赖](../type/runtime-dependent-type.md) |

~~~flurry
struct Point {
    x: f64,
    y: f64,
}

let origin = Point { x: 0.0, y: 0.0 }
~~~

字段构造项使用完整的 id: expr。其他容器遵循各自类型与操作契约；类型可以参与计算，不由此规定所有数组或切片 API。
