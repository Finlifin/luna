# 成员投影与来源

## Intro

点号告诉程序去取一个成员。对参数化代码而言，关键问题不只是“有没有这个名字”，还包括“这个名字由哪份接口提供”。

Flurry 在定义检查时保留选定的成员来源。实例化可以兑现该来源的具体实现，不能让同一段已检查代码悄悄改调用另一个同名成员。这种来源稳定性保证了参数化代码在实例化前后具有一致的语义。

## 语法

~~~text
projection -> expr.id
as_expr -> expr.as(expr)
application -> expr(argument*)
~~~

路径沿用投影语法。Self 等名字见[反身代词](self-reference.md)，自动接收者处理见[适配](receiver-adaptation.md)。

## 正式定义

类型侧查找使用该类型自身的成员容器及静态再导出，再按相应 implementation 来源查找。值侧先使用当前命题上下文提供的 trait 能力，再考虑结构和类型成员。

这里的类型成员容器不表示把类型名和值名分成不同的词法命名空间。名称遮蔽与路径遵循[统一查找规则](../code-organization/scope-and-paths.md)，公开入口遵循[再导出规则](../code-organization/imports-and-reexports.md)。

已选定的成员来源在实例化中保持。具体输入可以确定该来源对应的实现与关联项，不等于重新运行一次允许不同来源获胜的名称选择。

字段存在、字段可见、方法来源唯一和调用合法是不同条件。PTC 的存在性事实不能代替完整成员选择。

当候选能力指向不同来源，先应用当前阶段的来源规则：未限定的 comptime 查找优先考虑 `pure comptime trait` 的合法实现来源；同一优先层仍不唯一时，必须通过显式实现视角或精化要求消歧。实现视角参数是表达式，允许选择完整的参数化 trait 实例。

### comptime-only 接口的来源选择

`pure comptime trait` 只含 comptime phase 成员，`pure` 表示 only，不表示函数
无副作用。对两阶段均存在的类型，runtime 与 comptime 的同名方法可以分别放在
普通 trait 与 pure comptime trait 的实现中。

在原有成员查找允许的 trait 来源范围内，未限定的 comptime 查找先考虑
pure comptime trait 来源。普通 trait 中仅靠成员属性标记的 comptime 方法不
取得该声明级优先级；两个 pure comptime trait 仍可产生同级歧义。
优先层存在未决竞争者时等待，不能把失败的求解或调用当作回退信号。
完整规则见 [Trait 两相](../comptime/the-comptime-phases-of-traits.md#同名方法的-comptime-来源优先级)。

显式 `.as(Trait)`、已固定的成员来源及绑定到特定接口的语法协议不参加重新
选择。这个规则不改变字段、固有成员或词法名称的既有查找层次，也不允许实例化
改选另一接口。检查所在表达式的实际阶段，而不是仅依据外围定义是否含
`comptime` 关键字。

### 参数化接口成员

成员来源为参数化 trait 时，可以先投影成员，再以范式应用补齐接口参数，最后调用方法。例如 `student.serialize<.json>()` 的参数属于 Serialize 的 format，普通 serialize 方法仍执行普通调用。

此形式由已有投影、范式应用和调用组成。参数必须能唯一归属于已确定的开放来源；多个接口都有同名成员时，不通过试填参数随意选择来源，可以写 `student.as(Serialize<.json>).serialize()`。

开放 implementation 自身的依赖则在实现视角上绑定，例如 `value.as(Trait)<.space = space>.method()`；见[替换与依赖交付](substitution-inference.md)。这与接口实参、方法自己的实参分别检查。

## 常见示例

让两个接口都提供名为 `code` 的方法。它们服务不同用途：一种是设备协议编号，另一种是界面显示编号：

~~~flurry
trait WireCode {
    fn code(*self) -> u32;
}

trait DisplayCode {
    fn code(*self) -> u32;
}

struct Status {
    value: u32,
}

impl WireCode for Status {
    fn code(*self) -> u32 {
        self.value.*
    }
}

impl DisplayCode for Status {
    fn code(*self) -> u32 {
        self.value.* + 100
    }
}

fn protocol_code(status: *T) -> u32
where T :- WireCode {
    status.code()
}

test {
    let status = Status { value: 7 }

    println("wire: {}", status.as(WireCode).code());
    println("display: {}", status.as(DisplayCode).code());
    println("generic wire: {}", protocol_code(status.ref));
}
~~~

输出为 `wire: 7`、`display: 107`、`generic wire: 7`。前两次调用通过实现视角明确指定来源。`protocol_code` 在定义处根据 `WireCode` 要求检查，因此实例化为 `Status` 后仍使用协议接口。

这与“具体类型上最后找到哪个同名方法就调用哪个”不同。若把两个能力都带入上下文而未指定来源，应按成员选择规则解决歧义，不能仅凭返回类型相同选一个。

同样的来源要求也适用于关联类型。例如在已知 `MyPtr` 满足 `Deref` 的环境中，`MyPtr.as(Deref).Target` 指向该实例的关联项；同名的其他关联项不会自动与它合并。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 提供限定路径来消除 trait 成员歧义，C++ 具有限定名与模板名称查找规则；C 使用显式函数或函数指针，Zig 可显式选择命名空间。Flurry 的 .as(...) 实现视角和来源稳定性按本篇定义。[Rust 限定路径](https://doc.rust-lang.org/reference/paths.html#qualified-paths)、[Rust 方法查找](https://doc.rust-lang.org/reference/expressions/method-call-expr.html)

## 运行时实现设计引导

静态实现调用可以直接使用选定入口；若调用机制需要运行时证据，则交付对应来源的证据。不能仅用方法名生成一个全局动态查找。

关联类型本身静态消去，但它决定的参数与返回表示必须与实际入口一致。

## 编译器实现引导

成员解析结果应含：主体、来源种类、声明身份、完整实例参数、签名、必要要求、关联项替换及接收者动作。涉及 runtime dependency 时还须保留实现证据和环境取得计划。

1. 收集当前规则允许的来源。
2. 依据词法环境与 PTC 证据检查可用性。
3. 对未限定的 comptime trait 查找应用 pure comptime 来源优先层；保留未决竞争者，确定唯一来源或报告同层歧义。其余来源沿用原规则。
4. 固定来源，生成后续调用要求。
5. 实例化只兑现该来源所需的具体实现。
6. 验证关联项与调用 ABI 使用同一份证据。

不得以 HasSlot 或 Implements 的一个 bool 替换这些信息。诊断应展示冲突的来源，帮助作者选择显式视角。
