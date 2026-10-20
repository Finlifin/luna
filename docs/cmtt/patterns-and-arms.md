# 模式代码与 quoted arm

## Intro

表达式代码需要输入绑定，模式代码则在匹配成功时提供绑定。将两者接在一起，需要知道主体使用的是哪一次匹配取得的值。

quoted arm 保存这份关联，让一条规则的模式、guard 和成功主体作为整体被复用。

当前实现已接通有显式接口的局部 pattern const、代码别名及默认／显式输出连接。模式在构造时接受类型检查，接入时新鲜化绑定并进入普通 NIR 模式流程；guard 在实际匹配时执行，构造 quote 不执行 guard。覆盖性、失败分支与资源检查沿用普通匹配规则。输出绑定不会提前进入 subject 的类型作用域，quote 内部名字也不会泄漏到外层。模式 splice 与表达式 splice 共用单洞检查。

目前 splice 的操作数支持局部模式代码值、直接 pattern quote 和下述基本模式生成调用。省略的输出替换按该值声明的接口名字建立新绑定；显式替换可只重命名其中一部分，别名保留接口名字。重复目标名字会被拒绝，不会合并两个输出。与表达式代码的配对定向检查、runtime 依赖类型重绑定和完整 quoted arm 仍待实现。下面的完整契约与示例不因这些实现边界缩减。

已可执行的显式连接形式：

~~~flurry
fn pair_check() -> bool {
    const pattern: < (bool, bool) -| left: bool, right: bool > = '<(left, right)>;
    let ~[left = first, right = second]<pattern> = (true, false);
    let ~[left = third, right = fourth]<pattern> = (false, true);
    first and not second and not third and fourth
}
~~~

构造时检查每个公开输出确实由模式成功产生，且类型符合接口；显式 quote 替换可以给内部绑定另起公开名字。当前实现尚不支持一个内部绑定以多个公开名字导出。独立模式代码不能暗中捕获外部 runtime 局部值，但模式自己解构出的数据可以供 guard 使用。嵌套 pattern splice、析取模式和模式别名同样遵守这些检查。

默认连接也建立新的成功绑定，例如 `let ~<pattern> = (true, false);` 产生 left 和 right，`let ~[left = first]<pattern> = (true, false);` 产生 first 和 right。同名外层绑定可以被遮蔽，但不会成为本次匹配的写入目标；离开内部作用域后仍读取原来的外层绑定。先前构造的表达式 quote 也继续引用其原始身份。match 的后续候选和 let-else 的失败分支看不到本次匹配尚未交付的名字。

直接引用从接入位置的主语取得输入类型，例如 `let ~<'<(left, right)>> = (true, false);`。没有 quote 替换表时，成功绑定形成推导出的输出接口；提供 quote 替换表时，由表中端口指定公开输出，例如 `let ~<'[flag=value]<value>> = true;` 产生 flag。它与先声明模式 const 再接入经过相同的类型、捕获和新鲜化检查。没有预期主语的独立模式 const 仍需要显式接口。

表达式 splice 与模式 splice 共用执行阶段检查：quoted runtime 代码只能在 runtime 位置接入。comptime 函数、comptime 回调及必需的 comptime 求值不能执行模式 splice；在 quote 主体内构造将来执行的模式则遵守正常的单洞规则。

模式 quote 与表达式 quote 也建立独立的控制边界。guard 中的 return、break、continue 不能引用构造现场的函数或循环，包括尚在推导返回类型的外层 lambda。quote 内部自己声明的函数和循环保留各自的控制目标。构造检查完成后，外层的返回推导、循环和捕获环境恢复，不能因检查一份代码值而改变外层后续语句的含义。

### 模式生成函数的初步实现

显式 comptime 函数可以返回固定的模式接口，接收位置数据参数，使用局部数据绑定、if 与 match 选择模式，也可以转调另一模式生成器：

~~~flurry
comptime fn choose(enabled: bool) -> < bool -| flag: bool > {
    if enabled { '<true as flag> } else { '<false as flag> }
}
comptime fn forward(enabled: bool) -> < bool -| flag: bool > {
    choose(enabled)
}
fn selected_pattern() -> bool {
    const pattern = forward(true);
    true match {
        ~<pattern> => flag,
        _ => false,
    }
}
~~~

每个分支在定义阶段检查相同的输入与成功输出接口，即使函数尚未调用。实际生成只执行选中分支；普通数据表达式使用现有 comptime 求值器，不能读取调用者的 runtime 实参。生成调用复用隔离的生成器调用帧，递归构造和模板复制消耗预算；模式函数不产生 runtime 入口。

当前支持位置及具名可选的数据与模式代码输入、局部数据绑定、块尾结果、显式 return、完整 if/else、带守卫和解构的 match 与生成器转调。方法、具名代码结果、模式代码的变参集合尚未接通，遇到这些形式明确拒绝。返回接口可供局部 const 和 splice 取得输出名字。声明扫描预先收集函数返回模式接口的输出名字，因此模块级前向调用、跨文件导入及再导出不依赖 HIR 降低顺序；泛型调用和局部代码别名保留这些名字。这里只提前取得名字，实际模式、subject 和输出类型仍须通过 elaboration 检查。

模式生成器支持不可变值子句，并可与 Type 子句、普通数据参数混用。值子句在 comptime 调用中取得完成值，生成器主体可以据此选择或构造模式。值参数的领域可依赖前面的 Type 参数，也可以是已支持的不可变 nominal 聚合；指针等活动存储不能成为值子句实参。

~~~flurry
comptime fn invert(flag: bool) -> bool { not flag }
comptime fn configured_pattern()
    -> < bool -| value: bool >
    where .enabled: bool = true,
          .wanted: bool = invert(enabled),
          ensures wanted != enabled
{
    '<value if value == wanted>
}
fn selected_value_pattern() -> bool {
    const p = configured_pattern<.enabled = false>();
    true match { ~<p> => value, _ => false }
}
~~~

默认值可调用 comptime 函数并引用前面的子句。它依赖尚未完成的类型证据时等待推导，完成后供后续值／类型默认值使用；显式覆盖跳过该默认值的执行，定义检查仍检查所有默认表达式。普通调用参数和结果 subject 可为已注册的 nominal 索引提供推导证据。opaque normal-form 函数头没有可逆性规则，使用这种头时可能仍需显式提供值实参，不能借展开函数体补出它们。

值子句会进入生成调用的程序契约环境，相关谓词继续走程序验证路径。模式中的数据捕获仍须冻结，不能因实参来自值子句就绕过阶段边界。

模式生成器的数据参数可以带 `requires` / `ensures` 程序契约。默认实参完成后检查前置条件，主体返回后检查后置条件，显式提前返回也不绕过检查；`'old` 引用入口数据快照。定义阶段检查谓词的类型和阶段合法性，不执行未知数据上的谓词。发散主体没有退出，因而不执行后置条件。程序谓词复用程序验证与 comptime 求值路径，类型命题仍由类型约束系统处理。

~~~flurry
comptime fn checked_binding(allowed: bool, .copy: bool = allowed)
    -> < T -| value: T >
    where T, requires allowed, ensures copy and allowed == allowed'old
{
    return '<value>;
}
fn checked_pattern() -> bool {
    let ~<checked_binding(true)> = true;
    value
}
~~~

这里 `value` 是模式成功后才产生的 runtime 输出，不能用作生成器入口或退出的程序谓词输入；契约约束的是生成调用的数据，不是将来接受匹配的 runtime 值。

模式生成器支持 Type 子句、Type 子句默认值及声明上的 trait bound。默认值使用普通函数的类型实参补全规则，允许引用前面的类型参数，拒绝引用自身或后面的参数；类型推导完成后补全依赖默认值，显式具名实参可以覆盖它。默认值不展开 normal-form 头。未调用的定义同样检查默认表达式的合法性。

~~~flurry
comptime fn default_binding(input: T)
    -> < U -| value: U > where T, .U: Type = T
{
    '<value>
}
fn default_pattern() -> bool {
    const p = default_binding(true);
    let ~<p> = true;
    value
}
~~~

可以显式提供类型实参，也可以从已完成的数据／模式实参推导 subject 与输出类型：

~~~flurry
comptime fn binding_pattern() -> < T -| value: T > where T {
    '<value>
}
comptime fn pair_pattern(p: < T -| value: T >)
    -> < (T, bool) -| value: T, flag: bool > where T {
    '<(~<p>, flag)>
}
fn use_generic_pattern() -> bool {
    const base = binding_pattern<bool>();
    const pair = pair_pattern(base);
    let ~<pair> = (true, false);
    value and not flag
}
~~~

生成器内部的类型替换和调用处的类型应用检查复用已有规则；trait bound 必须满足，normal-form 的头保持不展开。不能由返回函数体把 `Identity<bool>` 与 bool 等同。类型推导还使用直接 splice 的 subject、显式模式绑定接口和外层模式生成器的返回接口。例如 `let ~<binding_pattern()> = true;` 可以推出 T 为 bool；复合 subject 同样参与推导。显式实参必须与已知 subject 相容，推导完成后仍检查 trait bound。这些 subject 上下文会在检查数据实参之前参与推导，因此当模式 subject 已知为 u8，且生成器的数据参数与 subject 使用同一个类型变量时，数字实参可取得 u8 目标类型。相同规则覆盖变参元素、展开包及属性字段；它不扩大数字范围，也不绕过显式类型实参或 trait bound 的一致性检查。提前完成的类型默认值会在后续检查中复用。

实参之间也可以提供类型证据。生成器调用先探测已知的数据类型及已有模式代码的接口，再按源码顺序执行实参；后面的已知类型可以约束前面的字面量。该探测不调用包生产者或回调，也不提前执行实参副作用。变参中的已知元素可以提供部分 tuple 上下文，最终仍检查全部实参和完整接口。

~~~flurry
comptime fn known_number() -> u8 { 42 }
comptime fn choose_numeric(first: T, second: T)
    -> < bool -| value: bool > where T
{
    '<value>
}
fn inferred_numeric_pattern() -> bool {
    const p = choose_numeric(17, known_number());
    let ~<p> = true;
    value
}
~~~

这里先由 `known_number` 的返回签名确定 T 为 u8，再以 u8 检查 `17`；真正求值时，仍先执行第一个实参，再执行 `known_number()`。把 `17` 改成 `256` 必须拒绝。

没有实参证据也没有这些结果上下文的调用，仍需显式类型实参或声明提供的默认值。

模式生成器的具名可选参数可以从前参构造数据或模式默认值：

~~~flurry
comptime fn choose_pattern(
    first: bool,
    .second: bool = first,
    .pattern: < bool -| value: bool > = '<value if value == second>,
) -> < bool -| value: bool > {
    pattern
}
~~~

调用支持 `...packet` 把 tuple 数据包分发到固定参数入口，并可与普通位置、具名实参混用。包生产者按源码顺序只执行一次；即使展开为空，也保留这次执行。已知的目标参数类型可用于检查包内字面量；数据类型参与泛型推导，缺省入口随后按声明顺序补全。

~~~flurry
comptime fn settings() -> (bool, bool) { (true, false) }
comptime fn selected(first: bool, second: bool, .enabled: bool = true)
    -> < bool -| value: bool >
{
    '<value if value == (enabled and first and not second)>
}
fn expanded_pattern() -> bool {
    const p = selected(...settings());
    true match { ~<p> => value, _ => false }
}
~~~

直接 tuple 语法的字段分别作为实参，因此可以包含模式代码实参；经普通数据求值器产生的数据包不能伪装成模式代码证据。非 tuple、宽度不匹配、重复供应和 runtime 包生产者均拒绝。声明侧的数据变参收集入口支持 tuple 集合，模式代码集合仍待接通。

数据变参 `...items` 把剩余位置实参收集为一个 tuple 输入。声明可给固定 tuple 类型，也可用 Type 参数接收推导出的完整 tuple 类型；空集合也是一个 tuple 输入。固定入口提供各元素的检查上下文，后续默认参数可以引用已经完成的收集值。具名 `.items=packet` 则供应整个集合，不能再重复供应其中的元素。注册 Tuple 构造器时，收集结果保留该 normal-form 头。

~~~flurry
comptime fn collected(...items: (bool, bool), .copy: (bool, bool) = items)
    -> < bool -| value: bool >
{
    let (first, second) = copy;
    let wanted = first and not second;
    '<value if value == wanted>
}
fn collected_pattern() -> bool {
    const p = collected(true, false);
    true match { ~<p> => value, _ => false }
}
~~~

收集本身不改变求值顺序：散布在具名参数之间的元素仍在各自源码位置执行，最后组装为一次输入。含活动指针的集合继续保留其对象身份。

属性调用把 `{ name: value, ... }` 中的具名字段收集到唯一可用的 record 变参入口。显式类型实参先确定各入口的类型，因此同一次调用可以把位置参数收集到 tuple、把属性收集到 record。字段独立按源码位置求值，不会因为最后组装到同一个 record 就跨越中间的数据或代码参数。固定 record 类型提供字段检查上下文；重复、未知或缺失字段以及歧义入口均拒绝。

~~~flurry
pure comptime fn schema(...fields: T) -> T where T { fields }
const OPTIONS = schema { enabled: true };
typealias Options = OPTIONS'type;
comptime fn configured(...options: Options) -> < bool -| value: bool > {
    let enabled = options.enabled;
    '<value if value == enabled>
}
fn configured_pattern() -> bool {
    const p = configured { enabled: true };
    true match { ~<p> => value, _ => false }
}
~~~

属性集合也参与类型推导，随后可供后面的默认参数引用。整个调用仍产生 comptime 模式代码；属性调用不使普通 comptime record 自动成为 runtime 值。

模式生成器的数据参数可以在本次 comptime 调用内保留活动存储，包括指针、含指针的聚合，以及同包中的合法 comptime 回调。直接传参、默认参数、生成器转调与展开包保留所指对象身份，不把指针复制成脱离调用者的快照。入口／退出契约可读取这些数据。模式最终只能捕获可冻结的数据，不能把活动指针或不合法的回调捕获带入 runtime。

~~~flurry
comptime fn remember(flag: *bool) -> < bool -| value: bool > {
    let wanted = flag.*;
    flag.* = not wanted;
    '<value if value == wanted>
}
~~~

生成器按段执行数据计算时，为当前可读取的机器标量 pointee 建立有预算限制的快照，并在本段入口用程序谓词检查该快照。这样后续机器整数运算可以利用本次 invocation 的当前数值。每段重新观察；写入、别名修改及调用仍按程序验证层规则使旧事实失效，不把前一段的条件无条件沿用到下一段。快照不延长对象生命期，也不授予 Copy 或所有权。

`remember` 先读取当前值，再修改调用者的存储；返回模式保留的是读取到的 bool，随后修改该存储不改变已生成模式的判别条件。直接在 quote 的 guard 中引用 `flag.*` 则会试图捕获 comptime 活动存储，必须拒绝。

显式实参按源码顺序求值，再按声明顺序填入缺省值；只有前面的参数对后面的默认值可见。显式覆盖跳过对应默认值的执行，但定义检查仍验证所有默认表达式。implicit 参数支持显式具名供应，尚不自动寻找供应者。缺少必需输入、重复供应和未知具名入口均拒绝。

生成器内部也可以用局部 const 保存模式代码、建立别名并返回。直接 quote 的局部绑定需要标注模式接口；生成器调用及别名可以继承接口：

~~~flurry
comptime fn cached_matching(expected: bool) -> < bool -| value: bool > {
    const pattern: < bool -| value: bool > = '<value if value == expected>;
    const alias = pattern;
    alias
}
~~~

这些代码值不进入运行时存储。分支内绑定遵循局部作用域；返回的代码保留自身接口与已完成的内容。表达式代码生成器同样可以绑定局部模式代码，在所构造的 runtime quote 中 splice；尚待抽象数据的模式以接口证据参与定义检查。

模式生成器使用与表达式生成器相同的语句控制：支持数据赋值、结构字段写入、while、已支持的 inline 范围循环、break／continue、带标签的循环转移，以及分支内提前 return。可以迭代普通数据，再据最终数据构造模式：

~~~flurry
comptime fn parity_pattern(count: .u8'builtin) -> < bool -| value: bool > {
    var remaining = count;
    var expected = true;
    while remaining > 0 {
        remaining -= 1;
        expected = not expected;
    }
    '<value if value == expected>
}
~~~

定义检查遍历循环体和不可达语句，检查每个返回接口，但不执行循环。所有可达的有限出口必须返回模式代码；无限循环可以通过定义检查，实际调用仍受构造预算约束。多个可能返回的模式只形成共同接口证据，不以首个模式推断覆盖性。let-else 的失败分支可提前返回模式，成功绑定只进入成功路径。

生成器内部的 var 还可以保存模式代码。赋值保持 subject 与成功输出接口，允许从直接 quote、其他代码绑定或生成器结果替换内容；const 别名保留建立时的代码值，不跟随 var 后续写入：

~~~flurry
comptime fn selected_pattern(flag: bool) -> < bool -| value: bool > {
    var pattern: < bool -| value: bool > = '<false as value>;
    if flag { pattern = '<true as value>; }
    pattern
}
~~~

实际调用只保留执行路径上的写入。定义检查遇到写入后保守地保留接口证据，不以某次候选写入证明模式不可反驳；它可参与 pattern quote 和表达式 quote 的接口检查，但不能被当作已完成的匹配行为。runtime 位置的模式代码声明仍使用 const。

带显式接口的局部模式绑定、已有变量的赋值，以及模式参数的实参，都可以使用 if、match、when 或块表达式构造代码。初始化从声明接口取得期望类型；赋值沿用变量接口；实参沿用被调参数接口：

~~~flurry
comptime fn conditional_pattern(flag: bool) -> < bool -| value: bool > {
    var pattern: < bool -| value: bool > =
        if flag { '<true as value> } else { '<false as value> };
    pattern = when {
        flag => '<true as value>,
        else => '<false as value>,
    };
    pattern
}
~~~

返回模式的 when 必须有 else。定义时检查全部候选；调用时按顺序求值条件，选中后不再执行后续条件或代码构造。inline if／when／match 使用相同构造规则。局部条件初始化目前需要显式模式接口；未标注的简单代码别名和生成器调用仍可推导接口。

带守卫的 let-else 可以绑定整个模式代码值：

~~~flurry
comptime fn guarded_pattern(flag: bool) -> < bool -| value: bool > {
    const input: < bool -| value: bool > = '<true as value>;
    let chosen if flag = input else { return '<false as value>; };
    chosen
}
~~~

初始化先于守卫，各执行一次；失败路径不取得 chosen，仍可访问外层同名绑定。else 必须发散，例如返回另一个模式或跳出所在循环。定义阶段检查初始化、守卫和失败分支；成功代码接口不会替代失败出口的结果接口检查。

模式代码也可以作为固定接口的位置参数，接受局部代码值、直接 quote 或另一生成器调用的结果。参数与数据输入分开保存，不编码为解释器的普通数据值：

~~~flurry
comptime fn select_pattern(
    flag: bool,
    yes: < bool -| value: bool >,
    no: < bool -| value: bool >,
) -> < bool -| value: bool > {
    if flag { yes } else { no }
}
fn selected_argument() -> bool {
    const pattern = select_pattern(true, '<true as value>, '<false as value>);
    true match { ~<pattern> => value, _ => false }
}
~~~

调用检查 subject、输出名字及各输出类型；直接 quote 从参数接口取得 subject。模式参数可返回、建立局部别名或继续传给其他模式生成器。定义阶段参数只有接口证据，不能凭空取得匹配行为或覆盖性；可以将这种抽象参数 splice 到另一份 pattern quote 中，继续检查组合接口。实际调用返回的已完成模式可以在 runtime 位置接入。

pattern quote 可以嵌入仅有接口证据的模式参数，构造 tuple、枚举载荷等更大的模式：

~~~flurry
comptime fn pair_pattern(p: < bool -| value: bool >)
    -> < (bool, bool) -| left: bool, right: bool > {
    '<(~[value = left]<p>, right)>
}
~~~

定义检查为未知部分保留独立的 pattern-code evidence，交付声明的成功输出绑定；这些绑定可用于组合后的 guard。证据不是 wildcard，不贡献匹配覆盖性，也不能进入执行器或物化后的程序。含证据的构造结果只保留检查完成的接口；调用取得实际代码后重新构造普通模式并新鲜化绑定。单洞约束不变。模式参数也支持默认连接；`~<p>` 从参数接口交付输出名字，别名继承同一接口。显式连接可以只改名部分输出，其余输出继续默认连接；若两者产生同名绑定则拒绝。参数接口本身不会把成功输出提前引入函数作用域。

返回表达式代码的生成器也可以接收模式代码参数，从而由调用者提供匹配规则：

~~~flurry
comptime fn accepts_pattern(p: < T -| value: T >)
    -> < input: T |- bool > where T {
    '(input match { ~<p> => true, _ => false })
}
~~~

已有模式代码的 subject 与输出类型可参与类型参数推导。模式参数沿用表达式生成器的参数路由，支持具名可选输入、默认模式、前参作为后参默认值，以及显式覆盖默认值。默认值只在实际缺省时构造，覆盖后不执行原默认值。trait 代码方法保留模式输入类别及完整接口，不能用普通数据参数替代；方法转发同样检查 subject、输出名与类型。模式代码集合及展开输入尚未实现。

match 复用普通模式检查与覆盖性规则。定义时检查所有分支；调用时只对输入求值一次，按顺序尝试模式和守卫，并只构造选中分支的模式代码。解构出的数据可进入生成模式的 guard：

~~~flurry
enum Mode { equal(bool), different(bool), all }
comptime fn choose_mode(mode: Mode) -> < bool -| value: bool > {
    mode match {
        .equal(expected) => '<value if value == expected>,
        .different(expected) => '<value if value != expected>,
        .all => '<value>,
    }
}
fn selected_mode() -> bool {
    const pattern = choose_mode(Mode.equal(true));
    true match { ~<pattern> => value, _ => false }
}
~~~

抽象输入尚不能决定 if 或 match 分支时，仅保留共同接口，不能把第一个候选的匹配行为作为结果。例如一个候选是不可反驳的绑定模式，另一个是字面量模式，定义检查不能据前者批准后续无失败出口的 let。

生成器数据可以提升到模式判别值和 guard：

~~~flurry
comptime fn matching(expected: bool) -> < bool -| value: bool > {
    '<value if value == expected>
}
comptime fn literal(expected: .char'builtin) -> < .char'builtin -| > {
    '< < expected > >
}
fn matches_a() -> bool {
    const pattern = literal('a');
    'a' match { ~<pattern> => true, _ => false }
}
~~~

定义检查验证模式及输出类型，但未取得参数值时只保留接口证据，不保存带活动生成器局部变量的可执行模式。实际调用使用本次完成的数据构造模式；不同调用的判别值及 guard 数据不会串用。结构体数据也可通过字段参与 guard，runtime 函数声明身份可以作为 guard 回调传递，调用留到实际匹配阶段。comptime 回调不能因此在 runtime guard 中调用。

表达式 quote 的定义检查也接收这种接口证据。例如生成器可以先构造依赖参数的模式，再把它放进带 fallback 的 match：

~~~flurry
comptime fn matcher(expected: bool) -> < input: bool |- bool > {
    const pattern = matching(expected);
    '(input match {
        ~<pattern> => value,
        _ => false,
    })
}
~~~

未知模式不贡献覆盖性。带有效失败出口的 let-else 可以继续检查；没有 else 的 let 不能仅凭未知接口证明不可反驳。定义阶段只保存表达式代码的接口，不保存含模式证据的可执行主体；实际生成时重新接入完成的模式，随后执行正常的覆盖性、资源与阶段检查。quote 内新建的 lambda 属于该 runtime 边界，不因外层生成器是 comptime 函数而被误标为 comptime callable。

## 语法

~~~text
contextual_pattern_type -> < expr -| runtime_binding* >
quote_pattern -> '<pattern> | '[ substitution* ] <pattern>
splice_pattern -> ~<expr> | ~[ substitution* ] <expr>
case_param -> case pattern
quote_function -> quote fn id(parameter*) (-> result)? clauses? (block | (= expr))?
~~~

pattern 和分组优先级见[模式匹配](../control-flow/pattern-matching/index.md#模式优先级与括号)。quoted arm 通过 quote fn 与函数级模式表达，不增加独立的 arm 引用关键字。

## 正式定义

### 输入与成功输出

< T -| Ψ > 描述匹配 T 的模式代码。匹配成功时产生一份新的 Ψ；失败路径没有这些绑定。类型中的 Ψ 是输出接口，不是在构造 quoted pattern 时已经创建的 runtime 对象。

独立 quoted pattern 的公开接口不能暗藏额外 runtime 局部输入。需要外部运行时阈值或存储身份的规则，应在整个规则接口中交付相应上下文，并将它同时用于模式和主体。

每次 pattern splice 都产生新绑定身份。重用同一代码值不会让两场匹配共享一份运行时变量。模式导出的依赖类型也随这次新鲜化一起更新。

### 配对的定向检查

当生成器接口明确要求一份 < T -| Ψ > 与一份 < Ψ |- U > 时，先确定模式的输出接口，再在该虚拟上下文中检查表达式代码。正文中的名字指向接口授予的绑定，而不是偶然同名的外部变量。

普通元组相邻放置两份 quote 不会自动建立这种关系；关系必须来自目标接口、合法的替换或完整的 arm。模式可以导出更多绑定，但未被主体使用的资源仍须履行清理责任。

### Arm 的完整接口

内部可以把一份 arm 记为 Arm(Γ; T; Ψ; U)：在外部 Γ 下匹配 T，成功产生 Ψ，再在 Γ、Ψ 下交付 U。Arm 是分析记号，不是公开类型工厂。

模式、成功主体和控制关系作为一份代码保存。不能仅把主体取出当作闭表达式，再靠同名变量补齐缺失输入。不同 arm 可以拥有不同的 Ψ，只需满足目标要求的外部接口和结果接合。

### 规则集合

模板的 arm 接入协议提供有限规则集合及分派契约。单条允许非穷尽的规则不承担全输入覆盖，但组合后的目标仍必须满足对应 match 的覆盖要求。

open code 适合分散维护的大量入口，如重写规则或路由；一个已由生成器完整持有的有限图不需要先变成分散注册集合。规则集合来源必须明确，不能把文件枚举顺序当作重叠模式的优先级。

## 常见示例

### 模式产生绑定，正文使用它

~~~flurry
fn matching_value(
    value: u32,
    comptime pattern: < u32 -| number: u32 >,
    comptime body: < number: u32 |- u32 >,
) -> u32 {
    value match {
        ~<pattern> => ~<body>,
        _ => 0u32,
    }
}

test {
    let first = matching_value(42u32, '<(20u32..100u32) as number>, '(number + 1u32));
    let second = matching_value(12u32, '<(20u32..100u32) as number>, '(number + 1u32));
    println("{} {}", first, second);
}
~~~

输出 43 0。第二次失败没有产生 number，因此 fallback 不能接入使用 number 的 body。matching_value 本身不是 quote，其中两个独立 splice 不违反单洞规则。

### 两次接入分别产生绑定

~~~flurry
fn extract_twice(left: u32, right: u32) -> u32 {
    const pattern: < u32 -| number: u32 > = '<number>;
    const read: < number: u32 |- u32 > = '(number);
    let ~[number = first]<pattern> = left;
    let ~[number = second]<pattern> = right;
    ~[number = first]<read> + ~[number = second]<read>
}

test { println("{}", extract_twice(20u32, 22u32)); }
~~~

输出 42。first 和 second 是两份不同的绑定；同一个 pattern 值只共享代码结构。

### 元组模式导出两个绑定

~~~flurry
fn add_pair(pair: Tuple<u32, u32>) -> u32 {
    const pattern: < Tuple<u32, u32> -| left: u32, right: u32 > = '<(left, right)>;
    const body: < left: u32, right: u32 |- u32 > = '(left + right);
    let ~<pattern> = pair;
    ~<body>
}

test { println("{}", add_pair((19u32, 23u32))); }
~~~

输出 42。模式成功建立的同一对应供给 body；不能用另一场匹配的 left 和本场的 right 拼出一份未经检查的依赖上下文。

### Quote 函数的分支保存成功关联

~~~flurry
enum Command { reset, add(u32), }
quote fn command_value(command: Command) -> u32;

quote fn command_value(case .reset) -> u32 { 0u32 }
quote fn command_value(case .add(number)) -> u32 { number + 1u32 }

fn request() -> Command {
    println("request");
    Command.add(41u32)
}

test { println("{}", command_value(request())); }
~~~

输出 request、42。实参只执行一次，add 分支的 number 与其主体一起接入。reset 分支不能使用这个 number。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 宏可接收 pattern 与 expr 片段，C++、Zig 可通过模板或编译时分派生成相应分支；它们不直接提供本篇模式输出上下文与正文输入上下文的代码类型关系。C 宏则需由作者显式管理生成绑定。[Rust 宏片段](https://doc.rust-lang.org/reference/macros-by-example.html#metavariables)

## 运行时实现设计引导

接入后的模式执行普通判别与绑定。只有成功路径初始化导出位置，失败转向下一臂或既定失败出口。尝试一个模式不能提前消费后续失败路径仍需要的值。

完整 arm 的 scrutinee 按目标构造只求值一次；模式观察可以按契约共享，成功主体不能在尚未选中时执行。

## 编译器实现引导

1. 取得 pattern 或 arm 的实际类别和输入接口。
2. 为外部 Γ 建立替换，并检查模式输入 T。
3. 给每次接入分配新鲜成功上下文 Ψ，同时更新全部依赖。
4. 在成功环境检查主体；失败环境不包含 Ψ。
5. 对 arm 统一替换模式、guard、主体及控制出口。
6. 组合有限集合，检查结果、覆盖、重叠策略、效果及资源责任。
7. 保存规则来源和集合契约，作为接入与缓存的依赖。

类型相容不能替代“这份成功绑定属于这次匹配”的证据。缓存可共享规则结构，但每个接入点必须重新实例化绑定身份。
