# 错误结果、传播与 catch

## Intro

错误处理面对的是已经交付的结果：它要么成功，要么携带某个错误值。后缀 ! 可以向外传播错误，也可以在当前位置选择替代结果。

需要统一记录或重试时，catch arm 可以在具体错误分支前后组织共同工作。

## 语法

~~~text
error_type -> !expr expr
error_new -> error expr
error_propagation -> expr!
error_elimination -> expr! { error_arm* }
error_arm -> case_arm | catch_arm
catch_arm -> catch pattern => (block | statement)
~~~

catch arm 只用于错误消去。普通 match 使用 error pattern 区分错误值，成功模式使用 pattern!。

## 正式定义

### 独立的结果 ABI

错误限定值编码成功或错误标签与载荷；错误行记录允许的错误类型。构造 error 值本身不是请求 effect handler，也不保存失败位置之后的 continuation。

后缀 ! 的成功路径取出主体值，错误路径从当前合法返回边界交付错误，并执行相应退出清理。结果表示、行嵌入和剩余错误见[错误限定类型](../type/qualified-types/error-type.md)。

### 普通消去

普通 arm 匹配错误载荷。成功输入直接交付成功值；错误输入选择相应 arm，分支结果成为该错误路径的结果。没有覆盖的错误需要合法的剩余传播路径。

handler 分支可以产生新的错误，输出错误行独立计算。不能因为输入某个错误已被匹配，就从输出中无条件删除同一种错误。

### 前置与后置 catch

最多有一个前置 catch 和一个后置 catch，分别放在普通 arm 之前和之后。只有一个 catch 时按后置处理。二者只在输入是错误时执行，绑定的是原始完整错误限定值，而不是已经剥去标签的载荷。

错误路径依次执行前置 catch、匹配的普通 arm、后置 catch。任何一步显式退出时，不再执行同一路径的后续步骤。

存在后置 catch 时，普通 arm 的普通完成不提前结束消去：其值被丢弃，然后执行后置 catch；后置路径交付错误路径的最终结果或执行显式控制动作。没有普通 arm 匹配时也进入后置 catch。

前置和普通 arm 对错误的观察、后置对完整值的交付必须满足资源规则。不隐式复制一次性载荷；如果某步已经移出后续仍需要的部分，不能再把完整错误当作有效值。

错误类型名以 Err 结尾，错误枚举的变体使用 PascalCase。

## 常见示例

### 两种错误由同一返回类型交付

~~~flurry
enum InputErr { Missing, }
enum ParseErr { Invalid, }

fn load_number(present: bool, valid: bool) -> ![InputErr, ParseErr] u32 {
    if not present { return error InputErr.Missing; }
    if not valid { return error ParseErr.Invalid; }
    42u32
}

test {
    let value = load_number(true, false)! {
        InputErr.Missing => 0u32,
        ParseErr.Invalid => 1u32,
    }
    println("{}", value);
}
~~~

输出 1。错误处理没有重新执行 load_number，也不会从 return 之后继续执行它。

### 共同观察与共同结果

~~~flurry
test {
    let value = load_number(false, true)! {
        catch _ => println("before"),
        InputErr.Missing => println("missing"),
        ParseErr.Invalid => println("invalid"),
        catch _ => 9u32,
    }
    println("{}", value);
}
~~~

依次输出 before、missing、9。若输入成功，两个 catch 均不执行，直接得到 42。

### 后置 catch 控制重试

~~~flurry
enum RetryErr { Again, Exhausted, }

fn attempt(number: u32) -> !RetryErr u32 {
    if number < 2u32 { error RetryErr.Again } else { 42u32 }
}

fn with_retry() -> !RetryErr u32 {
    var number = 0u32;
    while :retry_loop true {
        let value = attempt(number)! {
            .Again => println("retry requested"),
            catch failure => {
                if number < 2u32 {
                    number += 1u32;
                    continue retry_loop;
                }
                return failure;
            },
        }
        return value;
    }
    error RetryErr.Exhausted
}

test {
    with_retry() match {
        value! => println("{}", value),
        error _ => println("failed"),
    }
}
~~~

输出两次 retry requested，随后输出 42。failure 是完整的 !RetryErr u32，最终传播可以直接交付它。重试来自显式循环，而不是恢复先前失败调用的 continuation。

## 与 Rust / Zig / C / C++ 的关键差异

Rust Result 与 Zig error union 都提供成功／错误结果；C 可用返回码及单独载荷表达。C++ 异常还涉及异常展开，与本篇结果 ABI 不同。前后 catch 可以用这些语言的普通分支和共同代码显式组织，不意味着错误结果自带恢复能力。[Rust Result](https://doc.rust-lang.org/std/result/)、[Zig Errors](https://ziglang.org/documentation/master/#Errors)、[C++ 异常](https://eel.is/c++draft/except)

## 运行时实现设计引导

1. 只求值一次被消去表达式，保存完整结果及责任。
2. 成功时交付成功载荷，跳过全部错误 arm。
3. 错误时先执行前置 catch，再选择普通错误分支。
4. 存在后置 catch 时，将普通分支完成接到后置入口；无匹配也接到该入口。
5. 没有后置 catch 时，匹配分支交付结果，未覆盖错误按剩余路径传播。
6. 每个退出边按实际剩余责任清理，不复制整个错误供各步骤任意消费。

## 编译器实现引导

前后 catch 先展开为明确的顺序控制图，再按普通错误行、结果汇合和资源规则检查。保存的是同一个原始结果，不能把后置 catch 的输入换成普通 arm 的返回值。

错误处理的清理图可以被其他退出路径复用，但其错误标签、返回表示和匹配分支不因此成为 continuation 中止协议。对成功路径、未匹配错误、分支提前退出及后置结果分别检查。
