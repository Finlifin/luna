# 参数化、Any 与上下文检查

## Intro

编写参数化函数时，作者可以先规定操作需要什么，再让调用提供具体类型。信息可以逐步变得精确，但一次操作不能在缺少依据时自行成立。

Any 表示任意类型，any 表示任意 comptime 值。它们参与计算与检查；把一个描述放宽到 Any，并不会自动创建运行时动态分派。

## 语法

~~~text
fn_def -> fn id(parameter*) (-> expr)? clauses? { statement* }
clauses -> where clause*
type_decl_clause -> id
type_bound_decl_clause -> id : expr
trait_bound_decl_clause -> id :- expr
requires_clause -> requires expr
normal_form_head_application -> expr<argument*>
~~~

这是与本篇相关的局部语法；参数种类、函数前缀与其他子句见[函数](../functions/index.md)和[comptime](../comptime/purity-and-normal-form.md)。

## 正式定义

### 信息与使用要求

Any 是任意类型的描述；any 是任意 comptime 值的描述。相容关系允许检查接纳相应未知描述，但不把各具体类型或值合并为同一个东西。

只知道 T: Type，不自动获得固定大小、某个 trait 或某个字段。后续操作所需的事实可以来自声明、当前要求、合法结构观察或证明。没有写出一个要求，并不禁止编译器推导它；没有推导出它，也不能默认为真。

含 Any 的 runtime 参数在调用点实例化。实例获得具体输入之后继续检查；调用点单态化不授权定义端任意使用未知成员。ordinary 与 loose 的边界遵循[normalization](../comptime/normalization-and-instantiation.md)。

### 分支与目标上下文

无目标上下文时，同型分支汇合为该类型，异型分支汇合为 Any。没有自动合成一般 union 或公共父类型的规则。

存在目标类型时，先把目标要求传入分支，按已定义转换检查各分支，再检查交付。目标类型提供检查方向，不是允许任意强制转换的开关。

Any 可以出现在返回描述中。是否合法取决于具体实例能否交付所需类型、操作与表示，不能只因名字出现便拒绝，也不能因其允许出现便假设通用装箱。

### 不同适配分别处理

| 输入与目标 | 检查要点 |
|---|---|
| 普通值进入 optional | 按目标进行成功注入 |
| 已有 optional 进入更外层 optional | 保留源值整体，再包外层 |
| 无具体源类型的 null | 构造目标 optional 的外层空值 |
| 普通成功值进入 error qualified type | 成功注入 |
| 已有 error 值进入更宽错误行 | 需要独立的行嵌入及载荷映射 |
| 有成员要求的 Any | 所用操作还需要合法成员来源与签名依据 |

可接受性检查和实际值转换必须同时成立。单独证明某项性质，不会产生传递该值所需的表示方案。

## 常见示例

参数类型的关系可以先由签名说明，具体类型再由调用给出：

~~~flurry
fn identity(value: T) -> T
where T {
    value
}

fn pass_through(value: Any) -> Any {
    value
}

test {
    let integer: u32 = identity(7);
    let flag: bool = identity(true);
    let forwarded_integer: u32 = pass_through(integer);
    let forwarded_flag: bool = pass_through(flag);

    println("integer: {}", forwarded_integer);
    println("flag: {}", forwarded_flag);
}
~~~

输出为 `integer: 7`、`flag: true`。`identity` 明确保持输入与输出的类型关系；`pass_through` 的具体调用也能交付各自的结果。这些调用不要求先把整数和布尔值装入统一运行时盒子。

下面让返回目标影响分支检查：

~~~flurry
fn selected_limit(enabled: bool) -> ?u32 {
    if enabled {
        8
    } else {
        null
    }
}

test {
    selected_limit(true) match {
        limit? => println("limit: {}", limit),
        null => println("disabled"),
    }

    selected_limit(false) match {
        limit? => println("limit: {}", limit),
        null => println("disabled"),
    }
}
~~~

输出为 `limit: 8`、`disabled`。两个分支检查时已知道目标是 `?u32`，因此数值分支注入成功值，`null` 分支构造缺失。不能把这个结果归因于“任何异型分支都会自动组成 optional”。

给参数一个名字仍不授予未知成员。以下普通函数缺少调用依据：

~~~flurry
fn unspecified_length(value: T) -> usize
where T {
    value.len()
}
~~~

仅有 `T` 不能证明存在 `len` 或确定其来源。应提供实际需要的能力要求，或者采用能合法观察具体输入的编译时计算；把参数改名为 `Any` 不会补出成员契约。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 的泛型参数和 C++ 的约束模板提供按参数检查操作的实际对应物；C 的 void 指针则必须靠显式协议解释数据，不能对应 Any 的调用点实例化。Zig 的 anytype 可用于比较推导参数类型，但不能据此推导 Flurry 的分支汇合与返回规则。[Rust 泛型](https://doc.rust-lang.org/reference/items/generics.html)、[基础对照](index.md#与-rust--zig--c--c-的关键差异)

## 运行时实现设计引导

对已实例化的函数，确定每个参数和返回值的主体表示以及环境。若一个实例仍包含异型运行时分支，必须检查后续路径是否有已定义的统一交付方式。

例如两个分支都满足某个“可显示”要求，只解决操作能力的一部分；仍需知道调用入口、接收者适配与值的传递形式。实现不能仅据布尔能力事实生成未经定义的动态对象。

## 编译器实现引导

使用双向检查时，可将无目标推导和带目标检查分成两个入口，但共享相同语义规则。

1. 带目标检查先向子表达式传播目标。
2. 各分支在自己的事实环境中检查，并记录具体转换。
3. 无目标汇合只使用同型或 Any 规则；相等未证明不能伪装成同型。
4. 分支后的事实必须由所有可达前驱支持；类型汇合不能替代事实汇合。
5. 在实例化边界替换实例参数，保留定义端已确定的成员来源。
6. 交付前验证必要布局、环境、调用来源及结果契约。

诊断应区分“这个操作没有事实依据”和“这些已知分支缺少交付表示”。两者可能都发生在 Any 附近，但修复方式不同。
