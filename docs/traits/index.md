# Trait 与 implementation

## Intro

Trait 是一种名义接口抽象，借鉴了 Haskell 类型类和 Rust trait 的设计经验。与结构化类型匹配（duck typing）不同，名义接口要求实现者显式声明对接口的支持，从而让编译器在定义端而非使用端发现不兼容问题。

一个函数可以要求参数提供某个操作，而不必知道参数的具体类型。Trait 把这种要求写成接口；implementation 则说明某个类型怎样兑现接口。

对 C 开发者，可以先把 trait 想成一组有名字、有契约的操作。使用接口不要求每个对象都保存函数指针：类型和实现已确定时，编译器可以直接调用；需要在运行时选择具体类型时，再使用动态接口。

## 语法

~~~text
trait_def -> (pure comptime)? trait id (:- expr)? clauses? { (assoc_decl | definition | statement)* }
impl_trait_def -> comptime? impl expr for expr clauses? { (assoc_decl | definition | statement)* }
impl_def -> comptime? impl expr clauses? { (definition | statement)* }
derive_def -> comptime? derive expr for expr clauses?
trait_bound_decl_clause -> id :- expr
type_cast -> expr.as(expr)
dyn_cast -> expr.dyn(expr)
~~~

`expr`、参数与子句沿用[全局语法](../grammar/syntax)。`where T :- Reading` 声明参数 T 必须提供 Reading；`value.as(Reading)` 明确选择接口来源。

父 trait 与 trait bound 都允许用 `A + B + C` 同时要求多个接口，例如 `trait Report :- Reading + Named` 和 `where T :- Reading + Named`。否定要求放在子句中，如 `requires not (T :- Disabled)`，不放进这两处的正向能力组合。

## 正式定义

### 三个对象

| 对象 | 例子 | 含义 |
|---|---|---|
| Trait 声明 | `Reading` | 接口的声明身份、参数与成员要求 |
| 完整接口实例 | `Encode<.json>` | Trait 声明及其全部语义实参 |
| Implementation 实例 | `Reading for Gauge` | 某主体类型兑现该接口的实现及参数替换 |

相同方法签名不使两个 trait 成为同一接口。一个实现适用，也不意味着它已交付所有调用所需的运行时依赖。查询可以确定实现声明，随后再从明确的输入和环境取得依赖。

### 阅读路线

| 问题 | 章节 |
|---|---|
| 怎样声明接口、关联项、默认方法和父接口要求？ | [接口声明与契约](declarations.md) |
| 多份实现怎样检索和选择？ | [实现选择与结构索引](implementation-resolution.md) |
| 哪个包可以添加实现，哪些重叠会冲突？ | [包归属与一致性](coherence.md) |
| 实现依赖运行时对象时怎样绑定和传递？ | [替换与依赖交付](../type/substitution-inference.md) |
| 成员怎样选择来源和适配接收者？ | [成员投影](../type/member-projection.md)、[接收者适配](../type/receiver-adaptation.md) |
| Trait 两相、pure comptime trait 与同名方法的阶段优先级是什么？ | [Trait 两相](../comptime/the-comptime-phases-of-traits.md) |
| 怎样定义、指定和检查派生策略？ | [派生与策略](derivation.md) |
| 怎样隐藏具体类型并保留可调用接口？ | [动态接口](dynamic-interfaces.md) |
| 运算、字面量和 handler 怎样使用 trait？ | [语法协议与能力入口](protocols.md) |

### 调用保证

调用者满足 trait 的要求，就应能使用其接口。实现不能增加接口没有要求调用者提供的前提、错误处理能力或效应处理能力，也不能削弱结果保证。实现中的额外依赖必须被合法封闭，或由接口已经表达的环境关系取得。

成员来源在定义检查时固定。实例化可以确定该来源的具体实现和关联项，不重新选择另一个同名接口。结构精度决定同一实现查询中候选的优先级；它不取代不同接口之间的名称消歧。

未限定的 comptime 成员查找另有声明级规则：优先考虑 `pure comptime trait` 的合法实现来源，只有同层唯一才能选定；这不改变显式接口查询，也不允许按结构精度在两个不同 trait 之间选赢家。

## 常见示例

两个设备提供相同读取接口，调用方只依赖接口：

~~~flurry
trait Reading {
    fn read(*self) -> u32;
}

struct Gauge {
    value: u32,
}

struct FixedReading {}

impl Reading for Gauge {
    fn read(*self) -> u32 {
        self.value.*
    }
}

impl Reading for FixedReading {
    fn read(*self) -> u32 {
        100
    }
}

fn show_reading(value: *T)
where T :- Reading
{
    println("reading: {}", value.read());
}

test {
    let gauge = Gauge { value: 27 }
    let fixed = FixedReading {}
    show_reading(gauge.ref);
    show_reading(fixed.ref);
}
~~~

输出 `reading: 27`、`reading: 100`。show_reading 的定义依据 Reading 检查；实例化时分别取得 Gauge 和 FixedReading 的实现。调用无需先把设备转换成动态接口。

若另一个接口也提供 read，可以写 `gauge.as(Reading).read()` 指明来源。方法名相同不能证明它们履行相同契约。

## 与 Rust / Zig / C / C++ 的关键差异

| 语言 | 对应机制与区别 |
|---|---|
| Rust | trait/impl 是直接对应物；Flurry 的实现还可携带运行时依赖，并按归一类型结构精度选择候选。[Rust trait](https://doc.rust-lang.org/reference/items/traits.html) |
| Zig | 无对等的名义 trait 实现体系；可用 comptime 参数和操作检查组织泛型接口。[Zig comptime](https://ziglang.org/documentation/master/#comptime) |
| C | 无对等；函数指针和显式上下文参数可以表达运行时接口，静态调用则由程序员组织。[C11 草案 §6.7.6.3](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf) |
| C++ | concepts 表达要求，模板和虚函数分别服务静态与动态调用；Flurry 用独立 implementation 兑现名义接口。[C++ constraints](https://eel.is/c++draft/temp.constr)、[virtual](https://eel.is/c++draft/class.virtual) |

## 运行时实现设计引导

一次已检查调用至少需要实际入口、接收者和方法使用的环境。静态确定的入口可以直接调用；动态接口交付相应分派信息。仅服务类型检查的事实不必占据运行时空间。

不能把“没有逐对象 vtable”误解为“不需要任何依赖参数”。操作若依赖特定 allocator 或存储对象，该对象必须在使用点可取得。

## 编译器实现引导

接口使用沿同一条检查链传递信息：

1. 解析声明及完整接口实参，确定主体和所需 phase。
2. 在合法实现世界中检索候选，保留主体形态、要求及开放参数。
3. 隔离候选推断，按结构精度决定实现，处理未决和冲突。
4. 检查关联项、默认成员与接口契约。
5. 固定来源和替换，建立运行时依赖取得计划。
6. 检查接收者适配、调用及资源有效性，再降为实际调用。

实现证据应保留声明身份、完整接口实例、替换、要求依据和环境关系。单个“实现了”的布尔值不足以支持关联项投影、动态封装与自动清理。
