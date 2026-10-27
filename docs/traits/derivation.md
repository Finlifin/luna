# 派生与策略

## Intro

许多类型实现某个接口时遵循相同的模式——例如序列化、相等性比较或哈希计算。手动编写这些重复代码既繁琐又容易出错。派生机制将这类可复用的构造方式命名为策略，derive 把策略应用到具体类型，从而得到接受普通检查的 implementation。

同一个接口可以有不同策略。选择策略决定生成什么代码；最终调用仍通过接口和实现，不需要记住生成器的名字。

## 语法

~~~text
deriver_def -> derive fn id (parameter*)? for expr clauses? { (assoc_decl | definition | statement)* }
derive_def -> comptime? derive expr for expr clauses?
handler_application -> expr.use(expr)
attr_def -> ^expr definition
~~~

用 `^attrs.derivation.default` 标记默认策略。显式选择写成 `derive TraitExpr.use(strategy) for TypeExpr`；该形式复用 `.use(...)`，不在成员投影中引入特殊参数位置。需要分别派生多个接口时可以写多条 derive。

## 正式定义

### 策略与产物

Deriver 是生成实现的编译时定义。它在给定接口与主体的上下文中构造关联项和方法；主体在其中作为 Self 使用。生成逻辑执行在编译时，不意味着生成的方法属于 comptime phase 或在 comptime 执行。

Deriver 可以声明参数，参数规则与 comptime fn 相同。参数依赖子句参数，各组合按声明次序建立依赖；参数的阶段、quote 输入及缺省计算遵循[参数阶段与子句](../functions/comptime_fn.md)。这些输入服务于实现构造，不自动变成每个生成方法的 runtime 参数。

策略身份不进入生成物的接口身份。同一主体、同一完整 trait 实例、同一 phase 的两种策略若产生同精度重叠实现，仍然冲突。

生成物先完成普通实现检查，再成为可依赖的事实。检查包括归属、重叠、关联项、方法契约、phase 和依赖交付。策略不能以生成代码的名义跳过它们。

### 显式与默认选择

显式 `.use(strategy)` 只选择指明的策略，并检查它是否适用于指定完整接口与主体；失败直接诊断，不暗中改用默认策略。

没有显式策略时，检索合法可见的默认策略，匹配其接口及要求。必须得到唯一适用来源；仍有可竞争的未决策略时不能提交。两个同时适用的默认策略不按导入次序决胜，也不把 implementation 主体精度算法擅自搬成策略优先级。

默认注册须有本地定义依据。对策略声明的完整 trait 表达式按声明头和实参次序观察；本地 trait 可以注册，否则首个本地名义身份前不能出现自由参数。具体生成物仍独立执行[包归属](coherence.md)检查。显式具名策略可以供其他包选择，但不因此授予产物额外的实现权限。

### 与类型适配接合

外部接口和外部主体不满足归属规则时，可以在本地名义类型上生成实现，再由显式接收者视角提供便捷调用。该视角不为底层外部类型注册全局实现。

`comptime derive` 选择生成 comptime phase 的实现；普通 derive 生成 runtime phase 实现。两者共享产物检查机制。

## 常见示例

不依赖反射也能写完整策略。这里为一个参数化接口生成固定标签：

~~~flurry
trait FormatLabel
where format: Symbol
{
    fn label(*self) -> str;
}

derive fn json_label for FormatLabel<.json> {
    fn label(*self) -> str {
        "json"
    }
}

^attrs.derivation.default
derive fn text_label for FormatLabel<.text> {
    fn label(*self) -> str {
        "text"
    }
}

struct Record {
    id: u32,
}

derive FormatLabel<.json>.use(json_label) for Record;
derive FormatLabel<.text> for Record;

test {
    let record = Record { id: 7 }
    println("{}", record.label<.json>());
    println("{}", record.label<.text>());
}
~~~

输出 `json`、`text`。label 本身没有子句参数；尖括号在已确定的参数化接口来源中选择 format。两份实现属于不同的完整接口实例，可以共存。

派生器也可以将生成逻辑的输入写成形参：

~~~flurry
derive fn label_from_text(text: str) for FormatLabel<.text> {
    fn label(*self) -> str {
        text
    }
}
~~~

text 是生成器的编译时输入，生成方法仍是 runtime 方法；不会要求 record.label() 每次再传一份 text。参数的合法取得遵循 comptime fn 规则，产物仍须通过普通实现检查。

例如，将前面 `.text` 的派生请求替换为：

~~~flurry
derive FormatLabel<.text>.use(label_from_text("record")) for Record;
~~~

这里的字符串在派生时交付给策略。生成方法使用编译时值时，仍遵循普通的跨阶段 Splice 规则；策略输入不会赋予绕过转换协议的权限。

可选命名输入遵循普通调用规则：未提供时在本次派生中计算缺省值；显式提供时不执行该缺省计算。前面的输入可以参与后面输入的缺省表达式。

~~~flurry
derive fn label_with_default(.text: str = "record") for FormatLabel<.text> {
    fn label(*self) -> str { text }
}

struct DefaultRecord {}
struct CustomRecord {}
derive FormatLabel<.text>.use(label_with_default()) for DefaultRecord;
derive FormatLabel<.text>.use(label_with_default(.text = "custom")) for CustomRecord;
~~~

实参的求值顺序不因命名输入与声明顺序不同而改变。重复供参、未知参数名以及输入类型错误仍按普通调用规则诊断。

`implicit` 不改变策略输入的阶段。显式供应仍写成 `.name = value`；前面的 implicit 输入也可以参与后面的缺省计算：

~~~flurry
trait FeatureFlag {
    fn enabled() -> bool;
}

derive fn configured(implicit enabled: bool, .value: bool = enabled)
for FeatureFlag {
    fn enabled() -> bool { value }
}

struct Feature {}
derive FeatureFlag.use(configured(.enabled = true)) for Feature;
~~~

这里生成方法对 `value` 的使用仍须满足普通 Splice 协议。当前实现支持显式命名供应；未供应的 implicit 输入仍由普通调用系统诊断，不能把当前尚未实现的自动供应当成默认 `false` 或省略参数。

`lambda` 标记可以把 post-do 代码交付给策略的编译时回调。下面的回调在构造实现时计算结果，生成方法只读取该结果：

~~~flurry
derive fn transformed(value: bool, lambda action: comptime fn(bool) -> bool)
for FeatureFlag {
    let result = action(value);
    fn enabled() -> bool { result }
}

struct Transformed {}
derive FeatureFlag.use(transformed(false) do { not _0 }) for Transformed;
~~~

也可以显式交付 `transformed(false, |value| not value)`。回调按需执行；未使用的回调仍接受类型与阶段检查，但不会仅因作为实参被交付就调用其主体。多个 lambda 入口的歧义及 runtime 调用泄漏均按普通 comptime 调用规则拒绝。

变参输入交付一个完整的收集值。下面的策略先在编译时解构两个标志，只让生成方法捕获计算出的布尔结果：

~~~flurry
trait PairFlags {
    fn enabled() -> bool;
}

derive fn from_flags(...flags: Tuple<bool, bool>) for PairFlags {
    let (enabled, blocked) = flags;
    let allowed = enabled and not blocked;
    fn enabled() -> bool { allowed }
}

struct Feature {}
derive PairFlags.use(from_flags(true, false)) for Feature;
~~~

本例也可用 `from_flags(.flags = (true, false))` 交付整包，或用 `from_flags(...(true, false))` 展开实参。收集类型保留 Tuple 的 normal-form 身份；空收集、末尾固定参数以及扩展调用中的属性收集均遵循[函数调用](../functions/calls-and-returns.md)的规则。

选择另一个策略不会制造第二层方法身份：

~~~flurry
-- 接续上例；加入最后一行将产生冲突。
derive fn compact_json_label for FormatLabel<.json> {
    fn label(*self) -> str {
        "compact json"
    }
}

-- 拒绝：Record 已有同一接口实例的同精度实现。
derive FormatLabel<.json>.use(compact_json_label) for Record;
~~~

序列化采用同样的策略入口，例如 `derive Serialize<.json>.use(by_repr) for Student`。策略体可以依据类型的合法结构观察生成序列化方法，但字段编码、格式选择和失败行为仍由 Serialize 及策略自身的契约规定。

## 与 Rust / Zig / C / C++ 的关键差异

Rust derive 属性可调用派生宏生成实现；Flurry 将具名策略选择写进 derive 的接口表达式。[Rust derive](https://doc.rust-lang.org/reference/attributes/derive.html)

C 无对等语言机制，可由外部生成器生成函数；Zig 可以用 comptime 构造和检查代码所需信息；C++ 模板可复用实现。Flurry 的产物统一回到普通 implementation 检查。[C11](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf)、[Zig comptime](https://ziglang.org/documentation/master/#comptime)、[C++ constraints](https://eel.is/c++draft/temp.constr)

## 运行时实现设计引导

运行时只需要生成后的方法及实际依赖，不需要保留 deriver 的运行引擎。生成的方法仍可能分配、访问环境或产生效应，不能因为生成器在 comptime 执行就将这些行为提前执行或消除。

## 编译器实现引导

1. 解析 derive 的接口表达式；`.use(...)` 的操作数交付显式策略身份。
2. 解析主体和目标 phase；缺少显式策略时检索默认注册。
3. 隔离策略候选推断，检查唯一性和开放要求。
4. 在 Self、接口实参和合法反射环境中实例化策略。
5. 为产物保留生成来源，执行普通 implementation 全套检查。
6. 验证成功后登记产物；失败不得留下半份可见实现。
7. 将生成代码交付后续实例化，保留其 runtime 依赖和效果。

递归派生需要记录目标、策略和替换，检测无进展环并限制展开资源。缓存依赖策略主体、输入类型、观察到的结构、接口实参、phase 和实现世界；仅以策略名缓存会混淆不同产物。

诊断同时显示 derive 使用点、策略定义和失败的生成成员。重复处理同一生成请求可以按身份去重；两条独立 derive 产生的重叠实现不能只因方法文本相同就去重。
