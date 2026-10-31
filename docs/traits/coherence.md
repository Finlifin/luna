# 包归属与实现一致性

## Intro

在开放的包生态中，任何包都可以为任何类型添加接口实现，这会导致不同包提供互相冲突的实现。一致性检查（coherence）通过限制实现的归属权和检测重叠来防止这一问题，从而保证程序在任何合法的依赖组合下都能得到唯一确定的实现。

一个包可以给自己的接口或类型补充实现。第三方扩展也需要明确的归属依据，否则两个独立库可能各自合法，却在使用者处提供互相冲突的实现。

包归属决定能否声明实现；一致性检查决定这些实现能否共同存在。满足前者并不免除后者。

## 语法

~~~text
impl_trait_def -> comptime? impl expr for expr clauses? { (assoc_decl | definition | statement)* }
impl_def -> comptime? impl expr clauses? { (definition | statement)* }
newtype_def -> newtype id(<parameter*>)? = expr
typealias_def -> typealias id(<parameter*>)? = expr
~~~

包和名称导入沿用模块规则，本篇不增加实现导入语法。

## 正式定义

### 定义归属

名义类型和 trait 声明具有定义包。alias 保留原身份，不因重命名变成本地定义；newtype 建立新的本地身份。数值、字符串、Symbol 等可由多个包构造的普通值，本身不能授予实现归属权。

Inherent implementation 属于主体名义类型的定义包。Trait implementation 若实现本地 trait，可以在该包定义；否则按下述有序本地身份规则检查。

### 有序本地身份规则

对外部 trait，检查 normalized 主体，然后检查 trait 的显式实参，后者按声明参数顺序排列。每项按已暴露结构从外到内、从左到右观察：

1. 必须遇到一个归当前包所有的名义定义身份。
2. 到达这个本地身份以前，不得遇到仍可自由替换的量化参数或未确定的结构。
3. 到达本地名义头后，该头内部的参数不受“本地身份之前”的限制；后续位置也可以量化。
4. 外部包装可以包含本地身份，但不会因此成为本地定义；继续观察其参数时仍执行同一顺序检查。

只考虑实际归一结果中具有语义身份的项。调用一个本地工厂却得到外部类型，不会仅因源码出现本地函数名就满足规则。要求中的等式不能把本来不合归属规则的开放头追认成本地；声明应把决定归属的形态写进可观察的头。

例如外部 `Pair` 中，`Pair<Local, T>` 在 Local 的包可通过；`Pair<T, Local>` 在到达 Local 前遇到自由 T，不能通过。`Local<T>` 则先遇到本地头，可以量化其内部 T。

这保持“冲突有可追踪的定义责任”，而不承诺任何两次库版本升级都能兼容。下游仍必须检查依赖合流后的一致性。

### 可见世界与冲突

合法依赖图和静态作用域共同决定实现世界。普通 use 控制名称访问，不隐藏世界中已存在的冲突；选择方法时不能靠导入顺序改变 implementation 优先级。

同一主体和完整接口实例的同精度实现，必须证明共同适用范围为空。无法证明互斥时，不能以“尚未找到反例”接受最终实现世界。不同精度按[实现选择](implementation-resolution.md)处理。两个 phase 参与共同的一致性关系，具体规则见[两相](../comptime/the-comptime-phases-of-traits.md)。

派生产物执行同样的归属和冲突检查。默认方法属于同一份实现，不作为第二份实现参与竞争。

静态定义和 loose 中生成的定义仍遵守各自可见性与最终检查边界。已经交付的实现证据不能通过另一次名称导入被换成其他实现。

## 常见示例

本地适配类型提供接口，调用方使用明确视角：

~~~flurry
trait Code {
    fn code(*self) -> u32;
}

newtype LocalWord = u32;

impl Code for LocalWord {
    fn code(*self) -> u32 {
        16
    }
}

fn show_word(word: u32.as(LocalWord)) {
    println("code: {}", word.code());
}

test {
    show_word(7);
}
~~~

输出 `code: 16`。若 Code 来自依赖包，LocalWord 的定义包仍可提供这份实现。调用中的视角只为该值提供适配路径，不会向所有 u32 注入 Code 实现。

下面用独立包片段展示顺序检查。base 定义：

~~~flurry
pub trait Label {
    fn label(*self) -> str;
}

pub struct Pair
where A, B
{
    first: A,
    second: B,
}
~~~

在只依赖 base 的包 a 中，假定已导入 Pair、Label：

~~~flurry
struct LocalA {}

impl Label for Pair<LocalA, T>
where T
{
    fn label(*self) -> str {
        "a"
    }
}
~~~

另一个只依赖 base 的包 b 不能声明下列实现：

~~~flurry
struct LocalB {}

-- 拒绝：到达 LocalB 前已经遇到自由 U。
impl Label for Pair<U, LocalB>
where U
{
    fn label(*self) -> str {
        "b"
    }
}
~~~

否则两包合流后，`Pair<LocalA, LocalB>` 会同时匹配两份同精度实现。拒绝的是包 b 的归属，不需要等到下游构造该实例才发现问题。片段中的包身份由各文件所属包决定，不能把三个片段放入同一包验证这一拒绝理由。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 也有孤儿与重叠检查，但其 covered parameter、fundamental type 等具体规则不等于本篇有序遍历规则；Flurry 按归一后的已暴露结构确定本地依据。[Rust implementations](https://doc.rust-lang.org/reference/items/implementations.html)

C 没有相应 trait 注册体系；函数指针表由使用者显式组合。Zig 的 comptime 代码也不提供这套名义实现归属规则。C++ 的模板与约束选择有自己的声明和实例化限制，不采用本篇包级准入算法。[C11 §6.7.6.3](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf)、[Zig comptime](https://ziglang.org/documentation/master/#comptime)、[C++ constraints](https://eel.is/c++draft/temp.constr)

## 运行时实现设计引导

包归属和冲突在编译时检查；运行时不通过加载顺序裁决它们。分派表与静态调用都必须引用已验证的实现身份。

若构建过程改变了实现世界，相关实例与动态入口表需要重新验证，不能仅因函数签名未变而复用旧选择。

## 编译器实现引导

1. 给名义声明保留定义包和稳定声明身份；展开 alias，保留 newtype 身份。
2. 归一实现头到允许观察的形态，生成有序归属遍历。
3. 本地 trait 直接通过归属检查；外部 trait 检查首个本地依据及其之前的开放项。
4. 在共同实现世界中，用结构索引找出可能相交的同精度声明对。
5. 对声明参数分别改名，建立头相等、接口相等和双方要求的合取，查询可满足性。
6. 不可满足则允许共存；可满足则报告共同实例；未决按定义检查边界保留，最终仍未决则拒绝。
7. 对派生产物、开放定义完成后的世界执行同一检查。

不同接口实例不会仅因方法同名进入 overlap 检查；同名消歧是成员规则。实现内部开放环境参数也不能当作区分同一主体两份实现的额外身份。

固定头的归属遍历为 O(n)。同桶有 k 份实现时，朴素冲突检查可能需要 O(k²) 对查询；可用兼容结构进一步过滤，但不能用索引不相同替代已定义的互斥证明。
