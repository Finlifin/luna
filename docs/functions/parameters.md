# 参数与默认值

## Intro

参数声明规定一个输入怎样来到函数中。最直接的参数接收一个实参；可选参数提供缺省计算，变参则把多个输入组成一个值。

可以省略一个输入，不表示它不存在。调用主体之前，所有必需的参数都必须取得合法值或符号绑定。

## 语法

~~~text
anchored_parameter -> id | id : expr
optional_param -> .id : expr = expr
vararg_param -> ...id : expr
implicit_param -> implicit parameter
comptime_param -> comptime parameter
named_arg -> .id = expr
extend_arg -> ...expr
extended_application -> expr { (property | expr)* }
property -> id : expr
~~~

变参收集位置输入，拓展变参收集属性输入；声明及协议共同确定入口。它们都以整个收集结果的类型约束输入。修饰符能组合的范围由声明种类决定，不能任意堆叠。

## 正式定义

### 参数类别

| 类别 | 输入关系 | 缺省或接入方式 |
|---|---|---|
| 锚定 | 一个参数对应一个实参 | 必须取得该实参；lambda 的调用参数只有此类 |
| optional | 一个可省略输入 | 必须有默认表达式 |
| 变参 | 多个位置输入组成一个值 | 默认收集成元组，可由 trait 协议定制 |
| 拓展变参 | 多个属性输入组成一个值 | 默认收集成匿名结构体，可由 trait 协议定制 |
| implicit | 自动供应一个输入 | 由 elaboration 协议供应，也可显式按名提供 |

comptime 指定阶段；quote、lambda、case、catch 与接收者各有自己的角色，见[参数阶段](comptime_fn.md)和[参数 elaboration](parameter-elaboration.md)。这些维度不扩大闭包形参的种类。

### 显式命名供应

变参、拓展变参、implicit 和 optional 都接受 `.name = expr`。表达式直接提供那个参数的完整值，不再运行该位置的缺省供应或收集协议，但仍需通过类型、阶段、适配和契约检查。

直接提供一个元组给变参，不会再包装为只有一个元素的外层元组。一个位置不得重复供应，也不得既提供整个包又向同一包继续追加收集项。未知名字不能默默创建参数。

普通锚定参数不因存在此形式就自动成为具名可选参数。

调用表面形式与供应来源应分别阅读：

| 调用处写法 | 接入对象 | 是否执行该位置的缺省处理 |
|---|---|---|
| 位置表达式 | 锚定参数，或位置收集的一个输入 | 依据位置映射 |
| `.limit = expression` | 名为 limit 的 optional／implicit 等合法入口 | 不执行该位置默认或自动供应 |
| `.items = tuple_value` | 收集参数的完整值 | 不再收集，不再多包一层元组 |
| 扩展调用中的 `name: value` | 属性输入 | 由对应属性收集入口处理 |

命名实参中的点和等号，与属性中的冒号承担不同工作。直接提供一个属性包，也使用 `.name = expr`；字段数恰好为一时同样如此。

### 可选默认值

默认表达式在定义作用域中解析，可以依赖前面已声明的参数。它只在该输入缺省时计算；显式实参全部取得后，按形参声明依赖次序补齐缺失位置。

runtime 默认计算必须达到 **P3 稳定观测，并在合法入口下有限完成**。它可以观察有效的前序输入状态，不能隐含修改入口可观察状态、输出或交付新的可观察资源身份。适配与临时值清理也计入默认计算。

稳定性只在取得依据的范围内成立。同一个指针在两次调用中的内容可能不同，默认值不能只以指针位值为键永久缓存。默认值的纯度也不准许复制非 Copy 责任。

运行检查失败是调用准备的失败路径；总性不能作为删除前提检查的依据。comptime 与子句默认值遵守[编译时求值政策](../comptime/purity-and-normal-form.md)，不套用 runtime P3 的限制。

### 收集

默认位置收集保持各输入的类型和次序。默认属性收集保留字段名字及相应类型；它不是把字段统一转换为一个共同元素类型。

定制协议只改变如何供应自己的参数，不能重写其他参数或任意重排外层调用。具体边界见[elaboration](parameter-elaboration.md)。

收集保留每个输入的按值交付含义。收集一个非 Copy 值会转移相应责任，包整体再次交付时按包的实际能力处理。一个收集参数最终只有一个绑定；函数体不会为每个原始实参自动得到一组同名局部变量。

### 参数依赖与缺省路径

后面的默认表达式可以读取前面已经绑定的值，也可以使用前面显式提供的配置。它不能引用尚未声明的后序参数。将某个可选项改为显式供应，仅关闭该项的缺省路径，后面的默认表达式仍会看到本次实际绑定的值。

默认表达式由定义作用域决定意义，不在调用者局部变量中重新查找同名标识符。其 runtime 读取发生在本次调用准备阶段；“定义处解析”不等于“定义时只计算一次”。

## 常见示例

### 默认值依赖前面的参数

~~~flurry
fn select_limit(requested: u32, .limit: u32 = requested) -> u32 {
    limit
}

test {
    println("{} {}", select_limit(12), select_limit(12, .limit = 7));
}
~~~

输出 `12 7`。显式提供 limit 时，不执行其默认表达式。

### 分页接口中的连续依赖

~~~flurry
fn page_bounds(
    total: u32,
    .start: u32 = 0,
    .count: u32 = total - start,
) -> Tuple<u32, u32>
where requires start <= total and count <= total - start
{
    (start, start + count)
}

test {
    let (first_start, first_end) = page_bounds(100);
    let (tail_start, tail_end) = page_bounds(100, .start = 80);
    let (part_start, part_end) = page_bounds(100, .count = 5, .start = 20);
    println("{}..{}", first_start, first_end);
    println("{}..{}", tail_start, tail_end);
    println("{}..{}", part_start, part_end);
}
~~~

输出 `0..100`、`80..100`、`20..25`。第二次先绑定显式 start，再据它求缺省 count。第三次两个输入均已给出，按书写顺序取得 5 和 20，再分别绑定到 count 和 start；不会先执行两个默认值。

requires 同时约束合法区间和机器算术。若 total 小于 start，不能先执行无符号减法再用结果检查范围；默认计算本身的前提必须在执行前具备。

~~~flurry
-- 拒绝：first 的默认表达式引用后序参数 second。
fn invalid_dependency(.first: u32 = second, .second: u32 = 1) -> u32 {
    first
}
~~~

### 默认读取本次状态

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

输出 `1 2 9`。最后一次只使用显式 limit；前两次观察的是各自调用时的有效内容版本。

### 位置收集与完整供应

~~~flurry
fn volume(...shape: Tuple<u32, u32, u32>) -> u32 {
    let (width, height, depth) = shape;
    width * height * depth
}

test {
    let collected = volume(2, 3, 4);
    let supplied = volume(.shape = (2, 3, 4));
    println("{} {}", collected, supplied);
}
~~~

两个结果都是 `24`。shape 是三个元素的元组，不是三次独立参数声明。

变参也可以保留不同类型的输入：

~~~flurry
fn keep_inputs(...items: T) -> T
where T
{
    items
}

test {
    let (number, enabled, label) = keep_inputs(7u32, true, "ready");
    println("{} {} {}", number, enabled, label);
}
~~~

输出 `7 true ready`。items 的三个位置分别保留 u32、bool 和字符串类型；函数返回整个包，无需把它们转换成同一种元素类型。

~~~flurry
-- 拒绝：同一收集参数既收到位置输入，又收到完整包。
fn invalid_volume_call() {
    volume(2, 3, 4, .shape = (5, 6, 7));
}
~~~

### 属性收集与完整供应

~~~flurry
fn keep_properties(...properties: T) -> T
where T
{
    properties
}

test {
    let collected = keep_properties {
        width: 2u32,
        height: 3u32,
    }
    let supplied = keep_properties(.properties = collected);
    println("{} {}", supplied.width, supplied.height);
}
~~~

输出 `2 3`。扩展调用中的属性由这一入口收集为匿名结构体；第二个调用直接交付整个结构体。函数不要求两个字段具有同一种类型，也不在主体中依赖未知 T 的具体字段。

同一收集接口也可以保存配置中不同种类的字段。接续 keep_properties：

~~~flurry
test {
    let config = keep_properties {
        limit: 8u32,
        enabled: true,
        label: "worker",
    }
    println("{} {} {}", config.limit, config.enabled, config.label);
}
~~~

输出 `8 true worker`。位置收集的顺序槽和属性收集的命名字段不是同一个数据形态；读取接口要遵循实际取得的类型。

### 拒绝重复或不存在的命名输入

接续 select_limit，以下两个函数分别是拒绝例：

~~~flurry
fn repeated_limit() -> u32 {
    select_limit(12, .limit = 3, .limit = 7)
}

fn misspelled_limit() -> u32 {
    select_limit(12, .limti = 3)
}
~~~

前者为同一个位置提供两份值；后者没有可绑定的名字。都不能用“后一个覆盖前一个”或“忽略不认识的配置”解释。

### 输出不能隐藏在默认计算里

~~~flurry
fn announce_limit() -> u32 {
    println("selecting limit");
    8
}

-- 拒绝：默认计算包含输出。
fn invalid_default(.limit: u32 = announce_limit()) -> u32 {
    limit
}
~~~

允许在调用处显式取得这个值：

~~~flurry
fn configured(.limit: u32 = 8) -> u32 {
    limit
}

test {
    println("{}", configured(.limit = announce_limit()));
}
~~~

这里的输出属于显式实参求值，不冒充纯默认计算。以上拒绝例与合法例分别检查，不把包含非法声明的代码块当作可运行程序。

## 与 Rust / Zig / C / C++ 的关键差异

C++ 有默认实参，但默认计算不能直接求值同一函数的其他参数；Flurry 允许前序依赖，并限制 runtime 默认计算的行为。[C++ default arguments](https://eel.is/c++draft/dcl.fct.default)

C 的省略号参数通过可变参数访问机制读取，不能等同于保留逐项类型的元组收集。Rust、Zig 程序可通过结构体或元组显式组织配置，Flurry 的命名供应还覆盖 implicit 与收集参数。[C11 §7.16](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf)、[Zig functions](https://ziglang.org/documentation/master/#Functions)

## 运行时实现设计引导

直接供应与缺省供应是互斥路径。调用计划须在运行前确定每个位置采用哪条路径，避免把有默认值的参数实现成“总先算默认，再用显式值覆盖”。

收集临时量在每项成功后取得对应责任。失败时只清理已经取得的部分；完整包成功交付后，由参数的资源契约接管。不能让临时项与包字段同时各拥有一份终结责任。

对缺省值逐个执行阶段合法的计算，绑定前序结果后再处理后项。即使实现采用寄存器或直接构造结果位置，仍须保持这些次序及有效期。

对于收集包，区分源表达式计算与包布局：先后次序来自源码，字段偏移来自目标布局。把字段按对齐要求重排，不授权重排字段输入的输出、写入或失败。收集过程的暂存槽也须记录哪些责任已经转入最终包。

## 编译器实现引导

建立“形参 → 供应来源”表。每项至多一个最终供应来源，来源可能是显式值、收集计划、implicit 协议或默认表达式。

1. 读签名，建立名字与位置索引，区分参数角色。
2. 绑定显式输入，登记直接包供应并排除该位置的收集路径。
3. 拒绝重复、未知名字及无法归属的剩余输入。
4. 在定义作用域检查默认表达式，并记录前序参数依赖。
5. 对 runtime 默认计算检查 P3 范围、有限完成、实际适配及清理。
6. 按既有阶段规则执行或保留 comptime 输入；对子句默认值执行 normalization／defer 政策。
7. 产生[调用准备计划](calls-and-returns.md)，分别保留源码次序和依赖次序。

建议以每个参数的状态表达唯一供应规则：未供应、直接供应、收集中、待默认、待 implicit、已完成。直接供应与收集中不能同时成立；缺省计算只有在其前序依赖满足后才能从待处理进入已完成。静态阶段选择来源，runtime 阶段执行来源中的实际计算。

无定制协议时，n 个输入与 m 个参数可用 O(n + m) 映射；协议执行及证明成本另计。诊断必须指到冲突的两个供应来源，或默认表达式中违反行为要求的实际操作。
