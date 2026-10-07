# 函数级模式与分散定义

## Intro

一个操作面对不同输入形态时，可以把主体写在对应的参数模式旁。调用者仍调用同一个接口；模式成功后，主体直接使用这次匹配取得的绑定。

代码生成还需要复用这样的“模式加主体”。一条可接入的规则必须保留两者之间的绑定关系，才能把分散定义的处理分支安全地组装到目标位置。

## 语法

~~~text
function -> pure? comptime? inline? (unsafe | spec | verified | atomic)? fn id(parameter*) (-> result)? clauses? (block | (= expr))?
quote_function -> quote fn id(parameter*) (-> result)? clauses? (block | (= expr))?
case_param -> case pattern
template_quote_signature -> template quote fn id(parameter*) (-> result)? clauses?
~~~

pattern 复用[全局语法](../grammar/syntax)。open fn 指包内封闭的函数级定义集合；这里使用已有 fn 签名及 case 参数，不另引入一个函数修饰词。

## 正式定义

### 公共接口与分支定义

公共签名规定输入、结果及调用契约。带 case 的参数在对应锚定位置上描述模式。case 不增加一个 runtime 参数，也不把该位置变成可选输入。

调用时先按普通调用规则取得实参，再进行相应匹配。模式成功产生的绑定只在对应主体中可用；该主体须兑现公共结果及契约。失败匹配没有交付成功绑定，不能把成功路径中的值带到其他分支。

多参数分支匹配的是一组输入的组合。一个参数位置上的模式覆盖了其整个类型，不意味着整组组合已经覆盖所有调用。

### 定义集合的边界

open fn 的定义集合在所属 package 内封闭。分散到不同源码位置不使外部依赖包获得任意添加分支的能力。名称解析、可见性及来源身份按[代码组织](../code-organization/index.md)处理。

文件扫描次序不提供程序可以依赖的模式优先级。下面的例子使用互斥且完整的输入分区，不依赖重叠分支的选择。模式条件和效果仍须满足相应匹配规则，不能把“尝试一个分支”解释成已经执行其成功主体。

### Quoted arm 的绑定方向

quoted code 有 expr、pattern 和 arm 三类。它们的关键区别如下：

| 代码类别 | 对绑定的使用 | 接入后要检查的部分 |
|---|---|---|
| expr | 使用 contextual 接口提供的绑定 | 表达式结果、效果及资源交付 |
| pattern | 匹配成功后提供绑定 | 输入匹配、成功绑定、失败路径 |
| arm | 关联一个模式与使用其成功绑定的主体 | 模式、成功路径、结果及外部上下文 |

arm 通过 quote fn 加函数级模式表达，不增加独立的 quote arm 表面语法。具体的模式输出上下文、定向检查与接合算法见 [CMTT 模式与 arm](../cmtt/patterns-and-arms.md)。两个分支即使都写名为 value 的绑定，仍分别属于各自匹配；变量拼写相同不能使它们共享对象身份。

### 模板与 open code

模板提供规则的共同接口，分散定义提供具体规则，目标位置接收组合后的代码。这适用于有大量子入口的编译器 pass 或路由集合。只共享一个计算或一份函数签名时，使用普通函数或[简单模板](templates.md#签名抽象)即可。

组合必须保留每条规则的输入模式、外部 runtime 依赖、成功绑定和主体。目标仍检查类型、控制流、覆盖及资源要求；代码生成并不免除这些义务。quoted code 本身是 comptime 值，目标操作发生在生成后的 runtime 程序中。

## 常见示例

### 按命令形态分派

~~~flurry
enum Command {
    idle,
    read { slot: u32 },
    write { slot: u32, value: u32 },
}

fn describe(command: Command) -> ?u32;

fn describe(case .idle) -> ?u32 {
    println("idle");
    null
}

fn describe(case .read { slot }) -> ?u32 {
    println("read {}", slot);
    slot
}

fn describe(case .write { slot, value }) -> ?u32 {
    println("write {} = {}", slot, value);
    slot
}

test {
    let target = describe(Command.write { slot: 2, value: 7 });
    target match {
        null => println("no target"),
        slot? => println("target {}", slot),
    }
}
~~~

输出 `write 2 = 7`、`target 2`。调用仍只交付一个 Command；write 分支从它取得 slot 和 value，再计算公共接口要求的 ?u32。read 与 idle 的主体没有执行。

### 对照普通 match 的绑定关系

接续 Command，下面将相同的处理写在一个主体中：

~~~flurry
fn describe_local(command: Command) -> ?u32 {
    command match {
        .idle => {
            println("idle");
            null
        },
        .read { slot } => {
            println("read {}", slot);
            slot
        },
        .write { slot, value } => {
            println("write {} = {}", slot, value);
            slot
        },
    }
}
~~~

两个版本都只在成功分支中提供相应字段。函数级写法把这些主体分开组织，没有给调用者增加另一套传参方法。

### 多参数模式覆盖整个组合

~~~flurry
fn smaller_present(first: ?u32, second: ?u32) -> u32;

fn smaller_present(case null, case null) -> u32 {
    0
}

fn smaller_present(case first?, case null) -> u32 {
    first
}

fn smaller_present(case null, case second?) -> u32 {
    second
}

fn smaller_present(case first?, case second?) -> u32 {
    if first < second {
        first
    } else {
        second
    }
}

test {
    println("{}", smaller_present(null, null));
    println("{}", smaller_present(7, null));
    println("{}", smaller_present(null, 9));
    println("{}", smaller_present(7, 9));
}
~~~

输出 `0`、`7`、`9`、`7`。四个组合分别覆盖缺失／存在的两维输入；最后一个主体同时获得两个 u32。删去最后一个分支会留下“两者都存在”的未覆盖输入。

### 绑定只在自己的分支内存在

接续 Command，以下是独立拒绝例：

~~~flurry
fn invalid(command: Command) -> u32;

fn invalid(case .idle) -> u32 {
    slot
}

fn invalid(case .read { slot }) -> u32 {
    slot
}

fn invalid(case .write { slot, value: _ }) -> u32 {
    slot
}
~~~

idle 分支没有产生 slot。另两个分支声明同名绑定，不为 idle 提供输入；不能从其他规则的局部名字推导这里有一个可读值。

### Quote fn 保留模式与成功主体

~~~flurry
quote fn value_or_zero(value: ?u32) -> u32;

quote fn value_or_zero(case null) -> u32 {
    0
}

quote fn value_or_zero(case value?) -> u32 {
    value
}

fn input() -> ?u32 {
    println("input");
    7
}

test {
    println("{}", value_or_zero(input()));
}
~~~

输出 `input`、`7`。代码接入必须完成，输入仍按普通实参规则求值一次。存在分支中的 value 是这次成功匹配交付的绑定；接入时需要同时保存模式和主体对它的引用。不能先把主体当作一段无上下文表达式拿走，再期待同名变量补齐它。

该例将完整分支集合用于一个 quote fn 接口。用于目标匹配结构的分散规则，还须由模板接口交付所需的外部上下文，并在目标处完成相应覆盖和结果检查。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 可在函数体中使用 match，C、C++ 和 Zig 可在函数体中分派输入，再调用对应辅助函数。Flurry 的函数级模式直接将输入模式与各个主体关联；它也不同于仅依据静态参数类型选择函数的重载。[Rust match](https://doc.rust-lang.org/reference/expressions/match-expr.html)、[Zig switch](https://ziglang.org/documentation/master/#switch)。模板层的对照见[函数模板](templates.md#与-rust--zig--c--c-的关键差异)。

## 运行时实现设计引导

普通函数级分支可降为一个匹配决策结构。先取得整次调用的实参，再观察所需标签与载荷；仅执行成功主体。可以共享判别步骤，但不能重复计算源实参或把成功路径的移动提前到仍可能失败的位置。

分支主体的结果进入公共结果位置。各路径分别清理剩余责任，匹配绑定的作用域随路径保持。quote fn 的接入可消去独立调用边界，仍须在生成程序中保留这份控制流与责任关系。

## 编译器实现引导

1. 确定公共接口身份及所属 package，收集归属于它的分支定义。
2. 以公共签名为每个 case 参数提供输入类型，并检查对应位置。
3. 对每条分支建立独立的成功绑定环境，再检查主体和公共结果契约。
4. 将参数模式组合送入相应匹配检查，记录覆盖、失败路径和分支入口事实。
5. 对 quoted arm 同时保存模式、外部上下文、成功绑定、主体和来源；接入时对它们做一致替换。
6. 在组合后的目标上重新核对结果、效果、资源状态和退出清理。
7. 生成决策结构与调用／接入实现，保持原始实参求值次数。

分支登记的基础成本可按声明数线性增长；模式覆盖、条件证明与代码接入成本另计。不能以文件枚举顺序代替未交付的分派依据，也不能因两个分支环境字段数相同就复用成功绑定。

诊断应分别指出：分支所属接口无法确定、case 与锚定位置不符、成功绑定缺失、结果不满足公共要求、输入组合未覆盖，或规则上下文无法接入目标。错误定位同时保留分支源码与实际聚合位置。
