# Trait 的 comptime/runtime 两相

## Intro

一个类型在编译时和运行时可以提供不同的操作。Trait 描述这些操作的要求，方法声明则标明它在哪个阶段使用。

检查一次调用时，既要知道对象满足什么契约，也要知道当前操作属于哪个阶段。

## 语法

以下复用[全局语法](../grammar/syntax)中的定义、参数、子句和关联类型。

```text
trait_def -> trait id (:- expr)? clauses? { (assoc_decl | definition | statement)* }
impl_trait_def -> impl expr for expr clauses? { (assoc_decl | definition | statement)* }
comptime_method -> pure? comptime fn id(parameter*) (-> result)? clauses? block?
runtime_method -> fn id(parameter*) (-> result)? clauses? block?
```

Trait 内可以只有签名而没有函数体。`comptime` 参数与 `comptime` 方法是不同标记：普通 runtime 方法也能含有 comptime 参数。

## 正式定义

### 方法所属阶段

comptime 表达式调用 trait 的 comptime 相，runtime 表达式调用 runtime 相。两相可以分别实现；没有提供的相不能因另一相存在就视为可用。

这不授予同一作用域任意重名定义的能力。Flurry 不通过函数重载来区分本应唯一的同名定义；trait implementation 的匹配与 coherence 是另一项检查。

### 使用跨阶段结果

runtime 代码可以使用 comptime 结果，条件是该结果满足相应的跨阶段规则。它并不直接把一个 runtime 方法改成 comptime 方法，或反过来。

例如，`Splice.splice` 的输入带有 comptime 标记，方法自身仍在 runtime 执行。其输入用于实例化，输出是 runtime 目标值。

### 候选与 coherence

Trait 的 comptime/runtime 两相共享 implementation 候选集与 coherence。不能仅因调用阶段不同，就为重叠实现建立互不关联的冲突检查集合。

静态作用域内已有 implementation 先完成相应检查。loose 中生成的开放定义按线性可见性使用，在该环境整体实例化时接受最终 coherence 检查。详细边界见[两阶段检查](./normalization-and-instantiation.md#开放定义)。

### Eq、Hash 与 Splice

| Trait | 在编译时计算中的作用 |
|---|---|
| Eq / Hash 的 comptime 相 | 参与编译时值的 query 缓存比较与键处理 |
| Splice | 用 comptime 输入实例化 runtime 转换方法 |

Eq/Hash 的缓存作用不把它们等同于 PTC 逻辑推理。Splice 的有限自动派生规则见[跨阶段值传递](./quoting-and-splicing.md#有限自动派生)。

## 常见示例

下面的教学 trait 将编译时元数据操作与运行时读取操作分别命名；`Sample` 是本例定义，不是标准库 API。

```flurry
trait Sample {
    comptime fn marker() -> Integer;
    fn sample(*self) -> i32;
}
```

`marker` 是 comptime 方法，`sample` 是 runtime 方法。实现哪一相与具体方法如何工作，由 implementation 提供；调用者不能用其中一相替代另一相。

作为另一种参数安排，Splice 的方法具有 comptime 参数，但它的函数声明没有 comptime 修饰，因此属于 runtime 方法。这一判断只需要看声明，不需要先试执行一次方法体。

## 与 Rust / Zig / C / C++ 的关键差异

| 对照语言 | 对应机制与差异 |
|---|---|
| Rust | trait 可包含方法、关联类型与关联常量；关联常量是提供类型相关常量的实际手段。Flurry 另按 comptime/runtime 方法阶段组织调用。[Rust traits](https://doc.rust-lang.org/reference/items/traits.html) |
| Zig | 无对等的 trait 两相体系；实际用 comptime 参数与编译时 duck typing表达所需操作。[Zig 参考](https://ziglang.org/documentation/master/#Compile-Time-Parameters) |
| C11 | 无对等的 trait；接口可用函数表和约定实现，类型相关元数据可另设常量。[C11 草案 §6.7.6.3](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf) |
| C++ | concepts 与受约束模板表达操作要求，成员函数可有 constexpr/consteval 限定；Flurry 两相共享 implementation coherence。[C++ constraints](https://eel.is/c++draft/temp.constr)、[constexpr](https://eel.is/c++draft/dcl.constexpr) |

## 运行时实现设计引导

1. 为已检查的 runtime 方法调用使用对应实例及其调用约定。
2. 擦除仅用于实例化的 comptime 参数，保留 runtime 参数与返回值。
3. runtime 方法所需的常量、类型布局或代码结构必须已由编译时过程确定。

不能因一个 trait 有 comptime 相就为普通调用引入运行时 comptime engine；也不能因共享 coherence 就假定所有 trait 调用都有同一种派发方式。运行时派发形式属于具体调用与 trait 表示规则。

## 编译器实现引导

1. 在方法声明上保存所属阶段，分别于参数的阶段标记。
2. 名称解析确定相应定义；按调用的阶段检查方法可用性，不把试执行失败作为选择另一相的方法。
3. 在共享的 implementation 体系中检查适用条件及 coherence，避免两相各自产生不一致的实现选择。
4. 为 comptime 调用建立求值依赖；为 runtime 调用建立相应实例及结果契约。
5. loose 生成实现时，记录其可见性与整体实例化检查关系；最终冲突时拒绝依赖该结果的实例。

不变量：缺少某相的实现是缺少对应操作，不能通过忽略方法阶段来补足。Eq/Hash 的特殊处理必须维持正常的比较契约；Splice 必须进入普通 runtime 方法实例化路径。

复杂度取决于实现候选与约束求解。共享 coherence 要求的是相同的检查契约，不要求重复执行同一检查；缓存必须随其依赖正确失效。
