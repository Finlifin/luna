# Handler 的类型与恢复契约

## Intro

handler 规定计算产生错误或请求效应时怎样继续。这一机制源自代数效应（algebraic effects）理论，将副作用的描述与处理解耦，使程序获得结构化的控制流抽象能力。要读懂 handler 的类型，需要区分操作返回值、主体正常结果，以及整个处理结果。

错误分支处理、普通效应的直接返回和可恢复处理各有不同的控制流，因此不能都解释成“从行中删掉一个名字”。

## 语法

~~~text
error_handle -> expr! { error_arm* }
effect_handle -> expr# { case_arm* }
case_arm -> pattern => (block | statement)
~~~

下面的 k、Comp、Error、Return、Request 和 Handle 为内部类型/执行记号，不定义源码 continuation API 或新的正常完成分支语法。

## 正式定义

处理能力由 [trait 协议配合 elaboration](../traits/protocols.md#处理能力) 提供。下面的类型关系约束接入后的处理行为，包括结果交付、行消去与恢复路径。

### 三个结果

| 记号 | 意义 |
|---|---|
| Y | 一次 effect 操作返回给调用点的结果 |
| A | 被处理主体正常完成的结果 |
| B | 整个 handler 最终交付的结果 |

默认保留主体结果，即 B=A。若要 A→B，必须明确提供正常完成路径的转换，不能只修改某个 effect 分支的返回类型。操作签名中的 Y 保持不变。

### 错误处理

将输入拆为 Error(⟨e:P | ρ⟩,A)，要求 Lacks(ρ,e)。P 是错误载荷。完整类型关系如下：

~~~text
success : A -> Error(σ,B)
on_e    : P -> Error(σ,B)
forward : EmbedError(ρ,σ)
result  : Error(σ,B)
~~~

成功输入走 success；错误 e 走 on_e；其余错误保持身份与载荷，按 forward 交付。σ 独立于 ρ，可以重新包含 e，也可以包含分支产生的新错误。

### 直接返回的效应

操作签名 X→Y，处理分支 X→Comp(ε,Y)。分支产生 Y 后返回操作调用点，主体继续。分支在当前处理边界外执行，因此自己请求的效应在 ε 中取得入口。

这种形式无需在源码暴露 continuation。普通调用/返回可以实现它；类型里出现效应不要求每次分配恢复对象。

### 可恢复的 deep handler

~~~text
body        : Comp(⟨a | ε⟩,A)
on_return   : A -> Comp(ε,B)
operation a : X -> Y
resume k    : Y -> Comp(ε,B)
on_a        : (X,k) -> Comp(ε,B)
result      : Comp(ε,B)
~~~

分支位于当前边界外；恢复主体时重新安装当前 handler。ε 可以含 a，不要求 Lacks(ε,a)。所选的一层被处理后，外层同名层仍然存在。

恢复权单次使用，不可复制；每次重新挂起产生新的 continuation。能否逃逸由实际可达依赖决定，可以越过原安装位置，但所需栈与环境必须保持有效。恢复重新安装内部处理关系，并从恢复点供应外层效应入口；已保存的普通局部值和固定对象身份保持不变。

放弃通过隐藏中止通道执行尚存责任的清理，该通道独立于用户错误结果 ABI。resume 保证尾恢复；显式非尾恢复保留等待 B 的后继。builtin.Continuation 不带公开类型参数，原始显式操作履行其 unsafe 契约，不能据统一类型名推导任意载荷都可恢复。完整行为见[限定 continuation](../algebraic-effect-and-delimited-continuation/multi-prompt-delimited-continuation.md)。

### 部分处理与转发

缺少分支或 guard 未覆盖的请求，需要向外转发证据。输出行保留可能逃逸的标签；缺少外部入口时静态拒绝。

转发后，外部提供 Y，主体仍应在当前 handler 下继续。因此转发携带的恢复路径也要重新包裹当前 handler，不能只传裸 continuation。

handler 构造、安装或分支自身的行为分别按所在表达式环境检查，不因 handler 能处理主体某标签就提前擦除。

## 常见示例

先让一次操作返回 `u32`，而主体返回 `bool`，使两种结果不会在例子中碰巧相同：

~~~flurry
effect read_number() -> u32;

fn both_small() -> #[read_number] bool {
    let first = read_number()#;
    let second = read_number()#;
    first < 10 and second < 10
}

fn checked_with_outer_input() -> #[read_number] bool {
    both_small()# {
        read_number() => {
            println("inner request");
            let outside = read_number()#;
            outside / 2
        },
    }
}

test {
    let accepted = checked_with_outer_input()# {
        read_number() => {
            println("outer request");
            12
        },
    }
    println("accepted: {}", accepted);
}
~~~

输出顺序为：

~~~text
inner request
outer request
inner request
outer request
accepted: true
~~~

每个操作分支返回的 `6` 是 Y，类型为 `u32`。主体继续执行，最后产生的 `true` 是 A，类型为 `bool`。此处 handler 保留正常结果，因此 B 也是 `bool`。

两次 `inner request` 说明主体在第一次请求完成后仍使用内层处理器。内层分支自己的请求则在外层运行。若误把分支放回自身边界，第一条请求就会递归进入自己，无法得到这条轨迹。

对比错误 handler：失败后直接进入处理分支，函数内失败点后面的语句不继续执行：

~~~flurry
enum InputErr {
    Missing,
}

fn require_input(present: bool) -> ![InputErr] u32 {
    if present {
        5
    } else {
        error InputErr.Missing
    }
}

fn process_input(present: bool) -> ![InputErr] u32 {
    let value = require_input(present)!;
    println("after input");
    value * 2
}

test {
    let value = process_input(false)! {
        .Missing => 0,
    }
    println("result: {}", value);
}
~~~

这里只输出 `result: 0`。handler 的 `0` 是整个失败计算的替代结果，不是交回 `require_input` 调用点继续执行的 Y。

上述效应例子使用直接返回分支，无需显式恢复参数。涉及单次恢复能力的内部规则见后文运行时引导；continuation 的转交与逃逸另按实际依赖检查，不从这些直接返回分支推导。

## 与 Rust / Zig / C / C++ 的关键差异

C/C++ 异常、Rust/Zig 的错误结果与回调协议有各自的控制流用途，但都不能直接替代这里的 deep handler 类型规则。C++ 异常传播不以给失败操作提供一个 Y 来恢复其后续表达式。通用 handler 演算提供理论对应，具体恢复限制仍以 Flurry 契约为准。[C++ 异常](https://eel.is/c++draft/except)、[Handlers tutorial](https://www.eff-lang.org/handlers-tutorial.pdf)

## 运行时实现设计引导

内部执行规则如下，H.return 与 H.a 表示已检查分支：

~~~text
Handle(H, Return(v)):
    execute H.return(v) outside H

Handle(H, Request(a,x,k)) targeting H:
    execute H.a(x, lambda y: Handle(H,k(y))) outside H

Handle(H, Request(b,x,k)) forwarded outward:
    forward Request(b,x,lambda y: Handle(H,k(y)))
~~~

最后一条也适用于同名但路由指向更外层的请求。不能仅用 b 是否等于 a 判断是否由 H 接收。

直接处理可降低为环境参数和普通函数调用。可恢复处理需建立限定边界，并确保恢复 token 最多使用一次、满足实际可达依赖；释放或放弃路径必须按资源契约执行。

错误 handler 可降低为分支判别和载荷映射。未处理错误的转发路径也必须产生合法输出表示，不能留下遗漏分支。

## 编译器实现引导

1. 检查主体，取得输入行及结果 A。
2. 由 split 证据选定错误分支或 effect occurrence。
3. 为分支建立外部允许行；恢复参数的输入是 Y，结果是 B。
4. 验证正常完成路径；缺少显式结果转换时保持 A。
5. 检查所有分支行为、未覆盖输入和向外路由。
6. 为恢复及转发生成重新安装边界的包装。
7. 验证单次使用、实际逃逸依赖、恢复点外层供应与清理义务。
8. 交付输出行、结果表示和入口映射。

测试应分别覆盖正常返回、直接处理、同名向外请求、不同标签转发、恢复后的第二次请求、guard 未覆盖、结果 A→B 转换以及非法重复恢复。只验证最终行少了一个标签不足以证明实现正确。
