# CMTT：带上下文的代码生成

## 当前实现边界

当前 luna 已接通局部 `const` 表达式代码的解析、类型检查、卫生接入、IIR 实例化与解释执行。引用保留 runtime 绑定身份；可以推导上下文，也可以用 `< Ψ |- T >` 声明虚拟接口。默认对应、构造时暴露端口、接入时同时替换、代码别名、编译时条件选择和单洞组合均进入实际源码测试。

~~~flurry
fn selected(value: bool) -> bool {
    const original: < input: bool |- bool > = '(input);
    const combined = '(not ~[input = value]<original>);
    let value = false;
    not ~<combined>
}
~~~

`selected(true)` 返回 `true`：虚拟 `input` 在组合时接到原形参，后来的同名局部不改变这条绑定关系。代码创建本身不读取或消费形参。每次接入分配独立的内部局部身份和展开位置，生成程序继续接受原有的资源、清理、效应及阶段检查。

显式 `pure comptime fn` 已能接收完成的数据及具有具体接口的表达式代码参数，组合并返回表达式代码。数据支持标量、Type、Tuple、匿名记录与 struct/enum 值；保留完整名义类型身份，Tuple 使用绑定的 normal-form 头。生成器内支持不可变局部、直接返回和条件表达式；数据参数通过既有 comptime invocation 求值并冻结，代码参数保留上下文见证。

~~~flurry
pure comptime fn invert(code: < x: bool |- bool >) -> < x: bool |- bool > {
    '(not ~<code>)
}

fn twice(value: bool) -> bool {
    const original: < x: bool |- bool > = '(x);
    const generated = invert(invert(original));
    ~[x = value]<generated>
}
~~~

`twice(true)` 返回 `true`。生成器在编译时完成，生成的 runtime 程序仍接受移动、清理及效应检查。未调用的生成器也检查定义，包括未选中的分支；递归构造受预算约束。已有代码可以重新标注兼容接口，改名保留原绑定对应；固定依赖不能借重新标注变成自由端口。

生成器的普通参数支持具名可选入口及默认表达式，数据与表达式代码均可作为默认值。显式实参按源码顺序计算一次；随后按参数顺序计算缺省项，默认值可使用前序参数，不能读取自身或后序参数。覆盖默认值会跳过其执行，定义检查仍核验所有默认值的类型与阶段。这里执行的是 comptime 默认计算，遵循 comptime invocation 政策，不套用 runtime 默认值的 P3 限制。

~~~flurry
pure comptime fn select(
    first: < x: T |- T >,
    .second: < x: T |- T > = first,
) -> < x: T |- T > where T {
    second
}

fn selected(value: bool) -> bool {
    const generated = select('[input = value](value));
    ~<generated>
}
~~~

默认代码沿用前序代码的绑定对应，不复制其 runtime 资源。默认 quote 可以使用自身接口声明的虚拟名字，这些名字只在该默认表达式中可见。省略具有发散默认计算的参数仍会耗尽编译时预算；显式覆盖该参数不运行被跳过的默认计算。数据变参可按普通位置收集规则组成元组，支持空包、异构包和 `.name = packet` 整包供应；有绑定的 Tuple 构造器时保留其 normal-form 身份。收集包仍是一个数据参数，其声明类型约束整个包。位置元素与具名代码实参交错时按原源码顺序计算，再完成收集。元组字面量实参可使用 `...(a, b)` 展开：元素直接进入原参数检查及收集路径，保留目标类型和交错求值顺序。普通元组表达式也可展开：参数包在原源码位置完成一次 comptime invocation，元素冻结后按顺序供应，空包仍执行原表达式的检查。包宽度由已有元组接口确定；只有一个未知宽度且目标位置数量唯一时，也可由目标布局确定宽度。多个未知宽度不能任意分配，开放变参没有唯一宽度时需要明确包接口。代码容器及自动供应协议仍未接通。

~~~flurry
pure comptime fn pack(...items: T) -> < |- T > where T { '(items) }

fn packed() -> bool {
    const code = pack(true, 'a');
    let (enabled, letter) = ~<code>;
    enabled and letter == 'a'
}
~~~

普通包生产者同样保留收集语义：

~~~flurry
pure comptime fn packet() -> (bool, char) { (true, 'a') }

fn expanded() -> bool {
    const code = pack(...packet(), false);
    let (enabled, letter, extra) = ~<code>;
    enabled and letter == 'a' and not extra
}
~~~

`packet()` 只在编译时求值一次；结果元素与额外的 false 一起交给 pack。包中的数据不能冒充 quoted code 证据，runtime 包生产者也不能因出现 `...` 而提前执行。

`pack(.items = (true, 'a'))` 供应相同的完整包，不会多包一层元组。收集类型不能仅写成某个元素类型，也不能把收集后的元组解释成 Type 值。

扩展调用中的 `name: value` 属性已能收集到生成器的数据变参，组成匿名记录。它与 `.name = value` 的参数整包供应不同：属性名属于记录字段，而不是生成器的形参名。

~~~flurry
pure comptime fn configure(...properties: T) -> < |- T > where T {
    '(properties)
}

fn configured() -> bool {
    const code = configure { enabled: true, letter: 'a' };
    let config = ~<code>;
    config.enabled and config.letter == 'a'
}
~~~

属性可与普通位置实参、元组展开和代码实参交错，各项仍按源码顺序计算一次；收集不会把分散的属性提前到同一个求值位置。记录字段接受声明 schema 提供的目标类型，显式 Type 实参和目标代码接口也可帮助确定收集入口。当前需要唯一的合格记录收集入口，重复属性或多个不能消歧的入口会报错。生成器属性调用仍限于编译时代码构造位置，不能通过扩展调用语法在 runtime 执行。

生成器还支持 Type 泛型子句、trait／类型命题约束和具名 Type 默认值。类型参数可从数据类型、输入代码的结果与上下文类型、目标代码接口推导，也可以显式指定。实例化同时替换输入接口、输出接口与正文的类型绑定；嵌套生成调用各自保存类型环境，normal-form 头不展开。

~~~flurry
pure comptime fn forward(code: < x: T |- T >) -> < x: T |- T > where T {
    code
}

pure comptime fn compose(code: < x: T |- T >) -> < x: T |- T > where T {
    forward(code)
}

fn original(value: bool) -> bool {
    const generated = compose('[input = value](value));
    let value = false;
    ~<generated>
}
~~~

`original(true)` 返回 true。类型推导不授予新的绑定对应：显式暴露的端口可以适配接口名，固定依赖仍不能被转换成任意可替换输入。Type 子句默认值沿用普通函数规则，不被调用推导覆盖；所有类型约束先核验，才运行本次生成器。目标接口可以为 quoted 数值字面量供应类型。已保存代码值和数据绑定的类型也会在实参计算前参与推导；前序实参完成后，其类型信息继续传给后续代码参数和变参元素。这里只提前读取已有类型证据，不提前执行生成表达式或读取 runtime 数据，也不改变原有上下文对应。未解出的类型不会进入生成程序；需要后序任意计算结果才能确定的类型，仍可能要求显式类型实参或目标接口。上述支持尚不包括上下文多态或 `implicit quote` 的自动供应。

结构化配置可以选择生成结果，也可以在 quote 中提升已完成的字段数据：

~~~flurry
struct Options { invert: bool }

pure comptime fn configured(options: Options) -> < x: bool |- bool > {
    let {invert} = options;
    if invert { '(not x) } else { '(x) }
}

fn negate(value: bool) -> bool {
    const code = configured(Options { invert: true });
    ~[x = value]<code>
}
~~~

数据局部可以使用不可反驳的 let 模式直接解构，也可以使用普通 match 检查 enum 载荷。let 解构复用普通模式规则，包含嵌套结构、析取绑定重映射及顺序模式计算；未调用的生成器也检查字段、类型、覆盖和阶段。数据计算不能提前读取 quote 的 runtime 上下文或调用 runtime 函数。把 runtime 函数身份交给 comptime 辅助函数本身不等于调用该函数；最终提升的数据仍须通过不可变值冻结。

生成器的 `match` 既可选择表达式代码，也可驱动构造程序的控制流。主语只求值一次，模式和守卫复用普通匹配器；只有成功分支的绑定进入该分支，未选择的分支仍接受定义检查。

~~~flurry
enum Step { skip, wrap(bool), stop }

pure comptime fn generated(first: Step) -> < x: bool |- bool > {
    var step = first;
    var code: < x: bool |- bool > = '(x);
    while true {
        step match {
            .skip => { step = .wrap(true); continue; },
            .wrap(invert) => {
                const previous = code;
                code = if invert { '(not ~<previous>) } else { previous };
                step = .stop;
            },
            .stop => { return code; },
        };
    }
}
~~~

`generated(.skip)` 生成取反，`generated(.stop)` 保留原代码。分支中的 `return`、`break`、`continue` 作用于构造程序，quote 内部的控制流仍作用于生成后的 runtime 程序。尾部 match 的所有有限路径均返回代码时，不需要额外尾表达式；存在漏返回、类型不兼容、覆盖不足或阶段非法的分支则拒绝。各分支可以解构不同类型的载荷，匹配器传递绑定所用的内部类型不发布到 runtime 产物。

生成器的数据与代码局部支持 `var` 和整值赋值，可以逐步包裹已有代码。代码别名保存原值，赋值不会改动已经保存的别名：

~~~flurry
pure comptime fn twice() -> < x: bool |- bool > {
    var code: < x: bool |- bool > = '(x);
    const original = code;
    code = '(not ~<original>);
    code = '(not ~<code>);
    code
}
~~~

赋值维持变量原有的结果类型和上下文接口；兼容代码的接口改名可以适配，默认对应随被赋入的代码值传递。推导出的固定依赖只能接到原有对应，不能凭赋值新增或任意替换 runtime 输入；对应有歧义时需要显式端口。嵌套构造块内对外层变量的更新保留，新声明的局部不泄漏。

`while` 已能驱动代码累积，支持语句 if、嵌套循环、带标签的 break/continue、循环内提前返回和数据局部复合赋值：

~~~flurry
pure comptime fn invert_times(count: u8) -> < x: bool |- bool > {
    var remaining = count;
    var code: < x: bool |- bool > = '(x);
    while remaining > 0 {
        const previous = code;
        code = '(not ~<previous>);
        remaining -= 1;
    }
    code
}
~~~

零次迭代保留初始代码。未调用的生成器只检查循环体，不运行循环；调用时，迭代、环境快照和代码复制共同消耗构造预算，超限不发布部分结果。quote 内的 runtime 循环与生成器循环保持不同控制目标。

控制流检查区分继续、返回、各个循环目标的 break/continue，以及没有有限出口的路径。所有能结束的路径都返回代码时，可以省略尾表达式，例如 `while true { return '(true); }`；可能跳出循环却不返回代码则拒绝。不会结束的定义不是漏返回，但实际调用仍会受构造预算限制。检查不运行生成器来证明终止，只对布尔字面量及 not/and/or 作有限的控制流判断；未取得依据的条件保留两条路径，不会跳过不可达代码的类型与阶段检查。`for` 协议驱动的生成尚未接通。

生成器也支持 `let-else`／`var-else`，包括模式守卫。成功匹配的绑定进入后续构造作用域，失败候选的绑定不进入 else；else 必须返回、跳转循环或不终止，不能继续到绑定使用处。`var` 的成功绑定可以重新赋值。

~~~flurry
enum Input { none, some(bool) }

pure comptime fn configured(input: Input) -> < x: bool |- bool > {
    let .some(enabled) = input else { return '(x); };
    '(x and enabled)
}
~~~

`.none` 生成原输入，`.some(false)` 生成与 false 的合取。即使从未调用，定义中的 else 也检查阶段与结果类型；不会执行其中的无限循环。实际调用若进入不终止的 else，仍按构造预算失败。此处只解构完成的数据，不把代码见证伪装成普通可匹配的数据对象。

完成的数据支持 struct／匿名记录的嵌套字段赋值与复合赋值，以及 BitVec 字段视图的普通写回。更新复用普通位置检查，在独立 comptime 调用帧中执行，成功后再保存完成的数据；不会修改已创建 quote 保存的数据快照。`let` 的字段写入沿用[绑定规则](../resource-management/bindings.md)，不把绑定不可重赋值误解为对象只读。写入 runtime 上下文或通过显式指针写入仍拒绝。带索引的复合赋值尚未接通，不能用重复求值索引的改写冒充一次求值语义。

Type 数据留在 comptime，可以参与类型比较；不能将 Type 或包含它的对象交付到 runtime 接口。数据冻结不接受指针、可调用身份或活动存储，即使它们包在 nominal 对象里；也不会为接纳数据而展开用户 normal-form 头。可反驳 let 模式必须提供发散的 else 分支。

quote 函数的已检查正文独立保存在 NIR 的 `quote_templates` 中，dump 使用 `(quote-template ...)` 包装。模板保留泛型、契约、默认值和来源，不是 runtime 入口，也不能转换成普通函数指针。模板内部尚待展开的直接应用可以保留；可执行函数中的直接 quote 应用则必须完成展开。NIR 阶段检查同时检查模板正文，拒绝身份冲突及阶段错误。

具体的普通参数 `quote fn` 已接通源码展开。普通实参按调用顺序各求值一次，未使用的实参仍执行；主体与形参建立新鲜身份，应用没有对应的 runtime 函数调用。逻辑返回边界使用私有结果枚举及循环出口，保留闭包自身的 return；未返回资源在应用内清理，返回资源交给调用者。

~~~flurry
quote fn choose(value: bool) -> bool {
    while true {
        if value { return true; }
        break;
    }
    false
}

fn inverted(value: bool) -> bool {
    not choose(value)
}
~~~

`choose` 内的 return 只完成这次应用，因此 `inverted(true)` 仍返回 false。每份 quote／quote fn 主体独立检查单洞：一个显式 splice 与一个 quote fn 应用不能同时出现；普通 fn 的主体可以包含多次应用。生成器可以返回含 quote fn 应用的表达式代码。递归展开消耗预算，失败不退回 runtime 调用。作为 runtime 函数指针逃逸会拒绝。

默认参数复用普通调用路由：先按源码顺序求值所有显式实参，再在定义的形参环境中依次补齐缺省项。默认表达式读取的是已取得的绑定，不重新求值实参，也不会被调用者的同名局部捕获。对前序资源对象直接读取 Copy 字段，不要求整个对象是 Copy；对象可用性仍由移动检查保证，带构造或调用的取字段表达式不会因结果是 Copy 就自动取得稳定性证据。

~~~flurry
fn opposite(value: bool) -> bool { not value }

quote fn choose(first: bool,
    .second: bool = opposite(first),
    .third: bool = opposite(second)) -> bool {
    second and not third
}

fn selected() -> bool {
    choose(true, .second = true)
}
~~~

这里仅运行一次 `opposite`，用于 third。默认计算仍须满足稳定观测和有限完成的要求；这些证据在 quote 模板退出可执行函数表前检查，展开不会放宽默认值规则。内部返回包具有编译器来源标记，只用于证明结构化清理没有额外用户行为，不为其中的资源授予 Copy。

Type 泛型复用普通调用的推导与实例化，支持 `where T`、`where T :- Trait` 和 requires 类型命题。实例化先替换模板中的类型、默认表达式和普通调用信息，再卫生展开；不会展开 normal form 的头。泛型调用者中的内部返回存储保留同一组类型参数，随调用者实例化。

~~~flurry
quote fn identity(value: T) -> T where T { value }

pure fn forward(value: T) -> T where T { identity(value) }

fn checked() -> bool {
    forward(true) and identity('a') == 'a'
}
~~~

trait 约束与类型等式要求必须在展开前取得证据；不能以删除函数调用绕过它们。显式 `identity<bool>(true)` 也计作 quote 主体的一个接入点。内部返回包的空包覆盖位置和已移出载荷的清理状态有独立证据，因此泛型转发不要求凭空证明任意 T 可销毁，也不会授予 T 额外的复制能力。

Type 子句支持具名默认值；缺省时采用声明默认值，不以调用推导替换它，显式具名实参可覆盖并继续检查约束。默认值可引用前序类型参数。

~~~flurry
quote fn keep(value: T) -> T where .T: Type = bool { value }

fn defaults() -> bool {
    keep(true) and keep<.T = char>('a') == 'a'
}
~~~

固有 `quote` 方法可声明在 struct／enum 内部或固有 impl 中。接收者先求值一次，再按普通参数规则绑定；`*self` 保留指向原位置的引用，展开不会复制被引用的对象。容器和方法自身的类型参数均参与实例化；具名可选实参可以观察已绑定的接收者。方法调用与普通 quote 函数调用一样计入单洞限制，不能通过方法语法绕过 runtime 阶段要求。

~~~flurry
struct Flag { value: bool }

impl Flag {
    quote fn flip(*self) { self.value.* = not self.value.*; }
    quote fn read(self, .answer: bool = self.value) -> bool { answer }
}

fn changed() -> bool {
    var flag = Flag { value: false };
    flag.flip();
    flag.read()
}
~~~

展开保留所有普通实参的求值和资源义务：未使用的资源实参也必须求值并清理，按值返回的接收者把资源义务交给调用者，不产生额外复制。运行产物中没有这些 quote 方法的函数入口。

变参 quote 函数复用普通调用的收集规则，整包类型仍约束整个参数。位置展开和属性收集在展开主体之前建立参数帧，普通实参按原源码顺序各求值一次，空包生产者也执行；默认值随后使用已绑定的完整包。

~~~flurry
quote fn pack(...items: T) -> T where T { return items; }

fn configured() -> bool {
    let (enabled, letter) = pack(...(true, 'a'));
    let config = pack { enabled: enabled, letter: letter };
    config.enabled and config.letter == 'a'
}
~~~

这里两次 pack 都确定性展开，返回的元组／记录正常参与移动与清理；属性不是具名形参。固有方法的接收者先于包和属性求值，展开后仍保留独立的 return 边界。

普通 `lambda` 参数也复用调用路由，支持占位符闭包与尾随 `do`；普通 `implicit` 参数支持显式具名供应。先构造一次闭包值，再展开 quote 主体，捕获发生在它原来的实参位置。属性生产者先于尾随 `do` 执行时，闭包捕获其执行后的状态。闭包自己的 return、参数位置身份和仿射资源义务保持不变。

~~~flurry
quote fn apply(value: T, implicit enabled: bool, lambda action: fn(T) -> T) -> T
    where T {
    if enabled { return action(value); }
    value
}

fn transformed() -> bool {
    apply(false, .enabled = true) do { not _0 }
}
~~~

这里的 `action` 是普通 runtime 闭包参数，`enabled` 是按值传入的普通 implicit 参数。两者都不授予 contextual 绑定对应。缺少 implicit 供应时，当前仍报告自动供应协议尚未实现；多个 lambda 槽导致尾随正文目标不唯一时拒绝。trait 接口中的参数标记同样在选择实现前完成路由，泛型调用者晚期展开不改变其规则。

当前支持普通参数及上述参数协议、位置与属性变参收集、元组包展开、具名可选参数、默认表达式、Type 泛型子句及默认值、类型命题约束和固有 quote 方法。contextual 参数、值泛型子句及 arm/template 汇合仍未接通。trait 中的 quote 方法声明、默认正文及 quote 实现已能检查与展开，方法契约及 S-expression dump 保留 quote 类别；普通函数实现不能满足它的确定性展开要求。将 quote 实现用于普通 trait 方法所需的 callable 适配仍未接通。其它无主体 quote 声明可进入 HIR，但不能据此生成可执行函数；quote 与 comptime 组合仍拒绝。

`quote fn` 的显式 `comptime` 参数沿用普通分阶段调用规则。输入先完成并冻结，Type 输入可以决定后续参数与结果的类型；展开后的普通实参仍按源码顺序求值一次。comptime 形参不会成为 runtime 参数槽，quote 函数本身仍属于 runtime 代码类别。

~~~flurry
quote fn identity(comptime T: Type, value: T) -> T { value }

comptime fn opposite(value: bool) -> bool { not value }

quote fn choose(comptime flag: bool, value: bool,
    comptime .enabled: bool = opposite(flag)) -> bool {
    if enabled { return value; }
    false
}

fn checked() -> bool {
    identity(bool, true) and choose(false, true)
}
~~~

调用可以通过另一函数的 comptime 参数转发尚未取得的输入。NIR 保留 checked template 与阶段明确的调用；待调用者实例化后，共享的 staged binder 完成输入、默认值与依赖类型，重新检查替换后的类型命题，再卫生展开主体。trait 方法先选择实现并重绑定接口默认形参，随后采用同一路径。IIR 中不保留 quote runtime 入口。

输入不得读取未来的 runtime 参数或调用 runtime 函数。支持的数据域和冻结限制与普通 staged 输入一致；这里不额外开放活动指针或任意闭包图。递归展开受预算限制，耗尽时拒绝，不回退成运行时调用。效应结果沿用下面的 computation 边界，不因接通 comptime 参数而抹去效应行。

`quote fn` 已支持现有可检查程序谓词形式的 `requires`、`ensures` 和参数 `old`。它们属于程序验证层：调用处证明前置条件，每个正常返回路径证明后置条件。展开不会把证明义务变成隐式 `assumes`，也不会通过 PTC 求解程序状态。

~~~flurry
quote fn flip(value: bool) -> result: bool
    where requires value, ensures result == not value'old {
    value = not value;
    return value;
}

fn checked_flip() -> bool { not flip(true) }
~~~

入口快照在实参绑定后建立，仍要求相应类型具有 Copy 权限。后置条件检查放在每条返回路径的值完成之后、离开该逻辑作用域之前；尾表达式也经过同样处理。嵌套闭包的 return 属于闭包自身。结果只求值和交付一次，谓词观察不会消费仿射结果。显式 `assumes` 检查失败时，活跃资源仍沿原清理规则释放。

trait 接口契约与所选实现契约同时保留，形参、结果及旧值快照按身份重绑定；实现使用不同名字或放宽前置条件，均不能抹掉接口要求。尚未调用的模板也要验证其声明的后置条件。带 comptime 参数的模板在绑定输入后沿用同一契约展开路径。

内部 IR 以 `(snapshot ...)`、`(require-predicate ...)` 和 `(ensure-predicate ...)` 区分快照、调用义务与返回义务；它们有运行时检查语义，但程序验证不能将后两者当作凭空获得的事实。实际内存规格仍受现有程序验证器的可表达和可证明范围限制。

效应结果已能随 `quote fn` 展开，包含带 comptime 输入和 trait 实现选择的应用。实参准备留在调用者的效应环境中，主体进入自己声明的 computation 环境；内部返回汇合只保存完成值，效应行及出现位置不进入普通返回存储。

~~~flurry
effect read() -> bool;

quote fn subject(value: bool) -> #read bool {
    read()#;
    return value;
}

fn resumed() -> bool {
    subject(true)# { read() => resume false, }
}
~~~

这里 `resume false` 只交付 read 的结果，subject 继续执行并返回 true。调用者安装的 handler 若执行 return，仍返回它所属的调用者；quote 主体内部安装的 handler 执行 return，则只完成该 quote 应用。

现有 direct／direct-passing 执行路径保留安装、求值和请求顺序。direct-passing provider 先于受处理表达式的实参求值，重复请求不会重复安装；仿射槽位的请求次数检查仍有效，不能通过展开变成可复制资源。缺少匹配的效应环境和 comptime 中的 runtime 应用仍拒绝。

效应限定结果中的命名绑定现在指向 computation 正常完成后交付的值。例如 `-> result: #poll bool` 的 `ensures` 中，result 是 bool；它不表示效应环境。普通 fn、quote fn、staged 参数和 trait 方法共用这一规则。返回值的标量／聚合事实可以传给调用者，但 handler 可修改的外部存储事实必须失效；handler 提前返回需要证明它所退出的调用者契约，不能借用尚未完成的 callee 保证。

这次接线保留现有效应执行能力的边界：带资源清理调度的效应解释执行仍需要后续工作。它也不代表 inline effect 所要求的 callee 与 handler 联合展开已全部完成。

trait quote 方法在取得实现选择证据后展开。具体调用可在 NIR 阶段完成；普通泛型调用者先保留接口调用，待 IIR 实例化完成类型实参后选择正文并展开。接口默认实参仍引用接口的形参帧，展开时按参数位置对应到所选实现的新鲜绑定，不因实现改用不同形参名而改变默认值。

~~~flurry
trait Read { quote fn read(self) -> bool; }
struct Flag { value: bool }
impl Read for Flag {
    quote fn read(self) -> bool { return self.value; }
}

fn read(value: T) -> bool where T :- Read { value.as(Read).read() }
fn run() -> bool { read(Flag { value: true }) }
~~~

这里普通函数 read 仍可保留调用，trait 方法 read 必须展开。默认 trait 正文、泛型 impl、方法 Type 参数及引用接收者使用同一选择路径。递归展开仍受深度与工作预算约束；选择失败或缺少代码模板时拒绝，不回退 runtime 分派。延后生成的返回存储及来源进入当前实例图，清理规划使用对应布局，析构依赖扩展实例图时重新计算相关清理计划。

这尚不是完整 CMTT 实现。代码值目前保存在 elaboration 的代码环境中；值子句／方法／非纯生成器、异构代码容器与 `Any`/`any` 证据、上下文多态、依赖 runtime identity 的类型替换、pattern/arm 引用与 template 接入尚未接通，均不能据此宣称可用。普通局部 `const` 已支持可冻结数据、Type 和不可反驳的聚合解构；带 guard／else 的 runtime 局部 const、外部活跃指针及循环闭包图尚未接通；普通局部常量已可保存函数声明身份及拥有的闭包环境快照，并保留图内共享闭包身份。以下章节描述完整语言契约，不能为了适配这个实现子集而缩减。

## Intro

代码生成不仅要保存一段程序，还要知道它使用哪些绑定、产生什么结果。CMTT 把这些信息放在代码的接口中，使生成器可以组合尚未获得真实运行时输入的程序。

表达式代码使用上下文，模式代码在匹配成功后产生上下文，arm 代码把模式与使用其绑定的主体一起保存。

## 语法

~~~text
contextual_expr_type -> < runtime_binding* |- expr >
contextual_pattern_type -> < expr -| runtime_binding* >
quote_expr -> '(expr) | '[ substitution* ] (expr)
quote_pattern -> '<pattern> | '[ substitution* ] <pattern>
splice_expr -> ~<expr> | ~[ substitution* ] <expr>
splice_pattern -> ~<expr> | ~[ substitution* ] <expr>
substitution -> id = id
runtime_binding -> id : expr
~~~

替换表中的项以逗号分隔。pattern splice 的操作数也是一个 comptime 表达式，它求得模式代码；操作数本身不是源码模式。

## 正式定义

### 两种阶段，三种代码

所有 quoted code 都是 comptime 值。生成器在编译时取得、保存和组合这些值；生成的主体在 runtime 执行。引用运行时绑定保存的是绑定关系，不读取未来对象的内容。

| 类别 | 接口方向 | 使用时机 |
|---|---|---|
| expr | 给定上下文 Ψ，产生 T | 接入表达式位置 |
| pattern | 匹配 T，成功产生 Ψ | 接入模式位置 |
| arm | 外部上下文、模式、成功绑定及主体连在一起 | 接入函数级分支或匹配规则 |

上下文 Ψ 只包含 runtime 绑定，可以有多个成员并保持依赖次序。类型、代码值及生成器元信息处于 comptime 环境，不是 Ψ 中的运行时数据。

上下文多态采用 rank-1：生成器或模板的最外层可以量化上下文，使用时实例化；不能把仍可任意实例化的上下文多态方案塞进一个单态代码参数。

每段 quote 至多一个代码接入点。显式 splice 和 runtime 位置的 quote fn 应用均计数；普通函数体不自动是一段 quote。单洞不限制生成器最终能生成多少语句、状态或分支。

### 阅读路线

| 文档 | 内容 |
|---|---|
| [代码引用与阶段](quotation-and-stages.md) | 封闭与开放代码、两阶段、rank-1、单洞 |
| [上下文与替换](contexts-and-substitution.md) | 默认对应、显式改名、依赖与新鲜绑定 |
| [模式与 quoted arm](patterns-and-arms.md) | 成功上下文、定向检查和规则接合 |
| [Any 与代码见证](any-and-code-evidence.md) | 通配描述、真实接口与安全接入 |
| [Quote 函数与模板](quote-functions-and-templates.md) | 参数化代码、一次求值、逻辑返回和模板正文 |
| [组装有限程序](program-construction.md) | 循环生成、状态分派与正则自动机示例 |

## 常见示例

~~~flurry
pure comptime fn answer_code() -> < |- u32 > {
    '(42u32)
}

fn answer() -> u32 {
    ~<answer_code()>
}

test {
    println("{}", answer());
}
~~~

answer_code 在 comptime 返回代码值；answer 的生成主体在 runtime 交付 42。代码值不会作为一个通用运行时代码对象传到目标程序。

开放代码需要绑定供应：

~~~flurry
fn increment(value: i32) -> i32 {
    const code: < x: i32 |- i32 > = '(x + 1i32);
    ~[x = value]<code>
}

test { println("{}", increment(41i32)); }
~~~

x 是代码接口中的绑定，value 是接入处的真实 runtime 绑定；结果为 42。

## 与 Rust / Zig / C / C++ 的关键差异

C 预处理宏和 Rust 声明宏组织 token，C++ 模板与 Zig comptime 可按编译时输入生成不同程序。这些构造不直接提供本篇把显式 runtime 上下文放进代码类型、并让模式输出上下文的组合接口。[Rust 声明宏](https://doc.rust-lang.org/reference/macros-by-example.html)、[Zig comptime](https://ziglang.org/documentation/master/#comptime)

理论基础中的显式上下文与替换可参照 [Contextual Modal Type Theory](https://www.cs.cmu.edu/~fp/papers/tocl07.pdf) 与[上下文多态扩展](https://arxiv.org/abs/1801.09225)。Flurry 的 pattern、arm、单洞及 Any 接合由本主题自己的规则规定，不从论文自动继承其元理论结论。

## 运行时实现设计引导

runtime 只执行接入后的程序。普通调用、分支、对象访问和清理保持原有契约；quote 本身不要求在 runtime 分配代码对象或解释源码。

代码共享不等于资源共享或复制。两次接入如果使用同一份运行时 owner，必须检查生成后的真实使用顺序。

## 编译器实现引导

编译器分别保存阶段环境、runtime 绑定上下文、代码类别、结果契约和控制接口。quote 构造这些信息；splice 恢复接口、建立合法替换、新鲜化内部绑定，并接回目标程序。

接入后的程序仍接受阶段、类型、效果、可达性和资源检查。普通 Splice trait 的数据转换不替代这里的代码接入检查。
