# 代数效应与限定 continuation

## Intro

函数可以提出一个操作请求，由调用环境决定怎样处理。主体说明自己需要什么，handler 提供这一次计算使用的行为；可恢复处理还允许 handler 暂存剩余计算，并在以后继续执行。

先理解请求如何返回，再理解剩余计算如何被保存。错误结果也在本主题中介绍，但它有独立的数据表示和传播 ABI。

## 语法

~~~text
algebraic_effect -> async? inline? effect id(parameter*) (-> result)? clauses?
effect_enum -> effect enum id clauses? { effect_variant* }
effect_variant -> attribute* async? inline? id(parameter*) (-> result)?
effect_qualified_type -> #expr expr
effect_handle -> expr# { case_arm* }
perform_effect -> expr#
handler_bind -> pattern handles expr
handler_application -> expr.use(expr)
resume_statement -> resume expr? (while expr)?
~~~

独立签名以分号结束；操作组各项以逗号分隔。参数、子句和分支模式复用全局语法。

## 正式定义

### 从结果到控制

| 形式 | 调用点取得什么 | 谁决定何时继续 |
|---|---|---|
| 错误限定结果 | 成功值或错误载荷 | 普通结果分支与传播 |
| 同步直接处理 | 操作签名规定的结果 Y | handler 返回后，调用点继续 |
| 显式捕获 | handler 同时取得 continuation | 持有恢复权的代码 |
| async 处理 | 子工作的 Future | 调用者继续；等待由协议接入 |
| direct passing | 安装时已经取得的值 | 普通传值，不重复执行 handler |

效应行记录允许请求的完整操作实例，并保留同名处理层的 occurrence 顺序。多次调用同一个操作不增加处理层数。处理能力由 trait 协议配合 elaboration 提供；处理分支自己的错误、效应和资源行为仍需要检查。

显式 continuation 使用一次恢复权，不提供复制或多次恢复。每次再次挂起取得一个新的 continuation。能否逃逸由实际可达依赖决定；恢复重新安装内部 handler，并从恢复点取得外层入口。挂起计算在线程内恢复。

独立使用栈池时，使用者通过 .use(pool) 显式选择实例。runtime 可以封装并供应栈池，此时只需 .use(runtime)。栈存储与调度各自的职责可以由同一个实例统一接入；每段栈在完成清理后归还其来源池。

### 阅读路线

| 文档 | 主要问题 |
|---|---|
| [操作与处理](algebraic-effect.md) | 声明、操作组、直接返回、嵌套与转发 |
| [处理能力](handling-capability.md) | trait、绑定与 .use 怎样接入处理环境 |
| [错误结果与 catch](error-handling.md) | 错误传播、前后 catch 与重试 |
| [直接传值](direct-passing.md) | 安装时求值、局部覆盖、隐式值接口 |
| [限定 continuation](multi-prompt-delimited-continuation.md) | 捕获、尾恢复、逃逸、单次使用与清理 |
| [异步效应](async-algebraic-effect.md) | 子工作、Future、显式 runtime 与结构化退出 |
| [ABI 与运行时协议](effect-abi.md) | evidence、动态路由、显式栈池实例、中止通道与 FFI |

类型层的行运算见[效应限定](../type/qualified-types/effect-type.md)与 [handler 契约](../type/handlers.md)。

## 常见示例

~~~flurry
effect next_number() -> u32;

fn total() -> #[next_number] u32 {
    let first = next_number()#;
    let second = next_number()#;
    first + second
}

test {
    let result = total()# {
        next_number() => 6u32,
    }
    println("{}", result);
}
~~~

输出 12。handler 处理两次请求，每次返回 6；主体继续完成加法。这个程序不需要在源码中取得 continuation。

改用错误限定时，失败会阻止后续计算：

~~~flurry
enum InputErr {
    Missing,
}

fn require_number(present: bool) -> !InputErr u32 {
    if present { 6u32 } else { error InputErr.Missing }
}

fn doubled(present: bool) -> !InputErr u32 {
    let number = require_number(present)!;
    number * 2u32
}

test {
    let result = doubled(false)! {
        .Missing => 0u32,
    }
    println("{}", result);
}
~~~

输出 0。错误处理没有给失败位置提供一个数值并恢复乘法；它处理的是已经返回的错误结果。

## 与 Rust / Zig / C / C++ 的关键差异

错误结果可与 Rust Result、Zig error union 或 C 的带标签返回结构对照。可恢复效应没有与这四种语言的通用 handler 机制直接对等的构造；回调、协程和显式状态机是实际替代手段。C++ 协程分别规定挂起与恢复，不等于本章按效应 occurrence 安装 handler 的契约。[Rust Result](https://doc.rust-lang.org/std/result/)、[C++ 协程](https://eel.is/c++draft/dcl.fct.def.coroutine)

## 运行时实现设计引导

运行时先按所选协议确定入口，再采用对应控制方式：直接调用、读取安装值、捕获并切换，或启动子工作。错误结果维持独立的数据交付路径。

恢复必须同时接入执行位置、handler 环境、返回位置和资源责任。池化存储只有在这些依赖都结束后才能复用。调度器使用控制后端，但同步控制后端无需包含调度器。

## 编译器实现引导

1. 建立完整操作身份、参数角色、结果类型及属性。
2. 检查计算的允许行和每层处理 occurrence。
3. 通过协议取得入口、环境及 ABI 描述。
4. 按直接、捕获、async 或 direct passing 的契约展开控制流。
5. 独立检查错误结果与非局部控制目标。
6. 计算可达依赖、清理路径及恢复点的外层供应。
7. 交付协议兼容的调用接口，并由目标 pipeline 完成其合规检查。

不同 ABI 的入口不能仅因表面结果类型相同而互换。不能把属性丢失后的普通函数指针当成仍然携带处理或恢复契约的入口。
