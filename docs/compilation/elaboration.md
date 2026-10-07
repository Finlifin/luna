# Elaboration 与上下文检查

## Intro

同一段表达式可能需要周围的类型才能检查。一个 lambda 没写参数类型，并不意味着编译器要立即猜测；接收它的函数和其他实参可以一起提供依据。

Elaboration 把这些依据落实为确定的绑定、类型、调用和代码接入关系，同时保留程序本来的求值行为。

## 语法

```text
lambda -> |anchored_parameter*| (-> expr)? expr
anchored_parameter -> id | id : expr
application -> expr(argument*)
post_lambda -> expr do (lambda | block | expr)
comptime_param -> comptime parameter
implicit_param -> implicit parameter
quote_param -> quote parameter
```

形参角色及组合限制见[参数 elaboration](../functions/parameter-elaboration.md)。编译器中的推断变量不增加源语言占位符。

## 正式定义

### 双向取得信息

检查表达式可以从其自身推得类型，也可以使用外部已经要求的类型检查它。对同一次调用，callee 签名、外围期望和各实参的合法信息共同约束本次推断变量。

后面的实参可以帮助前面的实参。它并不允许引用尚未在词法作用域建立的名称，也不把某一分支的假设传播到不满足该前提的位置。

各来源必须相容；先取得的信息不具有覆盖后来冲突信息的特权。改变合法检查工作的调度次序不能改变已经完成的语义选择。

### 刚性参数与可求解变量

定义端的量化参数按其契约检查，不能为了使函数体合法就把 T 固定成 i32。一次具体调用新建的推断变量则可以由实参、期望类型及约束确定。

运行时依赖 identity 也不是可任意赋值的类型未知项。一个引用已经绑定某个对象，就不能因为另一个对象具有相同类型而改绑。

范式应用保持头与实参；观察函数签名或类型分类不授权展开范式函数体。

### 角色接入与执行顺序

显式实参归属、post do 路由、implicit 供应、变参收集和 quote 接入先明确各自负责的位置。协议生成的结果仍须满足该位置的类型、阶段、资源和契约。

普通实参按既有调用规则求值一次。编译器先检查哪个实参，不决定生成程序先执行哪个实参。捕获 lambda 环境与执行 lambda 主体也分别发生。

候选试探的信息留在候选内。确定选择后才向外发布所选证据和调用计划；不可安全重放的 comptime 执行受[候选提交规则](candidate-solving.md)约束。

### 检查强度

Ordinary/loose 由源构造决定。缺少信息时可以请求合法的计算、使用已有契约或保留允许延迟的义务，但不能把 ordinary 自动降为 loose。

仅知道一个描述与 Any 相容，不足以合并不同类型或取得某个方法。PTC 负责类型命题，程序状态规格由程序验证层处理。

## 常见示例

### 后面的实参给前面的 lambda 提供类型

```flurry
use std.prelude.*;

fn apply_first(function: fn(T) -> U, value: T) -> U
where T, U
{
    function(value)
}

test {
    let result = apply_first(|item| item + 1, 41i32);
    println("{}", result);
}
```

41i32 约束 T；lambda 在相应上下文中取得 i32 输入，结果为 i32，最终输出 42。编译器不需要在刚遇到 item 时默认它是某个整数类型。

| 信息来源 | 对本次调用的作用 |
|---|---|
| apply_first 的签名 | 关联 lambda 输入、value 和调用结果 |
| 41i32 | 确定 T 为 i32 |
| lambda 主体 | 在已知输入类型下检查加法并确定 U |
| 所有约束共同成立 | 交付调用结果，不能只保存先成功的一部分 |

### 编译时信息流不重排 runtime 求值

接续 apply_first：

```flurry
fn produce() -> i32 {
    println("produce");
    41
}

test {
    let result = apply_first(|item| item + 1, produce());
    println("{}", result);
}
```

produce 的签名足以提供 i32，类型检查不执行 produce。运行测试时只输出一次 produce，然后输出 42；重试 lambda 的检查也不会重复调用 produce。

### 定义端不能猜测泛型参数

下面是拒绝例：

```flurry
fn invalid_increment(value: T) -> T
where T
{
    value + 1
}
```

T 没有给出支持该加法及结果类型的要求。某个未来调用传入 i32，不能使这个 ordinary 定义跳过检查。可把接口限定为具体类型，或者按所需运算的真实协议声明约束。

### 显式供应 implicit 输入

```flurry
use std.prelude.*;

fn shifted(value: i32, implicit bias: i32) -> i32 {
    value + bias
}

test {
    println("{}", shifted(4, .bias = 3));
}
```

输出 7。显式供应确定 bias 的来源；这里不需要搜索一个恰好位于作用域中的 i32。自动供应是协议的工作，不是一般词法同类型搜索。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 也以推断变量积累类型约束；C++ 模板实参推导则规定哪些位置参与推导、哪些位置不推导。不能把这些规则整体搬到 Flurry：本章明确允许同一次调用内的合法信息双向协作。[Rust 类型推断](https://rustc-dev-guide.rust-lang.org/type-inference.html)、[C++ 模板实参推导](https://eel.is/c++draft/temp.deduct)

Zig 可用 comptime 参数组织泛型调用；C 无对等的泛型 lambda 上下文推断，可用显式函数签名与函数指针传递操作。Flurry 的参数协议另见[参数 elaboration 的对照](../functions/parameter-elaboration.md#与-rust--zig--c--c-的关键差异)。

## 运行时实现设计引导

输出统一的实参准备与调用计划，记录每个显式输入的源码位置、每个缺省输入的依赖，以及值或代码的交付角色。普通值先取得后复用绑定，不把源码表达式复制到多个使用点。

静态 trait 搜索、类型推断和协议选择不进入普通 runtime 调用。需要运行时环境的闭包、开放实现或 handler，则交付已经检查过的环境取得关系。

## 编译器实现引导

### 合作检查一次调用

```text
elaborate_call(source, context):
    read callee signature and parameter roles
    assign source arguments to parameter positions
    create fresh variables for this call's inferable inputs
    connect result with the legal outer expectation
    create independently resumable work where needed
    inspect argument types and propagate visible constraints
    check lambda bodies when their required observations are available
    resolve applicable protocols and commit the chosen evidence
    build one runtime evaluation plan in source order
    return result, evidence, evaluation_plan
```

新建可恢复工作不要求每个子表达式都分配任务。直接可完成的观察沿普通递归路径返回；真正受阻的部分交给[局部调度](local-dependencies.md)。

### 事实范围

每条事实至少保留声明或调用来源、绑定环境、phase、分支前提和候选事务归属。共享存储只是一种实现，不授予跨这些边界使用事实的权利。

已知的类型关系可以推动其他任务；重试时仍检查前提有效。不能用失效候选曾经写入的类型作为 lambda 的永久上下文。

### 失败与成本

区分找不到名称、阶段不符、实参冲突、协议来源歧义和缺少操作依据。若信息生产者已经失败，后续任务报告关联失败或抑制重复诊断，不继续等待。

记录表达式检查次数、实际挂起点、协议执行次数和候选数。成本不仅取决于源码大小，还取决于类型计算与搜索；对同一事实的重复观察可缓存，但缓存不能跨越未经验证的上下文。
