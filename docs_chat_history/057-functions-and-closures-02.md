# 函数与闭包：参数协议、上下文与表示边界

- 日期：2026-09-27
- global_index：057
- theme：functions-and-closures
- subindex：02
- 本轮目标：收束首轮审阅，吸收作者对参数体系和类型表示的补充，继续预填下一轮。

## 作者输入摘要与来源

作者消息为“已审阅。另有补充。”作答文件保留全部 F1–F6 预填，在末尾补充适用于 effect、fn、lambda、子句、typealias/newtype、normal form head、predicate、lemma、case def 的参数分类及相互依赖。

补充包括锚定参数与裸 id、trait bound 糖、implicit/comptime/quote、lambda 标记和 tactic lambda、可选参数的必需默认表达式、两种变参及 elaboration 协议、case 与 package 封闭的 open fn、接收者和 builtin.Continuation。derive fn 遵循 comptime fn 的参数规则；四类输入可显式按名称供应；template fn 与 template test 有不同的 body 绑定入口。作者还明确某些嵌入层或携带语言项的类型不能用户 impl，并要求裸 trait object 固定为两个 usize，以闭包限定表达所需环境。

作者逐字原文已追加到 056。本记录的归纳不替代该原文。以下完整 prompt 区分直接确认、受托选择和仍待审阅的候选。

## 可行性评估与写入边界

参数的绑定形态、供应来源、阶段及语法角色分别建模，能够保留 modifier 的组合能力，而不错误地把每种组合当成新运行时函数类别。具体合法组合仍以作者确认的构造范围为准。

关键风险在三个交界：elaboration 定制必须与已确认的实参求值规则相容；运行时默认值限制不能未经确认覆盖既有 comptime 默认政策；特殊类型的用户实现资格与 dyn 的环境交付必须保留语言对象及资源依赖。六项问题分别附互斥候选和预填，不预设本轮预填已获批准。

本轮未获函数主题正式写入授权，因此只更新 prompt/history。作者直接纠正的 dyn 表示、contextual 拼写和闭包参数范围列为后续文档同步项，不再次询问是否接受这些纠正。

## 完整新 prompt

# 函数与闭包：参数协议、上下文与表示边界

日期：2026-09-27；history：057；theme：functions-and-closures；第 02 轮。

**DRAFT — NOT CANONICAL。** 本轮归纳作者补充、承接上轮选择，并预填六个尚需确定的交界问题。只更新 prompt/history，尚不改写正式 docs。

## 一、上轮六项收束与本次纠正

上轮 F1–F6 的预填均保持不变；按已审阅收束为以下规则，并以本次补充限制它们的适用范围：

| 项目 | 承接结果 |
|---|---|
| F1 调用次序 | 先 callee／接收者，再按源码顺序求显式实参；各求值一次 |
| F2 可选参数 | 有默认表达式，可依赖前面的参数；只在缺省时求值。runtime 默认值的纯度在下文补足 |
| F3 捕获 | 默认按值：Copy 复制，非 Copy 移动；需要共享观察时显式捕获普通指针或视图 |
| F4 字段捕获 | 对能够证明独立交付的内建投影采用最小位置；保留真实依赖及剩余清理要求 |
| F5 捕获清理 | 去重后的首次捕获词法顺序形成语义字段，按逆序清理；物理布局不改可观察次序 |
| F6 普通 fn | 不隐式承接任意非空环境；需要环境时保留 `%C` 等实际交付 |

新增内容对先前记述有两项明确纠正，直接以你的补充为准：

1. quote fn 的 contextual 隐参使用 **implicit quote**。不再将先前的 contextual 属性当作现行方案。
2. **裸 dyn Trait 的大小固定为两个 usize。** 我先前在动态接口文档里写“语言不固定 dyn 必须是两个机器字”过度扩张了表示；所需环境由闭包限定表达，例如你给出的 `%{space: Space = space} dyn Drop`。

“闭包只有锚定参数”也明确收窄了现有 grammar 中 lambda 直接复用全部 parameter 的写法。后续修正文法时，要按构造种类检查参数，不能仅凭递归前缀产生式就接受所有组合。

## 二、参数系统的全貌

### 1. 分开记录四种信息

我建议实现和文档分别回答下面四件事：

- **绑定形态**：一个实参对应一个位置，还是收集一组位置实参／属性实参？
- **供应来源**：调用者显式给出、默认表达式补足，还是 elaboration 的 implicit 协议供应？
- **阶段与对象**：runtime 值、comptime 值，还是保留运行时符号的 quote 输入？
- **语法角色**：普通数据、lambda 表达式边界、case 模式、接收者、continuation 或 template 主体？

这是一种描述分工，**不表示任意修饰符都能任意组合**。每个声明位置仍按你确认的能力表检查。

### 2. 作者补充的规则地图

| 类别 | 已明确的含义与边界 |
|---|---|
| 锚定参数 | 一个 param 对应一个 arg；闭包的调用参数只有这一类 |
| 纯 id | lambda 中省略类型，由上下文等取得类型依据；typealias/newtype、子句及 normal form head 中的类型参数可缩写为 `id: Type`；MemoryEffect predicate 的特殊含义见 G1 |
| trait bound | 子句中的 `T :- TraitExpr` 脱糖为 `T, requires T :- TraitExpr`；不是另一种独立运行时实参 |
| implicit | elaboration 自动供应，可加 comptime，也可加 quote；允许 trait 提供 elaboration 协议 |
| implicit quote | 作为 contextual 变量，主要用于 quote fn；保留符号及绑定，不提前读取 runtime 内容 |
| lambda 标注 | 标明 post do 的目标，也为 tactic lambda 表达式建立解释边界 |
| comptime | 规定相应输入的阶段；不据修饰位置自行推导整个定义的执行类别 |
| optional | 可选供应，必须有默认表达式，可为 comptime，可依赖前面的参数 |
| 变参 | 默认将多个位置输入收集为元组，可为 comptime，可通过 trait 定制 elaboration |
| 拓展变参 | 默认将属性输入收集为匿名结构体，可为 comptime，可通过 trait 定制 elaboration |
| quote | 用于 comptime fn 和子句，指代 runtime 符号；允许 optional 形式；另有上面的 contextual implicit 用法 |
| case 参数 | open fn 的锚定参数模式，承载函数级模式匹配；open fn 的定义集合在 package 内封闭 |
| self／*self | 沿用已经确认的接收者及其资源语义 |
| catch | 绑定 continuation，目前固定 builtin.Continuation；实际操作接近 intrinsic，不能据此声称已经有很强的使用检查 |
| derive fn 参数 | 遵循 comptime fn 的参数规则，不因是派生器而独立设计一套参数机制 |
| template fn 主体 | 通过 quote 与 lambda 两个修饰标记绑定 body |
| template test 主体 | 通过唯一一个锚定子句参数绑定 body；不由“唯一锚定”推出禁止其他类别子句参数 |

参数依赖按你给出的方向组织：**子句参数 → 参数 → 返回值**，各组合内部按顺序依赖。参数可以依赖子句参数，返回值可以依赖两者；这个依赖检查次序与调用现场的 runtime 求值次序分别描述。

### 3. 显式命名供应不只针对 optional

你列出的四类——变参、拓展变参、implicit、optional——都可以按 `.name = expr` 直接供应。这里应把旧 grammar 的 optional_arg 在说明层面改成更一般的“显式命名实参”。

下面把三个尺寸收集成一个元组，也允许直接交付完整元组：

~~~flurry
fn volume(...shape: Tuple<u32, u32, u32>) -> u32 {
    let (width, height, depth) = shape;
    width * height * depth
}

test {
    let first = volume(2, 3, 4);
    let second = volume(.shape = (2, 3, 4));
    println("{} {}", first, second);
}
~~~

两个结果都是 24。第一个调用需要元组收集，第二个已经交付整个 shape。后一种不能再包装成“只有一个元组元素”的外层元组；直接供应也不执行缺省收集或 implicit 供应。

同一位置重复供应、或既直接交付完整包又向同一个包继续收集，应作为重复绑定处理，不悄悄拼接或覆盖。这是下面参数协议候选的唯一供应约束，不新增一种调用语法。

## 三、默认表达式的纯度选择

你让我选择层级，我选择：**runtime 可选参数的默认计算采用 P3 稳定观测，并要求在合法入口下有限完成。** 不新增第七个纯度等级；总性是已有的独立保证。

选择 P3 的理由是：默认值常常要观察前面参数的长度、配置字段或视图。只允许 P4/P5 会把这些常见操作排除；允许有界写入则会把省略实参变成隐含状态变更。

具体含义：

- 可以使用前面参数及合法、稳定的状态读取；每次调用观察本次状态，不跨版本盲目缓存。
- 默认计算及它所需的隐式适配、临时清理一起检查，不能把分配或输出藏到转换里。
- 不隐式修改调用者可观察状态，不输出、不交付新的可观察资源身份；需要这些行为时在调用处显式供应实参。
- 前提成立时有限完成；运行检查失败是调用准备的独立失败路径，不能把“默认表达式总性”当作删除前提检查的许可。
- Copy、move 和依赖交付仍按资源规则检查；纯度不允许同时把同一份非 Copy 责任交给两个参数。

~~~flurry
struct Limit {
    value: u32,
}

fn read_limit(source: *Limit, .limit: u32 = source.value.*) -> u32 {
    limit
}

test {
    let source = Limit { value: 1 }
    let first = read_limit(source.ref);
    source.value = 2;
    let second = read_limit(source.ref);
    let supplied = read_limit(source.ref, .limit = 9);
    println("{} {} {}", first, second, supplied);
}
~~~

输出为 `1 2 9`。最后一次不读取默认来源字段；前两次没有因为指针位值相同而复用旧的默认结果。前提及其实际需要的路径仍由契约检查。

此选择针对 runtime 默认计算；它与既有 **comptime 可选子句默认值可不纯** 的边界，需要在 G4 明确，不能悄悄同时改掉以前确认的规则。

## 四、对 elaboration 的理解

下面是候选的局部实现骨架，不是新增面向用户的编译阶段：

~~~text
从已定位定义的粗 HIR 签名读取参数角色
  → 识别显式供应、收集入口与缺省位置
  → 为 post do、tactic lambda、template body 确定接入边界
  → 按依赖关系完成阶段、类型及相应 trait 协议检查
  → 形成带来源及求值次序的参数交付计划
  → 在规定阶段执行表达式，并按真实控制流交付实参
~~~

关键是保留两个区别：

1. **elaboration 在编译时发生，不等于生成的实参计算也在编译时执行。** 候选接口按参数角色区分编译时数据与生成的 runtime 实参表达式；具体允许的结果由所选协议定义。
2. **建立 lambda 的表达式边界，不等于求值它的主体。** `_0 < _1` 是未来调用中的计算；不能先在外层调用点把它求成 bool 再尝试包装。

post do 的路由先读签名里的 lambda 标记；无标记时接到最后一个锚定参数。候选实现不凭参数类型“看起来像函数”随意寻找其他槽位。多个可行目标或目标已被供应时，应报告歧义／重复，不任意覆盖。

vararg 的 trait 协议可以改变如何形成那一个收集结果，但不能仅因发生在 elaboration 中就省掉生成程序的访问、责任和阶段检查。具体协议范围见 G3。

## 五、六项预填审阅

### G1. MemoryEffect predicate 的纯 id 是否省略了 quote 参数声明？

你特别指出纯 id 在 predicate 中主要服务于返回 MemoryEffect 的定义。结合之前的 modify，我最自然的解释是：

~~~flurry
predicate modify(target) -> MemoryEffect {
    read<target> and write<target>
}
~~~

候选展开为：

~~~flurry
predicate modify(quote target: lift Any) -> MemoryEffect {
    read<target> and write<target>
}
~~~

两段是同一定义的替代写法，不同时声明。

- **A. 在结果明确为 MemoryEffect 的 predicate 中，纯 id 是这个 quote 参数的简写。** 可直接组合内存区域规格，避免把 target 误当 Type 值；其他 predicate 不据此自动引用参数。
- **B. predicate 的纯 id 仍只表示 Type 参数，区域参数必须显式写 quote。** 规则更统一，但上面的第一种写法不能用于这个区域规格。

**预填答复：A。** 仅在这一明确的定义语境中采用简写。普通 predicate 默认返回 bool，并不因此获得隐式 quote。其余语境的纯 id 继续按相应参数种类解释，不建立“所有裸 id 都代表 Type”或“所有裸 id 都被 quote”的通法。

### G2. tactic lambda 的完整参数表由目标签名决定吗？

~~~flurry
fn compare_pair(first: u32, second: u32, lambda compare: fn(u32, u32) -> bool) -> bool {
    compare(first, second)
}

test {
    let ordered = compare_pair(2, 3, _0 < _1);
    let same_first = compare_pair(2, 3, _0 == _0);
    let explicit = compare_pair(2, 3) do |first, second| first < second;
    println("{} {} {}", ordered, same_first, explicit);
}
~~~

第二个表达式只出现 _0，但接收位置要求两个锚定参数。第三个展示已确认的 post do 路由：签名中的 lambda 标记定位 compare，显式 lambda 的主体仍等到调用时执行。

- **A. 参数数目和类型来自目标签名，占位符只引用相应参数。** 两个调用都合法；第二个相当于一个忽略第二参数的二元闭包。
- **B. 参数数目由表达式中的最大占位符下标决定，再与目标适配。** 第二个先成为一元闭包，需要另定补参数适配，或者拒绝它。

**预填答复：A。** 越界下标拒绝；同一下标重复出现引用同一个参数，并照常检查 Copy／仿射交付。不能把 `_0` 的重复出现变成重复计算外部实参。

候选作用域规则：在 lambda 标注位置建立边界；普通子表达式继承它。嵌套的另一个 lambda 标注位置建立新边界，显式 lambda 也形成独立边界。占位符不隐式跨这些边界引用外层参数；需要跨层捕获时改写成具名的普通 lambda。

未标记的参数位置，仍可显式传入 `|first, second| first < second`，但不能在那里单独启用 tactic lambda。post do 的缺省路由本身也不把未标记参数永久变成 tactic lambda 入口。

### G3. 参数 trait 的 elaboration 能否改写其他实参或外层调用？

这里有一个真正影响协议可组合性的边界：收集协议收到若干源输入后，是交付自己参数的位置，还是获得整个调用的控制权？

- **A. 协议只负责其参数的供应或收集，并交付可检查的结果；外层仍负责统一绑定与求值次序。** 不得隐式重排、重复或吞掉其他实参；需要延期代码时使用明确的 quote／lambda 角色。
- **B. 协议可接管整个调用并重新组织其他参数与控制流。** 元编程能力更大，但多个协议并存时还需规定优先权、重写冲突与调用语义由谁拥有。

**预填答复：A。** 先把协议限制为可组合的参数入口。显式 `.name = expr` 直接交付完整值时，跳过缺省供应／收集，仍核验适配与契约。原始表达式可用于编译时类型分析；其 runtime 效果仍按调用计划各执行一次。

具体候选执行计划：先按 F1 求显式 runtime 实参；随后按参数声明依赖顺序补齐缺失的 implicit 和 optional 位置。comptime 求值及符号绑定按自己的阶段完成，不把它们伪装成 runtime 指令。若某协议确实需要打破这个计划，应通过显式的语言入口另行说明，不能借 trait hook 暗改普通调用。

这里不指定协议的 trait 名、关联项名或私有 IR API；那些应在协议接口专节里按用途设计。

### G4. 默认值的纯度限制是否只适用于 runtime 默认计算？

本轮已选择 runtime 默认采用 P3 加有限完成；另一个现行定义是：

[comptime/purity-and-normal-form.md](/home/fin/luna/docs/comptime/purity-and-normal-form.md) 明写：

> 可选子句参数的默认值在 normalization 中直接执行，不要求该表达式纯。需要延迟时由 defer 包裹。

这与“所有可选参数默认都受同一纯度限制”不能同时直接沿用。

- **A. 这次 P3 要求约束 runtime 默认计算；comptime 与子句默认继续按既有阶段求值政策处理。** 保留已经确认的 normalization／defer 规则，不因参数表面相近就合并两个阶段。
- **B. 所有可选默认值都需具备对应阶段的受限纯度，同时修订原 comptime 定义。** 跨阶段更统一，但编译时默认值中的诊断、环境读取等能力需要按新界限重新核对。

**预填答复：A。** 不额外把 runtime 的完成性和内存状态等级强加到编译时求值上。comptime 可选输入仍须在其使用阶段取得合法值；需要的计算未完成，不能用“缺省”代替结果。

这一题只确认规则覆盖范围，不重新让你选择我受托决定的 runtime 纯度层级。

### G5. 不允许用户 impl 的类型，是否也排除泛型实现的间接覆盖？

你已经明确 reachability type、CMTT 类型、闭包类型等不能直接按普通类型使用 impl。还需要避免下面的泛型实现不经意绕过这条边界：

~~~flurry
trait Marker {}

impl Marker for T
where T
{}
~~~

- **A. 限制针对最终主体类型，直接 impl 与泛型匹配都受它约束。** 这个实现只覆盖允许用户实现的主体；泛型代码也不能因此无条件断言所有 T 都有 Marker。
- **B. 只禁止对这些类型写直接实现头，允许符合条件的泛型实现覆盖它们。** 仍能写通用 Marker 一类接口，但“不能 impl”实际只限制一种声明形式。

**预填答复：A。** 在候选适用性中检查主体是否允许用户实现；typealias 不隐藏真实主体。合法的名义包装仍按自己的主体和资源契约检查，不因此把内部语言对象变成普通可构造值。

“不能用户 impl”不等于语言不提供这些类型的 Copy、调用、quote/splice 或清理等能力。相应能力可以由已有语言规则或内建协议提供；不能要求用户给每个 Closure 手写一个 impl 才能调用它。

本轮只收束你点名的类别及其通用边界；不据“非名义类型”或“含特殊字段”把其余类型全部列入禁止集合。

我对这里的理解是：一个对象属于类型系统，不代表用户能替它构造语言所要求的绑定、环境或代码项。因此，类型值的存在与用户实现资格要分别判断。“不好直接构造”解释了设计动机，但还不能独自充当机械判定算法；不能因为某个普通抽象类型暂时看不到构造方法，就把它也排除。

### G6. dyn 构造需要额外环境时，是否自动形成闭包限定结果？

两 usize 的表示要求已经确认；这里仅问已有 `value.dyn(Trait)` 怎样交付它确实需要的环境。

你的例子：`%{space: Space = space} dyn Drop`，把两个不同问题分开了：dyn 主体仍是固定表示，完整值另有 space 的环境要求。它没有承诺“整个 %C dyn Trait 永远也只有两个 usize”，更不能暗中丢掉 space。

- **A. dyn 构造依据所选实现的环境需要推导 `%C dyn Trait`；环境为空才消去限定。** `let value = object.dyn(Trait)` 可以自动保留环境；赋给裸 dyn 目标时不能悄悄擦除非空 C。
- **B. 有环境的实现必须由显式闭包限定目标或单独的明确构造接入，普通 dyn 构造只产出裸 dyn。** 裸接口更直接，但用户需要在这些构造点明确写出环境承载意图。

**预填答复：A。** 让类型推导保存已有依赖，避免要求用户反复手写同一个环境。跨边界时，完整类型及实际取得方式仍须满足目标；外层共享环境可以减少复制，不能缩短它所需的有效期。

两 usize 可用数据入口与分派入口实现，但这只是实现方案，不从“大小固定”额外推出两个字段的公开顺序或固定 vtable 内容。Self 返回、分配与 Drop 仍需原有契约：固定表示不授权隐式使用任意 allocator，也不使拥有型对象自动 Copy。

对 `%C dyn Trait`，也不能改成“指针指向一个随意扩大的隐藏盒子，所以一切环境都不必再表达”。需要的环境仍在闭包限定与实际交付中可追踪。

## 六、我的评估与文档衔接

这套设计最有价值的地方，是参数修饰主要规定**如何接入**，而非制造很多彼此不兼容的运行时函数类别。lambda 标注给语法糖一个局部边界；quote 保留符号；implicit 把供应交给可检查协议。最终生成的代码仍遵守同一套阶段、类型与资源规则。

两个需要认真守住的边界是：

- 可定制 elaboration 不能无声地改掉普通实参的执行次数和顺序。
- 两 usize 的 dyn 主体不能用来隐藏其必需环境；闭包限定可以接住这些环境，前提是构造和目标类型交付都保留它们。

catch 的 builtin.Continuation 按你当前的 intrinsic 定位记录，不把普通闭包的重复调用检查自动套成完整的 continuation 安全保证。open fn 的 package 封闭集合、case 排序及控制流再在函数级匹配一轮展开；本轮不重新追问 open code 的通用用途。

### 正式写入时需要同步的位置

| 文件或部分 | 本轮明确的调整 |
|---|---|
| grammar 的参数与 argument 分类 | lambda 只用锚定参数；默认值；四类显式命名供应；derive fn 参数 |
| 函数参数篇 | 参数规则地图、依赖次序、默认收集与 trait elaboration |
| CMTT／quote fn 的相关说明 | contextual 参数采用 implicit quote；template 主体接口 |
| traits/declarations、implementation-resolution | 用户 impl 的主体范围与泛型适用性边界 |
| traits/dynamic-interfaces | 裸 dyn 的两 usize 保证与闭包环境交付 |
| closure-type | 承接给出的环境绑定形态，保持环境与主体的分别说明 |

其中 dyn 大小、contextual 拼写及闭包参数类别均由你本轮直接纠正，不再当成待拍板选择。G1–G6 的精确展开和边界仍是本轮预填，等待修改。

## 七、确认状态与自检

- **已确认**：F1–F6 的保留选择；本次作者补充的各类参数、依赖顺序、template 主体、特殊类型 impl 限制和裸 dyn 表示方向。
- **本轮受托选择**：runtime 默认采用 P3 稳定观测并要求有限完成；跨 comptime 的适用范围单列 G4。
- **仍待确认**：G1–G6，不将候选展开与 ABI 交付细节提前写成正式定义。
- **待同步的文档差异**：dyn 表示、lambda 参数范围、runtime 默认参数语法；comptime 默认值限制另由 G4 明确。未擅自修改旧正文。
- **语法／风格自检**：对照现行 grammar/style 及你本轮确认的局部增量；默认形参、lambda 标注和 `.name = expr` 按新确认解释，tactic lambda 按作者给出的形式使用；G1 的纯 id 展开明确标为候选。代码不是编译器通过报告。
- **落盘边界**：056 追加本次 prompt 全文；057 保存新提案；prompt 只保留当前轮。

## 落盘与检查

- 056 追加作者原始 prompt 全文及校验值；057 完整保留新轮评估、六项候选、预填与最终回复。
- prompt 只保留当前轮。替换前逐字比较作者快照，避免覆盖工作期间新增的作答。
- 六个 Flurry 代码块已按 grammar/style 与本轮明确的局部增量检查；元组类型使用 Tuple，取址与解引用使用后缀形式，表达式语句按规定分隔。
- 默认值、MemoryEffect predicate 简写、tactic lambda 参数表和 elaboration 边界的确认状态分别标明。只做文档与示例自检，未声称通过编译器验证。
- 本轮不修改 docs 正式正文，不操作编译器文件，不创建提交。

## 本轮最终回复

已归档你的完整补充，并更新 [prompt](/home/fin/luna/prompt)，预填了六项交界问题。上轮 F1–F6 按已审阅收束。

参数系统按绑定形态、供应来源、阶段和语法角色分别整理。默认表达式我选择 **P3 稳定观测＋合法入口下有限完成**，允许读取前面参数的状态；与 comptime 默认值政策的边界单列审阅。

两处纠正已明确记录：contextual 隐参使用 `implicit quote`；裸 `dyn Trait` 固定两个 `usize`，额外环境由闭包限定表达。本轮只更新 prompt/history。


## 作者答复与写入批准（2026-09-27）

作者消息：“可以开始编写了。”当前 prompt 的六项预填均未修改，按此次批准收束；完整作答文件如下。

SHA-256：`21520c7eef70b3b2c4b83e15bd8d5cd6b84259744208daa0e8c4d1ecf7d53991`。

````markdown
# 函数与闭包：参数协议、上下文与表示边界

日期：2026-09-27；history：057；theme：functions-and-closures；第 02 轮。

**DRAFT — NOT CANONICAL。** 本轮归纳作者补充、承接上轮选择，并预填六个尚需确定的交界问题。只更新 prompt/history，尚不改写正式 docs。

## 一、上轮六项收束与本次纠正

上轮 F1–F6 的预填均保持不变；按已审阅收束为以下规则，并以本次补充限制它们的适用范围：

| 项目 | 承接结果 |
|---|---|
| F1 调用次序 | 先 callee／接收者，再按源码顺序求显式实参；各求值一次 |
| F2 可选参数 | 有默认表达式，可依赖前面的参数；只在缺省时求值。runtime 默认值的纯度在下文补足 |
| F3 捕获 | 默认按值：Copy 复制，非 Copy 移动；需要共享观察时显式捕获普通指针或视图 |
| F4 字段捕获 | 对能够证明独立交付的内建投影采用最小位置；保留真实依赖及剩余清理要求 |
| F5 捕获清理 | 去重后的首次捕获词法顺序形成语义字段，按逆序清理；物理布局不改可观察次序 |
| F6 普通 fn | 不隐式承接任意非空环境；需要环境时保留 `%C` 等实际交付 |

新增内容对先前记述有两项明确纠正，直接以你的补充为准：

1. quote fn 的 contextual 隐参使用 **implicit quote**。不再将先前的 contextual 属性当作现行方案。
2. **裸 dyn Trait 的大小固定为两个 usize。** 我先前在动态接口文档里写“语言不固定 dyn 必须是两个机器字”过度扩张了表示；所需环境由闭包限定表达，例如你给出的 `%{space: Space = space} dyn Drop`。

“闭包只有锚定参数”也明确收窄了现有 grammar 中 lambda 直接复用全部 parameter 的写法。后续修正文法时，要按构造种类检查参数，不能仅凭递归前缀产生式就接受所有组合。

## 二、参数系统的全貌

### 1. 分开记录四种信息

我建议实现和文档分别回答下面四件事：

- **绑定形态**：一个实参对应一个位置，还是收集一组位置实参／属性实参？
- **供应来源**：调用者显式给出、默认表达式补足，还是 elaboration 的 implicit 协议供应？
- **阶段与对象**：runtime 值、comptime 值，还是保留运行时符号的 quote 输入？
- **语法角色**：普通数据、lambda 表达式边界、case 模式、接收者、continuation 或 template 主体？

这是一种描述分工，**不表示任意修饰符都能任意组合**。每个声明位置仍按你确认的能力表检查。

### 2. 作者补充的规则地图

| 类别 | 已明确的含义与边界 |
|---|---|
| 锚定参数 | 一个 param 对应一个 arg；闭包的调用参数只有这一类 |
| 纯 id | lambda 中省略类型，由上下文等取得类型依据；typealias/newtype、子句及 normal form head 中的类型参数可缩写为 `id: Type`；MemoryEffect predicate 的特殊含义见 G1 |
| trait bound | 子句中的 `T :- TraitExpr` 脱糖为 `T, requires T :- TraitExpr`；不是另一种独立运行时实参 |
| implicit | elaboration 自动供应，可加 comptime，也可加 quote；允许 trait 提供 elaboration 协议 |
| implicit quote | 作为 contextual 变量，主要用于 quote fn；保留符号及绑定，不提前读取 runtime 内容 |
| lambda 标注 | 标明 post do 的目标，也为 tactic lambda 表达式建立解释边界 |
| comptime | 规定相应输入的阶段；不据修饰位置自行推导整个定义的执行类别 |
| optional | 可选供应，必须有默认表达式，可为 comptime，可依赖前面的参数 |
| 变参 | 默认将多个位置输入收集为元组，可为 comptime，可通过 trait 定制 elaboration |
| 拓展变参 | 默认将属性输入收集为匿名结构体，可为 comptime，可通过 trait 定制 elaboration |
| quote | 用于 comptime fn 和子句，指代 runtime 符号；允许 optional 形式；另有上面的 contextual implicit 用法 |
| case 参数 | open fn 的锚定参数模式，承载函数级模式匹配；open fn 的定义集合在 package 内封闭 |
| self／*self | 沿用已经确认的接收者及其资源语义 |
| catch | 绑定 continuation，目前固定 builtin.Continuation；实际操作接近 intrinsic，不能据此声称已经有很强的使用检查 |
| derive fn 参数 | 遵循 comptime fn 的参数规则，不因是派生器而独立设计一套参数机制 |
| template fn 主体 | 通过 quote 与 lambda 两个修饰标记绑定 body |
| template test 主体 | 通过唯一一个锚定子句参数绑定 body；不由“唯一锚定”推出禁止其他类别子句参数 |

参数依赖按你给出的方向组织：**子句参数 → 参数 → 返回值**，各组合内部按顺序依赖。参数可以依赖子句参数，返回值可以依赖两者；这个依赖检查次序与调用现场的 runtime 求值次序分别描述。

### 3. 显式命名供应不只针对 optional

你列出的四类——变参、拓展变参、implicit、optional——都可以按 `.name = expr` 直接供应。这里应把旧 grammar 的 optional_arg 在说明层面改成更一般的“显式命名实参”。

下面把三个尺寸收集成一个元组，也允许直接交付完整元组：

~~~flurry
fn volume(...shape: Tuple<u32, u32, u32>) -> u32 {
    let (width, height, depth) = shape;
    width * height * depth
}

test {
    let first = volume(2, 3, 4);
    let second = volume(.shape = (2, 3, 4));
    println("{} {}", first, second);
}
~~~

两个结果都是 24。第一个调用需要元组收集，第二个已经交付整个 shape。后一种不能再包装成“只有一个元组元素”的外层元组；直接供应也不执行缺省收集或 implicit 供应。

同一位置重复供应、或既直接交付完整包又向同一个包继续收集，应作为重复绑定处理，不悄悄拼接或覆盖。这是下面参数协议候选的唯一供应约束，不新增一种调用语法。

## 三、默认表达式的纯度选择

你让我选择层级，我选择：**runtime 可选参数的默认计算采用 P3 稳定观测，并要求在合法入口下有限完成。** 不新增第七个纯度等级；总性是已有的独立保证。

选择 P3 的理由是：默认值常常要观察前面参数的长度、配置字段或视图。只允许 P4/P5 会把这些常见操作排除；允许有界写入则会把省略实参变成隐含状态变更。

具体含义：

- 可以使用前面参数及合法、稳定的状态读取；每次调用观察本次状态，不跨版本盲目缓存。
- 默认计算及它所需的隐式适配、临时清理一起检查，不能把分配或输出藏到转换里。
- 不隐式修改调用者可观察状态，不输出、不交付新的可观察资源身份；需要这些行为时在调用处显式供应实参。
- 前提成立时有限完成；运行检查失败是调用准备的独立失败路径，不能把“默认表达式总性”当作删除前提检查的许可。
- Copy、move 和依赖交付仍按资源规则检查；纯度不允许同时把同一份非 Copy 责任交给两个参数。

~~~flurry
struct Limit {
    value: u32,
}

fn read_limit(source: *Limit, .limit: u32 = source.value.*) -> u32 {
    limit
}

test {
    let source = Limit { value: 1 }
    let first = read_limit(source.ref);
    source.value = 2;
    let second = read_limit(source.ref);
    let supplied = read_limit(source.ref, .limit = 9);
    println("{} {} {}", first, second, supplied);
}
~~~

输出为 `1 2 9`。最后一次不读取默认来源字段；前两次没有因为指针位值相同而复用旧的默认结果。前提及其实际需要的路径仍由契约检查。

此选择针对 runtime 默认计算；它与既有 **comptime 可选子句默认值可不纯** 的边界，需要在 G4 明确，不能悄悄同时改掉以前确认的规则。

## 四、对 elaboration 的理解

下面是候选的局部实现骨架，不是新增面向用户的编译阶段：

~~~text
从已定位定义的粗 HIR 签名读取参数角色
  → 识别显式供应、收集入口与缺省位置
  → 为 post do、tactic lambda、template body 确定接入边界
  → 按依赖关系完成阶段、类型及相应 trait 协议检查
  → 形成带来源及求值次序的参数交付计划
  → 在规定阶段执行表达式，并按真实控制流交付实参
~~~

关键是保留两个区别：

1. **elaboration 在编译时发生，不等于生成的实参计算也在编译时执行。** 候选接口按参数角色区分编译时数据与生成的 runtime 实参表达式；具体允许的结果由所选协议定义。
2. **建立 lambda 的表达式边界，不等于求值它的主体。** `_0 < _1` 是未来调用中的计算；不能先在外层调用点把它求成 bool 再尝试包装。

post do 的路由先读签名里的 lambda 标记；无标记时接到最后一个锚定参数。候选实现不凭参数类型“看起来像函数”随意寻找其他槽位。多个可行目标或目标已被供应时，应报告歧义／重复，不任意覆盖。

vararg 的 trait 协议可以改变如何形成那一个收集结果，但不能仅因发生在 elaboration 中就省掉生成程序的访问、责任和阶段检查。具体协议范围见 G3。

## 五、六项预填审阅

### G1. MemoryEffect predicate 的纯 id 是否省略了 quote 参数声明？

你特别指出纯 id 在 predicate 中主要服务于返回 MemoryEffect 的定义。结合之前的 modify，我最自然的解释是：

~~~flurry
predicate modify(target) -> MemoryEffect {
    read<target> and write<target>
}
~~~

候选展开为：

~~~flurry
predicate modify(quote target: lift Any) -> MemoryEffect {
    read<target> and write<target>
}
~~~

两段是同一定义的替代写法，不同时声明。

- **A. 在结果明确为 MemoryEffect 的 predicate 中，纯 id 是这个 quote 参数的简写。** 可直接组合内存区域规格，避免把 target 误当 Type 值；其他 predicate 不据此自动引用参数。
- **B. predicate 的纯 id 仍只表示 Type 参数，区域参数必须显式写 quote。** 规则更统一，但上面的第一种写法不能用于这个区域规格。

**预填答复：A。** 仅在这一明确的定义语境中采用简写。普通 predicate 默认返回 bool，并不因此获得隐式 quote。其余语境的纯 id 继续按相应参数种类解释，不建立“所有裸 id 都代表 Type”或“所有裸 id 都被 quote”的通法。

### G2. tactic lambda 的完整参数表由目标签名决定吗？

~~~flurry
fn compare_pair(first: u32, second: u32, lambda compare: fn(u32, u32) -> bool) -> bool {
    compare(first, second)
}

test {
    let ordered = compare_pair(2, 3, _0 < _1);
    let same_first = compare_pair(2, 3, _0 == _0);
    let explicit = compare_pair(2, 3) do |first, second| first < second;
    println("{} {} {}", ordered, same_first, explicit);
}
~~~

第二个表达式只出现 _0，但接收位置要求两个锚定参数。第三个展示已确认的 post do 路由：签名中的 lambda 标记定位 compare，显式 lambda 的主体仍等到调用时执行。

- **A. 参数数目和类型来自目标签名，占位符只引用相应参数。** 两个调用都合法；第二个相当于一个忽略第二参数的二元闭包。
- **B. 参数数目由表达式中的最大占位符下标决定，再与目标适配。** 第二个先成为一元闭包，需要另定补参数适配，或者拒绝它。

**预填答复：A。** 越界下标拒绝；同一下标重复出现引用同一个参数，并照常检查 Copy／仿射交付。不能把 `_0` 的重复出现变成重复计算外部实参。

候选作用域规则：在 lambda 标注位置建立边界；普通子表达式继承它。嵌套的另一个 lambda 标注位置建立新边界，显式 lambda 也形成独立边界。占位符不隐式跨这些边界引用外层参数；需要跨层捕获时改写成具名的普通 lambda。

未标记的参数位置，仍可显式传入 `|first, second| first < second`，但不能在那里单独启用 tactic lambda。post do 的缺省路由本身也不把未标记参数永久变成 tactic lambda 入口。

### G3. 参数 trait 的 elaboration 能否改写其他实参或外层调用？

这里有一个真正影响协议可组合性的边界：收集协议收到若干源输入后，是交付自己参数的位置，还是获得整个调用的控制权？

- **A. 协议只负责其参数的供应或收集，并交付可检查的结果；外层仍负责统一绑定与求值次序。** 不得隐式重排、重复或吞掉其他实参；需要延期代码时使用明确的 quote／lambda 角色。
- **B. 协议可接管整个调用并重新组织其他参数与控制流。** 元编程能力更大，但多个协议并存时还需规定优先权、重写冲突与调用语义由谁拥有。

**预填答复：A。** 先把协议限制为可组合的参数入口。显式 `.name = expr` 直接交付完整值时，跳过缺省供应／收集，仍核验适配与契约。原始表达式可用于编译时类型分析；其 runtime 效果仍按调用计划各执行一次。

具体候选执行计划：先按 F1 求显式 runtime 实参；随后按参数声明依赖顺序补齐缺失的 implicit 和 optional 位置。comptime 求值及符号绑定按自己的阶段完成，不把它们伪装成 runtime 指令。若某协议确实需要打破这个计划，应通过显式的语言入口另行说明，不能借 trait hook 暗改普通调用。

这里不指定协议的 trait 名、关联项名或私有 IR API；那些应在协议接口专节里按用途设计。

### G4. 默认值的纯度限制是否只适用于 runtime 默认计算？

本轮已选择 runtime 默认采用 P3 加有限完成；另一个现行定义是：

[comptime/purity-and-normal-form.md](/home/fin/luna/docs/comptime/purity-and-normal-form.md) 明写：

> 可选子句参数的默认值在 normalization 中直接执行，不要求该表达式纯。需要延迟时由 defer 包裹。

这与“所有可选参数默认都受同一纯度限制”不能同时直接沿用。

- **A. 这次 P3 要求约束 runtime 默认计算；comptime 与子句默认继续按既有阶段求值政策处理。** 保留已经确认的 normalization／defer 规则，不因参数表面相近就合并两个阶段。
- **B. 所有可选默认值都需具备对应阶段的受限纯度，同时修订原 comptime 定义。** 跨阶段更统一，但编译时默认值中的诊断、环境读取等能力需要按新界限重新核对。

**预填答复：A。** 不额外把 runtime 的完成性和内存状态等级强加到编译时求值上。comptime 可选输入仍须在其使用阶段取得合法值；需要的计算未完成，不能用“缺省”代替结果。

这一题只确认规则覆盖范围，不重新让你选择我受托决定的 runtime 纯度层级。

### G5. 不允许用户 impl 的类型，是否也排除泛型实现的间接覆盖？

你已经明确 reachability type、CMTT 类型、闭包类型等不能直接按普通类型使用 impl。还需要避免下面的泛型实现不经意绕过这条边界：

~~~flurry
trait Marker {}

impl Marker for T
where T
{}
~~~

- **A. 限制针对最终主体类型，直接 impl 与泛型匹配都受它约束。** 这个实现只覆盖允许用户实现的主体；泛型代码也不能因此无条件断言所有 T 都有 Marker。
- **B. 只禁止对这些类型写直接实现头，允许符合条件的泛型实现覆盖它们。** 仍能写通用 Marker 一类接口，但“不能 impl”实际只限制一种声明形式。

**预填答复：A。** 在候选适用性中检查主体是否允许用户实现；typealias 不隐藏真实主体。合法的名义包装仍按自己的主体和资源契约检查，不因此把内部语言对象变成普通可构造值。

“不能用户 impl”不等于语言不提供这些类型的 Copy、调用、quote/splice 或清理等能力。相应能力可以由已有语言规则或内建协议提供；不能要求用户给每个 Closure 手写一个 impl 才能调用它。

本轮只收束你点名的类别及其通用边界；不据“非名义类型”或“含特殊字段”把其余类型全部列入禁止集合。

我对这里的理解是：一个对象属于类型系统，不代表用户能替它构造语言所要求的绑定、环境或代码项。因此，类型值的存在与用户实现资格要分别判断。“不好直接构造”解释了设计动机，但还不能独自充当机械判定算法；不能因为某个普通抽象类型暂时看不到构造方法，就把它也排除。

### G6. dyn 构造需要额外环境时，是否自动形成闭包限定结果？

两 usize 的表示要求已经确认；这里仅问已有 `value.dyn(Trait)` 怎样交付它确实需要的环境。

你的例子：`%{space: Space = space} dyn Drop`，把两个不同问题分开了：dyn 主体仍是固定表示，完整值另有 space 的环境要求。它没有承诺“整个 %C dyn Trait 永远也只有两个 usize”，更不能暗中丢掉 space。

- **A. dyn 构造依据所选实现的环境需要推导 `%C dyn Trait`；环境为空才消去限定。** `let value = object.dyn(Trait)` 可以自动保留环境；赋给裸 dyn 目标时不能悄悄擦除非空 C。
- **B. 有环境的实现必须由显式闭包限定目标或单独的明确构造接入，普通 dyn 构造只产出裸 dyn。** 裸接口更直接，但用户需要在这些构造点明确写出环境承载意图。

**预填答复：A。** 让类型推导保存已有依赖，避免要求用户反复手写同一个环境。跨边界时，完整类型及实际取得方式仍须满足目标；外层共享环境可以减少复制，不能缩短它所需的有效期。

两 usize 可用数据入口与分派入口实现，但这只是实现方案，不从“大小固定”额外推出两个字段的公开顺序或固定 vtable 内容。Self 返回、分配与 Drop 仍需原有契约：固定表示不授权隐式使用任意 allocator，也不使拥有型对象自动 Copy。

对 `%C dyn Trait`，也不能改成“指针指向一个随意扩大的隐藏盒子，所以一切环境都不必再表达”。需要的环境仍在闭包限定与实际交付中可追踪。

## 六、我的评估与文档衔接

这套设计最有价值的地方，是参数修饰主要规定**如何接入**，而非制造很多彼此不兼容的运行时函数类别。lambda 标注给语法糖一个局部边界；quote 保留符号；implicit 把供应交给可检查协议。最终生成的代码仍遵守同一套阶段、类型与资源规则。

两个需要认真守住的边界是：

- 可定制 elaboration 不能无声地改掉普通实参的执行次数和顺序。
- 两 usize 的 dyn 主体不能用来隐藏其必需环境；闭包限定可以接住这些环境，前提是构造和目标类型交付都保留它们。

catch 的 builtin.Continuation 按你当前的 intrinsic 定位记录，不把普通闭包的重复调用检查自动套成完整的 continuation 安全保证。open fn 的 package 封闭集合、case 排序及控制流再在函数级匹配一轮展开；本轮不重新追问 open code 的通用用途。

### 正式写入时需要同步的位置

| 文件或部分 | 本轮明确的调整 |
|---|---|
| grammar 的参数与 argument 分类 | lambda 只用锚定参数；默认值；四类显式命名供应；derive fn 参数 |
| 函数参数篇 | 参数规则地图、依赖次序、默认收集与 trait elaboration |
| CMTT／quote fn 的相关说明 | contextual 参数采用 implicit quote；template 主体接口 |
| traits/declarations、implementation-resolution | 用户 impl 的主体范围与泛型适用性边界 |
| traits/dynamic-interfaces | 裸 dyn 的两 usize 保证与闭包环境交付 |
| closure-type | 承接给出的环境绑定形态，保持环境与主体的分别说明 |

其中 dyn 大小、contextual 拼写及闭包参数类别均由你本轮直接纠正，不再当成待拍板选择。G1–G6 的精确展开和边界仍是本轮预填，等待修改。

## 七、确认状态与自检

- **已确认**：F1–F6 的保留选择；本次作者补充的各类参数、依赖顺序、template 主体、特殊类型 impl 限制和裸 dyn 表示方向。
- **本轮受托选择**：runtime 默认采用 P3 稳定观测并要求有限完成；跨 comptime 的适用范围单列 G4。
- **仍待确认**：G1–G6，不将候选展开与 ABI 交付细节提前写成正式定义。
- **待同步的文档差异**：dyn 表示、lambda 参数范围、runtime 默认参数语法；comptime 默认值限制另由 G4 明确。未擅自修改旧正文。
- **语法／风格自检**：对照现行 grammar/style 及你本轮确认的局部增量；默认形参、lambda 标注和 `.name = expr` 按新确认解释，tactic lambda 按作者给出的形式使用；G1 的纯 id 展开明确标为候选。代码不是编译器通过报告。
- **落盘边界**：056 追加本次 prompt 全文；057 保存新提案；prompt 只保留当前轮。

````
