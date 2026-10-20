# 两层类型与阶段边界

## Intro

程序在编译时决定类型，在运行时使用由这些类型约束的值。一个类型可以依赖运行时对象，但编译器不因此获得读取未来内存的能力。

理解两层的关键是区分“一个值属于哪一层”和“一个表达式何时被执行”。编译时表达式可以描述运行时程序；描述程序与运行程序是两件事。这种分层借鉴了多阶段编程（multi-stage programming）的思想，在语言层面防止阶段信息意外泄漏，同时允许类型安全的代码生成。

## 语法

~~~text
comptime_block -> comptime { (definition | statement)* }
quote_param -> quote parameter
lift_type -> lift expr
~~~

普通值跨阶段传递不要求统一的显式 splice 表达式。

这些形式的精确操作契约见[跨阶段值传递](../comptime/quoting-and-splicing.md)；本篇只说明它们与类型的关系。Type、Any、any 是值名，使用普通表达式语法。

## 正式定义

### 类型值

Type 描述类型值。类型值属于 comptime 宇宙，不是可以自动存入运行时结构的描述符；因此 Type 没有把自身传入运行时的通用 proxy。

类型表达式可以引用编译时参数、执行合法的编译时计算，并得到其他类型。一个表达式的结果尚未求出时，编译器仍可依据其返回类型和合法结果契约检查后续操作。

### 描述与执行

quote/lift 保留代码或运行时依赖身份。这样的描述不把运行时对象的当前或未来内容变成一个已知编译时常量。splice 根据相应的跨阶段契约把合法产物接回程序；能否 splice 由其所属机制与实现决定。

| 已有信息 | 可以得出的结论 |
|---|---|
| 参数 T 的类型是 Type | T 是类型值 |
| T 的结构事实已成立 | 可以使用该事实许可的结构操作 |
| 某个 runtime 对象有可引用的 identity | 可以保留依赖；不能任意读取其未来值 |
| 某个类型能描述 runtime 值 | 不意味着该类型值本身能作为 runtime 数据传递 |

宇宙判断是类型性质，执行阶段是表达式上下文，两者须分别检查。comptime-only 类型不适用的运行时布局查询，应报告前提不成立，而不是返回“非固定大小”。

### 检查与兑现

ordinary normalization 要求操作依据可获得。loose normalization 可以按其规则保留义务。comptime 函数与块采用 loose；其中定义的 runtime 函数仍按 ordinary 检查自己的主体。实例化代入输入，继续执行和证明，不能交付含有未兑现必要义务的实例。

## 常见示例

下面的工厂在编译时产生一个保存两个同型值的结构。调用者随后在运行时构造并使用这个结构：

~~~flurry
pure comptime fn PairOf(T: Type) -> Type {
    struct Pair {
        first: T,
        second: T,
    }

    Pair
}

typealias Samples = PairOf(u32);

fn show_samples(first: u32, second: u32) {
    let samples = Samples { first: first, second: second }
    println("samples: {}, {}", samples.first, samples.second);
}

test {
    show_samples(12, 18);
}
~~~

输出为 `samples: 12, 18`。`PairOf(u32)` 决定字段类型；传入 `show_samples` 的两个数值决定字段内容。运行时的 `samples` 不需要保存工厂对象或一个 `Type` 字段。

同一个类型值也可以控制另一个类型的构造：

~~~flurry
const Word: Type = u32;
typealias Counter = Word;
typealias CounterPair = PairOf(Counter);

test {
    let counters = CounterPair { first: 3, second: 9 }
    println("counters: {}, {}", counters.first, counters.second);
}
~~~

`Word` 保存编译时的类型值，`Counter` 为该类型命名，`CounterPair` 则是执行类型工厂的结果。这三种声明都不会把类型描述作为计数器的运行时数据。

以下结构试图把 `Type` 当作运行时字段交付，应在需要其运行时表示的位置拒绝：

~~~flurry
struct RuntimeTypeBox {
    stored_type: Type,
}

fn make_type_box() -> RuntimeTypeBox {
    RuntimeTypeBox { stored_type: u32 }
}
~~~

这里缺少的是 `Type` 的运行时交付机制。若实际需求只是生成不同字段类型的结构，第一段类型工厂已经完成了这件事。

## 与 Rust / Zig / C / C++ 的关键差异

Zig 的 comptime type 值是直接类比；Rust 的常量泛型和 C++ 模板参数提供另外两种把静态输入带入类型的方式。Flurry 的阶段契约还允许在参数未知时依据要求检查操作，不能把任一种实例化策略整体搬来。[Rust 泛型参数](https://doc.rust-lang.org/reference/items/generics.html)、[基础对照](index.md#与-rust--zig--c--c-的关键差异)

C 没有对应的类型值计算机制，程序员使用声明、宏和外部生成程序；这不等同于 Flurry 中一个返回 Type 的编译时函数。

## 运行时实现设计引导

运行时代码只应取得已经定义的值、代码入口和环境。检查所有跨阶段边：边上携带的是常量物化结果、生成代码、代理表示还是环境引用，必须明确。

对 Type 的处理不能兜底为传一个内部类型对象地址。若某个运行时操作需要类型信息，其交付机制必须有独立契约，不能借编译器恰好有一个类型描述对象来实现。

## 编译器实现引导

为表达式分别记录宇宙、执行阶段、依赖集合、结果契约和未决义务。执行器只接受当前阶段允许执行的表达式；求解器只消费有合法来源的事实。

每次跨阶段操作按以下顺序处理：

1. 检查输入属于操作的定义域。
2. 验证所需实现或转换契约。
3. 保留必要 identity、绑定和作用域。
4. 在允许阶段产生结果；不能执行时保留受约束结果项。
5. 交付前检查结果保证及所有依赖仍然有效。

缓存键应覆盖影响结果的实际语义输入，尤其是运行时依赖身份和目标配置。阶段不允许的读取、尚未兑现的义务以及求值失败应分开诊断。
