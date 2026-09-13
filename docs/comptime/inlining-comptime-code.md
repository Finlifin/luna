# Comptime 块与 inline 控制流

## Intro

有些程序在编译时就能决定采用哪种结构，但要等运行时才能处理其中的数据。inline 控制流把这两件事分开：编译器确定分支或展开结构，生成的代码处理运行时值。

comptime 块则直接执行块内的编译时计算。块放在 runtime 函数中，也不需要读取尚不存在的 runtime 数据。

## 语法

`expr`、`pattern`、`label`、`block`、`case_arm` 复用[全局语法](../grammar/syntax)。本节只列相关控制流形式。

```text
comptime_block -> comptime { statement* }
inline_if -> inline if expr block (else block)?
inline_when -> inline when { condition_arm* else_arm? }
inline_if_is -> inline if expr is pattern do block (else block)?
inline_if_match -> inline if expr is do { case_arm* }
inline_for -> inline for label? pattern in expr block
condition_arm -> expr => (block | statement)
else_arm -> else => (block | statement)
```

一般列表项用逗号，语句用分号；以 `}` 结束的语句省略其后分号。普通控制流见[基本控制流](../control-flow/basic.md)。

## 正式定义

### Comptime 块

块内计算属于 comptime，采用 loose 检查强度。块可以出现在 runtime 定义中；该 runtime 定义仍按 ordinary 检查。块内若再定义 runtime 函数，该函数也按 ordinary 检查。

块的结果可以用于后续编译时计算；实际进入 runtime 数据流时按跨阶段规则处理。块内的 `comptime` 不授予读取 runtime 参数实际值的能力。

### Inline 分支

`inline if`、`inline when` 以及相应模式分支在编译时确定所选分支。所选分支中仍属于 runtime 的操作进入生成代码。

ordinary normalization 要检查相应分支中的操作，而不只是等待一次具体实例化选中某支后才开始检查。路径条件能提供的命题参与分支检查。

inline 条件可以是任意 comptime bool 表达式。编译器尝试提取良定义命题；不能提取时，按未知 bool 检查相应分支，不因此增加路径事实。条件未知不等于条件为 false，也不等于可以提前执行两侧副作用。

### Inline 循环

`inline for` 在编译时确定展开结构。遍历元组的结构时，元素的位置和类型可以在编译时用于选择代码，元素值仍可以属于 runtime。

展开循环不免除循环体操作的 requirement。例如，取得一个元素的类型不等于该元素一定实现某个 trait；使用对应操作仍须有约束支持。

普通 `for` 的边界即使是编译时常量，也不等同于 `inline for`。前者没有强制展开的语义承诺。

### 失败

必须在编译时确定的分支条件或展开结构，在所需输入已经绑定后仍无法合法求值时，为编译错误。普通 runtime 条件应使用对应的 runtime 控制流。

未选分支不进入该实例的 runtime 代码；这不取消它在 ordinary normalization 中承担的检查。

## 常见示例

### 只在编译时选择结构

```flurry
fn signed_value(comptime negate: bool, value: i32) -> i32 {
    inline if negate {
        -value
    } else {
        value
    }
}
```

`negate` 决定实例中的分支，`value` 在运行时才传入。负号是否出现在实例中由编译时输入决定。

### 多路选择

```flurry
fn mark(comptime kind: Integer, value: i32) {
    inline when {
        kind == 0 => println("plain: {}", value),
        kind == 1 => println("marked: {}", value),
        else => println("other: {}", value),
    }
}
```

选择打印形式发生在编译时，实际打印发生在运行时。

### 块内求值

```flurry
fn print_area() {
    const AREA = comptime {
        12 * 12
    }
    println("{}", AREA);
}
```

块尾的值用于初始化常量；以 `}` 结束的绑定后面不再写分号。

### 展开固定结构

```flurry
fn print_indices() {
    inline for i in 0 .. 3 {
        println("{}", i);
    }
}
```

展开后包含三个打印操作。这些打印不因为位于 inline 循环体中就提前在编译时执行。

## 与 Rust / Zig / C / C++ 的关键差异

| 对照语言 | 对应机制与差异 |
|---|---|
| Rust | const 块用于常量求值；没有等同于这里的逐项 inline 展开语句，代码重复可用宏表达。[Rust 常量上下文](https://doc.rust-lang.org/reference/const_eval.html#const-context)、[宏重复](https://doc.rust-lang.org/reference/macros-by-example.html#repetitions) |
| Zig | `inline for` 同样展开循环；编译时已知条件会跳过未选分支分析。Flurry ordinary 还承担定义端的分支检查。[Zig 参考](https://ziglang.org/documentation/master/#Compile-Time-Parameters) |
| C11 | `#if` 在预处理阶段选择文本，不根据函数的类型契约传播命题。[C11 草案 §6.10.1](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf) |
| C++ | `if constexpr` 的 discarded statement 有模板实例化规则；Flurry 的 inline 分支按 ordinary/loose 契约检查。[C++ constexpr if](https://eel.is/c++draft/stmt.if) |

## 运行时实现设计引导

1. 对已选择的 inline 分支，只生成该分支的 runtime 操作。
2. 对有 n 个展开项的 inline 循环，生成 n 份相应 runtime 主体；每份使用其对应的编译时结构信息。
3. 对主体中的普通 runtime 条件、循环与调用，保留它们的控制和数据依赖。
4. 对已执行完的 comptime 块，不生成重复求值；其结果跨界时仍执行必要的转换。

若主体大小为 B，直接展开的代码规模可达 O(nB)。实现可进行保持语义的优化，但不能把未执行的 runtime 副作用移到编译期。资源不足以完成必要展开时应报告编译失败。

## 编译器实现引导

### 分支检查与选择

1. 在 normalization 建立分支前的事实环境。
2. 分析条件的阶段与依赖；能提取命题时，分别构造对应分支的适用事实。
3. 在 ordinary 下检查分支操作。条件不能提取命题时，保守检查而不制造额外事实。
4. 在条件输入已具备且控制依赖允许时执行条件，选出该实例的分支。
5. 将所选分支继续实例化；保留其中的 runtime 操作。

不变量：检查某支不等于执行某支。未选择分支的计算不得仅因遍历 IR 就产生求值副作用。

### 循环展开

1. 检查被遍历对象满足该遍历操作的 requirement。
2. 取得展开所需的结构；对结构已知、元素值未知的对象，仅提取阶段允许的结构信息。
3. 为每个展开项绑定对应编译时信息，检查并实例化主体。
4. 合并主体的 runtime 控制流，保持原有次序及控制转移目标。

元组的结构遍历不能降为先读取 runtime 元素来猜测类型。普通循环也不能仅因边界已知就按强制展开处理。
