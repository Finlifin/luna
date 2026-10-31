# 类型系统

## Intro

类型决定一个值可以怎样使用，也决定程序需要准备怎样的表示。Flurry 允许程序在编译时计算类型；检查一段程序时，编译器还能使用关于类型的已知事实，而不必先把每个类型参数算成具体结果。

从 C 迁移时，可以先分清三个问题：这是不是同一个类型，这个值能否交给某个操作，这个值需要怎样的存储。三个问题有关联，却没有同一个答案。两个大小相同的结构体仍可以是不同类型；一个指针的大小已经确定时，其指向对象的大小仍可以依赖运行时信息。

## 语法

以下是阅读入口，具体产生式随各篇给出；`expr`、`parameter`、`clauses` 同[全局语法](../grammar/syntax)。

~~~text
typealias_def -> typealias id (<parameter*>)? = expr
newtype_def -> newtype id (<parameter*>)? = expr
pointer_type -> *expr
optional_type -> ?expr
error_type -> !expr expr
effect_type -> #expr expr
closure_type -> %expr expr
refinement_type -> refines { expr } expr
~~~

## 正式定义

### 阅读地图

| 要回答的问题 | 章节 |
|---|---|
| 类型怎样参与计算，与运行时怎样交界？ | [两层类型](2-level-type.md) |
| 声明、别名、工厂产生的类型怎样比较？ | [类型身份与关系](type-identity.md) |
| Any、参数化与分支汇合怎样检查？ | [参数化与上下文检查](comptime-gradual-dependent-type.md) |
| 约束能够提供哪些事实？ | [精化要求](refined-type.md)、[PTC](propositional-constraints/index.md) |
| 编译器具体能证明什么？ | [原子目录](propositional-constraints/atoms.md)、[理论求解器](propositional-constraints/theory-solvers.md)、[组合与诊断](propositional-constraints/solver-protocol.md) |
| 类型模式能够观察什么？ | [类型模式](type-patterns.md)、[出现位置事实](propositional-constraints/occurrence-typing.md) |
| 字段、大小与表示怎样查询？ | [反射与布局](reflection-and-layout.md) |
| 成员从哪里来，三个反身代词指什么？ | [成员投影](member-projection.md)、[Self / Itself / itself](self-reference.md) |
| 为什么有些调用会自动取址或转换？ | [接收者适配与隐式转换](receiver-adaptation.md) |
| 实现参数如何恢复，运行时依赖如何交付？ | [替换与依赖交付](substitution-inference.md)、[Trait 总览](../traits/index.md) |
| 怎样定义数据？ | [struct](user-defined-types/structs.md)、[enum](user-defined-types/enums.md)、[union](user-defined-types/unions.md)、[newtype 与 alias](user-defined-types/newtype-and-typealias.md) |
| 缺失、失败与效应怎样进入类型？ | [optional](qualified-types/optional-type.md)、[error](qualified-types/error-type.md)、[effect](qualified-types/effect-type.md)、[两类行](rows.md)、[handler 类型契约](handlers.md) |
| 值依赖运行时对象时怎样传递？ | [运行时依赖](runtime-dependent-type.md)、[闭包环境](qualified-types/closure-type.md) |

### 四条贯穿全章的规则

1. 类型值属于编译时。计算类型可以观察编译时输入，也可以保留对运行时对象的依赖；保留依赖不等于读取未来的运行时值。
2. 操作需要的事实必须有依据。声明、已成立的要求、类型结构、合法实现和已验证契约都可以提供依据；信息不足不能自行补成肯定或否定。
3. 相等、匹配、相容、子类型和实际转换分别定义。某次调用接受了一个值，不因此证明源类型和目标类型相等。
4. 程序实例交付时，需要的类型、操作来源、表示和阶段义务必须兑现。编译器在定义端保留未知信息，并不授予程序在运行时猜测这些信息的能力。

定宽编码教学见[ADD/SUB](user-defined-types/fixed-width-enum-example.md)，从具体位表推导互斥和覆盖；参数化计算教学见[静态形状张量](../comptime/static-shaped-tensors.md)。

资源有效性、安全等级和 trait 实现选择有各自的契约。类型中携带的信息可以参与这些检查；单独存在一个类型描述，不能代替它们所要求的证据。

## 常见示例

下面的程序同时用到声明身份、显式转换、命名构造和 optional。屏幕坐标与世界坐标具有相同直接表示，但接口有意区分二者：

~~~flurry
struct Position {
    x: i32,
    y: i32,
}

typealias ScreenPosition = Position;
newtype WorldPosition = Position;

fn locate_marker(visible: bool) -> ?ScreenPosition {
    if visible {
        ScreenPosition { x: 12, y: 8 }
    } else {
        null
    }
}

fn show_world_position(position: WorldPosition) {
    let raw = position.as(Position);
    println("world position: ({}, {})", raw.x, raw.y);
}

fn show_marker(visible: bool) {
    locate_marker(visible) match {
        position? => {
            show_world_position(position.as(WorldPosition));
        },
        null => println("marker hidden"),
    }
}

test {
    show_marker(true);
    show_marker(false);
}
~~~

输出为 `world position: (12, 8)`、`marker hidden`。`ScreenPosition` 只是已有类型的名字；`WorldPosition` 有新身份，因此调用中显式转换。这里约定两套坐标的数值映射为恒等，cast 本身不会计算平移或缩放。

`locate_marker` 的返回目标使结构值进入 optional，匹配再取出完整的坐标值。整个过程不需要把 `Type` 存入对象，也不需要让缺失值伪装成特殊坐标。

可沿下列代码路线继续阅读：

| 想实际写出的程序 | 示例入口 |
|---|---|
| 用编译时输入生成字段类型，再构造运行时值 | [PairOf 类型工厂](2-level-type.md#常见示例) |
| 查询数组头参数并取得长度 | [类型模式](type-patterns.md#常见示例) |
| 为索引调用建立可证明的边界 | [PTC 契约链](propositional-constraints/index.md#常见示例) |
| 从原始指令字绑定寄存器编号 | [ADD/SUB 解码](user-defined-types/fixed-width-enum-example.md#常见示例) |
| 区分没有修改、清除配置与设置配置 | [嵌套 optional](qualified-types/optional-type.md#常见示例) |
| 观察同名效应的内外处理顺序 | [handler](handlers.md#常见示例) |
| 用整数索引访问指定存储的一列 | [NodeIndex](runtime-dependent-type.md#常见示例) |

## 与 Rust / Zig / C / C++ 的关键差异

| 对照机制 | 阅读 Flurry 时需要调整的预期 |
|---|---|
| C 的声明类型、typedef 和显式数据表示 | 不要把类型计算理解成预处理文本替换；其结果参与正式的类型检查。[C11 草案 §6.7](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf) |
| Rust 的名义数据类型与泛型 | 可以类比声明身份；Flurry 的 Type 参数本身不附带额外布局或 trait 事实。[Rust 类型](https://doc.rust-lang.org/reference/types.html) |
| Zig 的编译时 type 值 | 类型可以由计算得到这一点接近；Flurry 还在未知输入下依据要求与结果契约检查操作。[Zig 0.15.2：comptime](https://ziglang.org/documentation/0.15.2/#comptime) |
| C++ 模板与约束 | 约束是有用的类比；PTC 的具体原子、阶段兑现和 Unknown 结果要按 Flurry 的契约理解。[C++ 约束](https://eel.is/c++draft/temp.constr) |

## 运行时实现设计引导

运行时需要交付值表示和操作所需环境，不需要把完整 Type 或 PTC 求解器带入程序。

对每个交付位置建立清单：主体表示、所需环境、调用入口、实际转换、错误或效应路由。全部静态确定的内容可直接降为目标程序；需要运行时取得的环境必须由有效的承载位置提供。禁止以一个未定义的通用动态盒子替代无法兑现的项目。

零额外环境、共享环境、逐值环境是不同的布局选择。优化可以消除冗余存储，但必须保持相同的值含义、对象依赖和处理器路由。

## 编译器实现引导

建议将类型检查组织为以下可独立验证的契约：

1. 建立类型项与声明身份，保留需要后续实例化的输入。
2. 从词法环境和当前路径收集带来源的事实；按具体操作提出证明目标。
3. 记录成员来源、关联项证据和转换动作；不要把这些信息压成一个兼容性布尔值。
4. 推进类型计算与求解；对未决项保留阶段、作用域和依赖。
5. 实例化时替换允许替换的参数，继续兑现义务。
6. 在表示交付点核验布局、环境和路由，输出可执行的表示计划。

检查失败应指出缺的是哪一项契约。类型不相等、要求未证明、布局非法与求解预算耗尽应当能被区分。
