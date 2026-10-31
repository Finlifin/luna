# 精化要求

## Intro

一个类型描述可以只提供部分信息。精化要求（refinement）把使用它时必须成立的性质写出来，使程序能够在尚未知道具体类型时检查操作。

精化不会制造字段、方法或运行时包装。它说明已有对象满足什么；实际操作仍沿合法成员来源和表示契约执行。

## 语法

~~~text
refinement_type -> refines { expr } expr
requires_clause -> requires expr
ensures_clause -> ensures expr
bool_matches -> expr matches pattern
~~~

谓词使用 and、or、not；量词属于布尔公式。反身代词见[Self、Itself、itself](self-reference.md)。

## 正式定义

精化由主体类型描述和关于受检对象的谓词构成。检查对象时绑定局部受检对象，并验证要求。只有成立的事实可以支持后续操作。

requires 是操作前提，ensures 是合法调用提供的结果保证。不得用同一调用尚未获准的结果保证倒证其前提。用户保证须经定义证明，或在准许阶段通过具体执行验证后交付，不能作为免检公理。

| 已建立要求 | 支持的检查 | 独立前提 |
|---|---|---|
| 结构分类 | 分类许可的观察 | 具体字段与元素 |
| 字段及其类型 | 结构操作的类型前提 | 可见性与访问条件 |
| trait 实例 | 能力及关联项 | 具体实例的合法实现 |
| 长度与下标关系 | 带前提的序列操作 | 容器访问契约 |
| 可执行 predicate | 该命题与准入推导 | 完整符号求解能力 |

同名成员不代表同一来源。多个事实支持不同来源时，以显式实现视角或更精确要求确定操作。

本篇精化提供类型检查的能力与性质，不自动建立关于任意运行时内存变化的程序验证逻辑。

## 常见示例

给“只接收 tuple”写出实际参数要求，再分别传入不同形状的元组：

~~~flurry
fn accept_tuple(value: refines {
    Type.predicates.is_tuple(Itself)
} Any) -> bool {
    true
}

test {
    println("pair: {}", accept_tuple((1u32, true)));
    println("triple: {}", accept_tuple((1u32, 2u32, 3u32)));
}
~~~

两次都输出 `true`。函数要求的是 tuple 分类，没有限制长度，也不因此获得访问某个特定元素的依据。`accept_tuple(1u32)` 则不满足参数要求。

前提与结果保证可以在编译时计算中接成一条具体调用链：

~~~flurry
pure comptime fn next_index(index: Integer) -> result: Integer
where ensures result == index + 1 {
    index + 1
}

pure comptime fn checked_index(index: Integer, length: Integer) -> Integer
where requires 0 <= index and index < length {
    index
}

pure comptime fn next_checked_index(index: Integer, length: Integer) -> Integer
where requires 0 <= index and index + 1 < length {
    let next = next_index(index);
    checked_index(next, length)
}

test {
    const NEXT: usize = next_checked_index(2, 4).as(usize);
    println("next index: {}", NEXT);
}
~~~

输出为 `next index: 3`。`next_index` 的保证说明结果与原输入的关系；调用方再结合自己的前提，证明传给 `checked_index` 的数值合法。这里使用编译时数学整数，推理不偷用机器整数可能溢出的运算等式。

调用 `next_checked_index(3, 4)` 不满足要求，应被拒绝。不能先假设 `checked_index` 已经成功，再用这个假设反过来证明输入在界内。

## 与 Rust / Zig / C / C++ 的关键差异

C++ 约束、Rust trait bound 都有能力检查用途；Flurry 还通过 PTC 组合结构、整数和行等性质。C 使用显式协议，Zig 可执行编译时检查；这些手段不能替代 PTC 的具体规则。[C++ 约束](https://eel.is/c++draft/temp.constr)、[Rust bound](https://doc.rust-lang.org/reference/trait-bounds.html)

## 运行时实现设计引导

静态事实不要求运行时证据对象；操作所需环境、实现入口和 handler 路由仍须交付。证明失败不能默默改成运行时断言：运行时检查必须有对应契约。

## 编译器实现引导

1. 确定受检对象是类型还是值，建立代词绑定。
2. 检查谓词良定义；部分观察先证明前提。
3. 将准入公式交 PTC，保留来源、依赖及阶段。
4. 成功路径加入事实；模式绑定随成功证据产生。
5. 实际操作另保留成员来源、签名与适配。
6. 按 ordinary/loose 规则处理残余，交付前兑现。

嵌套精化不能覆盖词法 Self。缓存包含实际受检对象；同一文本用于不同对象不是同一查询。
