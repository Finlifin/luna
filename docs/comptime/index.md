# 编译时计算

## Intro

传统编译时计算（如 C 的预处理器宏或 C++ 的 constexpr）在表达能力和类型安全之间存在取舍。Flurry 将编译时计算设计为语言的一等公民：编译时函数使用与运行时相同的语法和类型系统，同时通过执行阶段标记区分两者的边界。

Flurry 程序中的一部分计算由编译器执行，其余计算留给生成的程序。编译器先处理已知的信息，让程序保留需要运行时数据的工作。

输入尚未确定时，编译器仍能利用操作的契约检查代码。知道一个结果满足什么条件，往往已经足以检查后面的操作，而不必先把这个结果完整算出来。

## 语法

本节列出阅读入口；`expr`、`parameter`、`result`、`clauses`、`statement` 和 `block` 复用[全局语法](../grammar/syntax)。

```text
comptime_function -> pure? comptime fn id(parameter*) (-> result)? clauses? block
comptime_parameter -> comptime parameter
comptime_block -> comptime { (definition | statement)* }
inline_if -> inline if expr block (else block)?
defer_expr -> defer expr
```

`comptime` 修饰函数、参数和块时，分别约束函数的执行阶段、参数的可用阶段和块内计算。`defer` 保留尚待求值的表达式；其位置规则见[两阶段检查](./normalization-and-instantiation.md)。

## 正式定义

### 两个执行阶段、两个编译过程

| 名称 | 对程序的含义 |
|---|---|
| comptime | 计算在编译时发生，结果可以参与类型与代码的确定 |
| runtime | 计算由生成的程序执行 |
| normalization | 检查定义，利用已知输入完成相应编译时计算 |
| instantiation | 代入具体输入，继续编译时计算与检查，得到对应实例 |

normalization 与 instantiation 都是编译过程。runtime 函数的定义要经过它们；comptime 函数的定义与调用也要经过它们。不能把 normalization 等同于 comptime、把 instantiation 等同于 runtime。

一个表达式暂时算不出，可能只是缺少 comptime 输入。这不使它变成 runtime 表达式。反过来，即使实参写成常量，普通 runtime 函数也不会因此自动成为 comptime 函数。

### 操作与检查

ordinary 上下文要求定义端完成相应的合法性检查。检查依据包括参数约束、当前路径上的命题和先前操作提供的结果契约。loose 上下文允许指定检查留到实例化；它不免除最终检查。

`comptime` 函数与块采用 loose；在其中定义的 runtime 函数仍采用 ordinary。这些规则见[normalization 与 instantiation](./normalization-and-instantiation.md)。

### 跨阶段传递

运行时上下文中的子表达式没有 runtime 义务、且所需输入已知时，优先在 comptime 求值。结果实际进入 runtime 数据流时，遵守该类型的跨阶段规则。

comptime 参数在运行时调用约定中擦除。普通值的 `Splice` 转换则可以保留运行时调用；两者见[跨阶段值传递](./quoting-and-splicing.md)。

### 阅读路线

| 顺序 | 章节 | 主要问题 |
|---|---|---|
| 1 | [Normalization 与 instantiation](./normalization-and-instantiation.md) | 输入未知时能检查什么？defer 和 PTC 如何配合？ |
| 2 | [Comptime 函数、纯性与范式](./purity-and-normal-form.md) | 如何编写编译时计算？pure 承诺什么？ |
| 3 | [Comptime 块与 inline 控制流](./inlining-comptime-code.md) | 怎样在编译时决定分支和展开结构？ |
| 4 | [跨阶段值传递](./quoting-and-splicing.md) | comptime 值如何用于 runtime？ |
| 5 | [Trait 的 comptime/runtime 两相](./the-comptime-phases-of-traits.md) | 调用哪一相？开放定义如何检查？ |
| 6 | [案例：静态形状的张量加法](./static-shaped-tensors.md) | 形状、契约和元素计算如何协作？ |

[属性](./attribute.md)使用编译时表达式描述元数据。[编译流水线](../compilation/phases.md)给出编译过程的输入与输出。

## 常见示例

```flurry
pure comptime fn square(value: Integer) -> Integer {
    value * value
}

fn print_values() {
    println("{}", 1 + 23);
    println("{}", square(12));
}
```

第一行打印中的加法在编译时得到 24；第二行的 `square` 在编译时得到 144。打印操作仍在运行时发生。

```flurry
fn select_value(comptime first: bool, left: i32, right: i32) -> i32 {
    inline if first {
        left
    } else {
        right
    }
}
```

`first` 决定生成哪个分支；`left` 与 `right` 是运行时参数。编译时选择分支不需要读取这两个值。

## 与 Rust / Zig / C / C++ 的关键差异

| 对照语言 | 对应机制与差异 |
|---|---|
| Rust | `const fn` 可在常量上下文求值，也可像普通函数一样被调用；Flurry 的 comptime 函数属于独立执行阶段。[Rust Reference](https://doc.rust-lang.org/reference/const_eval.html#const-functions) |
| Zig | comptime 参数同样要求调用处取得编译时值；Flurry 还要求 ordinary 定义依操作契约完成相应检查。[Zig 语言参考](https://ziglang.org/documentation/master/#Compile-Time-Parameters) |
| C11 | 无对等的编译时函数机制；实际手段是常量表达式、宏和外部代码生成。[C11 草案 §6.6、§6.10](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf) |
| C++ | `constexpr` 函数可参与常量或运行时求值；Flurry 的 comptime 函数只属于 comptime 阶段。[C++ 工作草案](https://eel.is/c++draft/dcl.constexpr) |

## 运行时实现设计引导

生成的程序需要满足以下边界：

1. 接收实例的 runtime 参数，不接收已擦除的 comptime 参数。
2. 执行实例中保留的控制流、调用与数据操作。
3. 需要普通值跨界转换时，执行对应 `Splice` 方法的 runtime 实例，或执行保持其行为的优化结果。

不变量：运行时不承担“重新猜测编译时类型”的工作；必需的编译时检查失败时，不得交付依赖该失败结果的实例。运行时代价由保留的操作决定，不能仅凭源码含 comptime 就断言为零。

## 编译器实现引导

编译器应分别记录三个问题：表达式属于哪个执行阶段、它的合法性检查是否完成、它的具体值是否已取得。

1. 依据定义与上下文确定阶段及 ordinary/loose 检查强度。
2. 用当前契约检查操作；能够求值的 comptime 部分参与计算。
3. 保存依赖未知输入的计算，以及后续检查可用的结果契约。
4. 实例化时提供输入，继续计算和检查，输出完整的 runtime 实例或 comptime 结果。

检查通过不等于操作已经执行；一个具体值未知也不等于所有下游检查都必须等待。详细的 query 算法与失败处理见[两阶段检查的编译器引导](./normalization-and-instantiation.md#编译器实现引导)。

代码值的完整规则见 [CMTT](../cmtt/index.md)：代码引用与普通值的 Splice 分别检查，quoted expr、pattern 和 arm 保留各自上下文及接入契约。
