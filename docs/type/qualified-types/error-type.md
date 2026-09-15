# 错误限定类型

## Intro

错误限定类型说明一次计算可能交付成功值，也可能交付某个错误分支。错误行列出分支身份，成功主体说明正常结果是什么。

处理错误可以改变允许返回的分支，但必须保留未处理错误的身份与载荷。

## 语法

~~~text
error_type -> !expr expr
error_handle -> expr! { case_arm* }
propagate_error -> expr!
~~~

错误构造、case_arm 和传播沿用全局语法。内部 Error(E,A) 表示错误行 E 与成功主体 A，不是公开类型工厂。

## 正式定义

错误行使用唯一标签，标签是完整错误类型实例。同类限定合并错误分支，重复的同一身份不形成新的处理层；空错误限定消去。

成功值进入错误限定是成功注入。已有错误结果进入更宽行则是分支嵌入：需证明源错误可在目标表示，并保持标签和载荷。二者不因表面都可被接受而成为同一操作。

消去某个输入错误后，输入余行不含该标签；输出行却可以再次包含它，因为处理分支可以返回同一种错误。输入余行与输出行不能强制相同。

错误本身不附带恢复失败位置之后计算的 continuation。处理分支发出的 effect 在自己的允许效应环境中检查，不变成 error 标签。

## 常见示例

下面把“超过允许数量”作为带载荷的错误。成功分支直接交付数值；失败分支保留被拒绝的输入：

~~~flurry
enum LimitError {
    too_large(u32),
}

fn checked_count(count: u32) -> ![LimitError] u32 {
    if count <= 8 {
        count
    } else {
        error LimitError.too_large(count)
    }
}

fn doubled_count(count: u32) -> ![LimitError] u32 {
    let accepted = checked_count(count)!;
    accepted * 2
}

fn show_count(count: u32) {
    doubled_count(count) match {
        value! => println("doubled: {}", value),
        error .too_large(rejected) => println("rejected: {}", rejected),
    }
}

test {
    show_count(3);
    show_count(9);
}
~~~

输出为 `doubled: 6`、`rejected: 9`。后缀 `!` 在成功时取出 `u32`，在失败时从当前函数继续交付同一个错误；失败后不执行乘法。

错误处理分支也可以返回成功值，或再次产生原错误：

~~~flurry
fn count_or_zero(count: u32) -> u32 {
    checked_count(count)! {
        .too_large(_) => 0,
    }
}

fn retry_with_smaller_count(count: u32) -> ![LimitError] u32 {
    checked_count(count)! {
        .too_large(rejected) => checked_count(rejected / 2),
    }
}

test {
    println("fallback: {}", count_or_zero(9));

    retry_with_smaller_count(20) match {
        value! => println("accepted: {}", value),
        error .too_large(rejected) => println("still rejected: {}", rejected),
    }
}
~~~

输出为 `fallback: 0`、`still rejected: 10`。第二个 handler 已处理输入错误，但分支中新调用仍可能失败，因此输出类型继续包含 `LimitError`。错误分支处理的是载荷模式；普通 `match` 则显式区分成功模式和 `error` 模式。

## 与 Rust / Zig / C / C++ 的关键差异

Rust Result 和 Zig error union 是直接的结果表达手段；C 错误码常需要另行约定载荷，C++ 异常还定义栈展开控制流。Flurry 的错误行不自动引入异常展开或可恢复控制流。[Rust Result](https://doc.rust-lang.org/std/result/)、[C++ 异常](https://eel.is/c++draft/except)

## 运行时实现设计引导

表示计划区分成功与具体错误。嵌入可以重编码分支索引，但不能丢失错误身份或改变载荷解释。

处理过程只有三种来源：成功、被处理错误、未处理错误。分别检查分支转换和输出表示，不能让未处理分支落入无定义路径。

## 编译器实现引导

1. 规范化唯一错误行，证明严格扩展的 lacks。
2. 检查成功注入或错误嵌入所需证据。
3. handler 拆出目标标签，保留输入余行。
4. 独立检查输出行与各分支结果。
5. 对未覆盖载荷保留向外返回路径。
6. 交付载荷映射及分支控制流。

错误包含关系本身不等于整个类型的通用子类型规则。开放尾部仍需证据，不能只凭当前列出的标签通过。
