# Normalization 与 instantiation

## Intro

泛型代码的检查策略通常在两个极端之间选择：完全延迟到实例化（如 C++ 模板），或要求定义端提供所有约束（如 Rust 的 trait bound）。Flurry 采用折中方案：在定义端利用操作契约完成能做的检查，只把确实需要具体输入的计算留给实例化。

一个函数可以在还不知道具体输入时接受检查。编译器依靠参数和操作的契约，判断函数中的使用是否合法；只有确实需要具体输入的计算，才等待实例化。

因此，延迟一个结果的计算，不意味着放弃对后续代码的检查。

## 语法

以下是本章涉及的局部形式。`expr`、`parameter`、`decl_clause`、`block` 等复用[全局语法](../grammar/syntax)。

```text
comptime_block -> comptime { (definition | statement)* }
defer_expr -> defer expr
requires_clause -> requires expr
ensures_clause -> ensures expr
assumes_statement -> assumes expr
comptime_predicate -> comptime predicate id(parameter*) block
optional_decl_clause -> .id : expr = expr
```

`defer_expr` 是表达式，可以用于需要延迟的类型计算和可选子句参数默认值。PTC 的 `requires`、`ensures`、`assumes` 使用具有相应逻辑与可执行含义的命题。

## 正式定义

### 定义端与实例化端

normalization 检查定义，并执行此时要求完成、且输入具备的编译时计算。instantiation 用具体参数继续计算与检查。定义中出现一个调用时，该调用也可能立即提供足够的 comptime 输入，使相关计算就地完成。

这两个过程适用于所有代码，不仅适用于 runtime 函数。comptime 函数也有定义检查和具体调用。

### ordinary 与 loose

| 上下文 | 检查规则 |
|---|---|
| ordinary | 要求规定的操作合法性检查在 normalization 完成 |
| loose | 允许相应约束检查留到 instantiation；当前即可发现的错误仍须检查 |
| comptime 函数或 comptime 块 | 采用 loose |
| loose 内定义的 runtime 函数 | 该函数仍采用 ordinary |
| 其他代码 | 默认采用 ordinary |

进入 loose 不使外部定义降低检查强度。loose 中生成的 runtime 函数引用包围它的 comptime 环境时，这些引用按 defer 处理；函数自身仍须按可用契约检查。

ordinary/loose 描述检查要求，comptime/runtime 描述执行阶段。这是两个不同的分类。

### PTC：操作契约

PTC（propositional type constraints）用命题表达类型检查所需的条件。每项操作可以按前置要求和结果保证理解：

1. 使用操作前，当前上下文必须满足其 requirement。
2. 操作的结果契约可以为后续使用提供命题。
3. 使用这些命题不要求操作的完整结果已经算出。

例如，已知一个尚未具体化的类型是元组类型，可以满足元组遍历操作对容器种类的要求；这并不同时给出元组长度、各元素类型或各元素支持的方法。

PTC 支持 comptime predicate。它与运行时程序验证的 predicate 是不同系统中的构造，即使二者共享部分语法。PTC 命题具有明确的可执行语义。

`requires`、`ensures` 中的命题必须符合 PTC 的良定义要求。ordinary 中要求完成的证明不能以“求解结果未知”冒充成功；无法满足相应要求时为编译错误。`inline` 条件可以是任意 comptime bool 表达式，其命题提取规则见[inline 控制流](./inlining-comptime-code.md)。

### assumes

PTC 中的 `assumes` 使相应命题可用于后续检查。在 instantiation 中，编译器执行该语句，检验具体输入是否满足命题；不满足时为编译错误。

这使定义端可以在相应假设下继续检查，同时在具体实例化时兑现该条件。不能把 `assumes` 当作永不检查的用户逻辑公理；PTC 与程序验证都不开放任意用户公理。

### defer

`defer` 允许相应表达式不在当前 normalization 中取得具体结果。它不自动延迟所有依赖该表达式的合法性检查。

| 使用位置 | 保留的计算 | ordinary 中仍要做的工作 |
|---|---|---|
| 参数或变量的延迟类型 | 求出具体类型 | 按当前可用约束检查对值的操作 |
| 含延迟参数类型的函数调用 | 由 comptime 输入求出并替换参数类型 | 输入具备时就地计算，再检查实际参数 |
| 可选子句参数默认值 | 求出默认值 | 保留参数本身的要求；待输入具备后检查结果 |

延迟类型尚未具体化时，值按 `Any` 及已知 refinement 参与 ordinary 检查。投影、view 和方法等操作必须有足够约束支持；“以后会实例化”不能单独使它们合法。

refinement 的可提取命题可以用于当前检查，相关约束在 instantiation 执行。无法提取的部分不提供证明事实。

`defer` 不读取未来的 runtime 值。一个格式化调用可以有 runtime 实参，同时在 comptime 根据格式字符串计算形参类型；这是两种数据依赖，不能混为一谈。

### 开放定义

loose 环境内的 implementation、derivation 按线性可见性使用。该环境整体实例化时，生成的定义与相关静态作用域中已经完成检查的定义一起接受 coherence 检查；失败则该实例化不成立。

loose 中不允许声明自身带子句参数的开放分散定义。静态作用域提供名称解析的起点，来自满足无子句限制的 `mod`、`struct`、`enum`、`union` 定义。模块的完整可见性规则由模块系统规定。

### 错误归属

| 情况 | 结果 |
|---|---|
| ordinary 中必检的操作要求不成立 | normalization 编译错误 |
| 延迟计算取得具体输入后失败 | instantiation 编译错误，指向触发的调用 |
| PTC assumes 在实例化执行为不满足 | instantiation 编译错误 |
| 生成定义无法通过 coherence | 对应实例化失败 |
| 必需的 comptime 求值耗尽资源 | 编译失败，报告求值与资源上下文 |

## 常见示例

### 计算完成与检查完成可以不同步

```flurry
fn keep(value: T) -> T where T {
    value
}

fn use_keep() {
    let value: i32 = keep(23);
    println("{}", value);
}
```

检查 `keep` 的定义不需要知道 T 最后会是什么类型；参数与结果之间的关系已经写在签名中。具体调用提供输入后，编译器得到相应实例。

### 直接执行 PTC 条件

```flurry
comptime predicate is_enabled(flag: bool) {
    flag
}

fn require_enabled(comptime enabled: bool) {
    assumes is_enabled(enabled);
}

fn use_enabled() {
    require_enabled(true);
}
```

`is_enabled` 是本例定义的 PTC 谓词，直接表达布尔输入成立。本例调用的条件在实例化中满足；把实参改成 `false` 会在实例化时执行该谓词并检查失败。

### 延迟格式对应的参数类型

格式化函数的参数类型可以由 `defer std.fmt.Parse(format)` 表示。格式字符串确定后，就地执行对应类型计算，再检查传入参数。

```flurry
fn show_name(name: str) {
    println("Hello, {}!", name);
}
```

这里格式字符串是 comptime 输入，`name` 是 runtime 参数。解析格式不需要读取 `name` 的实际字符内容。格式解析函数返回的具体类型及诊断措辞属于格式库契约。

## 与 Rust / Zig / C / C++ 的关键差异

| 对照语言 | 对应机制与差异 |
|---|---|
| Rust | trait bounds 在函数体内支持受约束操作；Flurry 的 PTC 还以可执行命题衔接延迟结果与实例化检查。[Rust bounds](https://doc.rust-lang.org/reference/trait-bounds.html) |
| Zig | comptime 参数采用编译时 duck typing；Flurry ordinary 要求输入未知时依操作契约完成指定检查。[Zig comptime 参数](https://ziglang.org/documentation/master/#Compile-Time-Parameters) |
| C11 | 无对等的开放定义检查机制；宏展开后检查具体代码，常量条件可用静态断言。[C11 草案 §6.7.10、§6.10](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf) |
| C++ | constraints 与模板中的 static_assert 分别用于适用性和断言；Flurry PTC assumes 还为定义端的后续检查提供命题。[C++ constraints](https://eel.is/c++draft/temp.constr)、[static_assert](https://eel.is/c++draft/dcl.pre) |

## 运行时实现设计引导

两阶段检查必须在相应 runtime 实例交付之前完成：

1. 确认实例所需的类型、布局和调用目标已经确定。
2. 确认依赖具体输入的编译时检查已经成功。
3. 擦除 comptime 输入与已经执行的 PTC 检查，保留 runtime 参数及操作。
4. 按实例的 runtime 控制流执行，不重复进行类型构造或 PTC 求解。

不变量：一个失败的实例化不能以“运行时再检查”的形式通过；这不禁止程序另有显式 runtime 检查。普通 `Splice` 方法实例中的 runtime 工作仍须保留。

## 编译器实现引导

### Query 图的输入与状态

normalization 的结果包含 query 计算图：节点表示检查或计算，边表示依赖。comptime 求值可以触发 query，query 也可以触发 comptime 求值。

持久产物保留稳定绑定、契约和语言允许的残余计算；本次推断的可变状态、等待登记与 continuation 不直接进入缓存。实例化从冻结结果建立自己的局部状态。具体表示与交付见[两阶段计算与冻结](../compilation/staging-and-freezing.md)。

实现应区分两种依赖：需要前置节点的具体结果，或只需要其可用结果契约。对一个节点至少保存其输入依赖、当前结果或契约、检查要求以及诊断来源。

以下是实现用状态，不是语言关键字：等待输入、可处理、已完成、失败。合法性检查与具体求值分别记录状态；前者完成不代表后者完成。

### Normalization 算法

1. 在 HIR 上按构造标记 ordinary/loose；类型推导进展不得静默改变该分类。
2. 建立参数、绑定、路径事实与操作契约的环境。
3. 遇到 ordinary 操作，求解其 requirement。成功后使适用的结果命题可供后续节点使用；必检义务失败则报错。
4. 区分普通 comptime 调用与范式应用。对可执行的计算，按实际控制依赖及输入就绪情况执行；对范式应用保留头与实参，不展开头的函数体。不能因正在检查分支而提前执行分支副作用。
5. 对合法延迟的位置保存求值节点。后续只需契约的节点继续处理；需要具体结果的节点等待相应输入。
6. 遇到 PTC assumes，将命题纳入适用的检查上下文，并保留该语句的实例化执行。
7. 处理嵌套定义时重新依据其构造确定检查强度，不把父 loose 强度传播给 runtime 函数。
8. 输出已经完成的检查、可用契约和剩余计算依赖；实现可用多趟处理完成这些工作。

### Instantiation 算法

1. 绑定具体类型、子句参数及 comptime 输入。
2. 激活其输入已经具备的计算；为新取得的结果更新依赖节点。
3. 执行到 PTC assumes 等检查时求出其命题值；失败则终止依赖该检查的实例化。
4. 完成延迟类型替换、分支选择及被调用定义的实例化。
5. 对 loose 生成的开放定义执行相应 coherence 检查。
6. 只有实例所需计算与检查全部完成后，才输出 runtime 实例或 comptime 调用结果。

具体实例的 IIR 已完成类型中的 comptime 部分，但仍可以保留 runtime 依赖和高级控制结构，无需先成为 SSA。合法范式头的保留不等于尚欠普通求值，见[IIR 与代码交付](../compilation/iir-and-codegen.md)。

在控制依赖尚未确定前，不因参数替换而执行所有分支。实际执行过程中发现的新依赖应由同一机制调度；不能以一个未计算节点的占位结果代替真实结果。

### 两种执行载体

| 载体 | 职责 |
|---|---|
| comptime interpretation | 在 runtime 定义等混合场景中就地解释相关 IR |
| comptime invocation | 将独立 comptime 函数编译为 engine 字节码，在调用时执行 |

两者遵守相同的求值、类型与副作用语义，可以使用不同资源预算。预算不同时，某次求值可能只在其中一个预算下完成；不能据此把两者描述为不同语言子集。

### 不变量、复杂度与失败

- 契约依赖必须保留其绑定和适用上下文，不能把局部命题传播到不适用的路径。
- 缓存命中必须恢复正确的结果及相关诊断；包含不纯计算的 query 不能仅因采用 query 接口就视为纯。
- 对固定有限 DAG，工作队列调度自身可做到 O(V + E)；这不包含谓词求解、解释执行及实例化扩展的代价。
- comptime 可递归且不保证终止；资源耗尽必须报告。依赖循环也不能用占位成功来打断，更不能把“正在求值”当作已证明事实。
- 失败诊断同时保留触发调用与原始定义来源，以区分当前输入导致的失败和定义中已可确定的错误。
