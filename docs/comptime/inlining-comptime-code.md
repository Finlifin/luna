# Comptime 块与 inline 控制流

## Intro

有些程序在编译时就能决定采用哪种结构，但要等运行时才能处理其中的数据。inline 控制流把这两件事分开：编译器确定分支或展开结构，生成的代码处理运行时值。

comptime 块则直接执行块内的编译时计算。块放在 runtime 函数中，也不需要读取尚不存在的 runtime 数据。

## 语法

`expr`、`pattern`、`label`、`block`、`case_arm` 复用[全局语法](../grammar/syntax)。本节只列相关控制流形式。

```text
comptime_block -> comptime { (definition | statement)* }
inline_if -> inline if expr block (else block)?
inline_when -> inline when { condition_arm* else_arm? }
inline_if_is -> inline if expr is pattern do block (else block)?
inline_if_match -> inline if expr is do { case_arm* }
inline_for -> inline for label? pattern in expr block
condition_arm -> expr => (block | statement)
else_arm -> else => (block | statement)
```

一般列表项用逗号，语句用分号；以 `}` 结束的语句省略其后分号。普通控制流见[基本控制流](../control-flow/basic.md)。

## 正式定义

### Comptime 块

块内计算属于 comptime，采用 loose 检查强度。块可以出现在 runtime 定义中；该 runtime 定义仍按 ordinary 检查。块内若再定义 runtime 函数，该函数也按 ordinary 检查。

块的结果可以用于后续编译时计算；实际进入 runtime 数据流时按跨阶段规则处理。块内的 `comptime` 不授予读取 runtime 参数实际值的能力。

### Inline 分支

`inline if`、`inline when` 以及相应模式分支在编译时确定所选分支。所选分支中仍属于 runtime 的操作进入生成代码。

ordinary normalization 要检查相应分支中的操作，而不只是等待一次具体实例化选中某支后才开始检查。路径条件能提供的命题参与分支检查。

inline 条件可以是任意 comptime bool 表达式。编译器尝试提取良定义命题；不能提取时，按未知 bool 检查相应分支，不因此增加路径事实。条件未知不等于条件为 false，也不等于可以提前执行两侧副作用。

### Inline 循环

`inline for` 在编译时确定展开结构。遍历元组的结构时，元素的位置和类型可以在编译时用于选择代码，元素值仍可以属于 runtime。

展开循环不免除循环体操作的 requirement。例如，取得一个元素的类型不等于该元素一定实现某个 trait；使用对应操作仍须有约束支持。

普通 `for` 的边界即使是编译时常量，也不等同于 `inline for`。前者没有强制展开的语义承诺。

### 失败

必须在编译时确定的分支条件或展开结构，在所需输入已经绑定后仍无法合法求值时，为编译错误。普通 runtime 条件应使用对应的 runtime 控制流。

未选分支不进入该实例的 runtime 代码；这不取消它在 ordinary normalization 中承担的检查。

当前实现已保留 HIR 中的 inline 控制要求，并接通条件可完成求值的 `inline if` 和 `inline when`。普通 runtime 函数的 `comptime bool` 参数可通过现有函数实例化入口选择分支，分支中的 runtime 参数仍保留到运行时。`inline when` 按源码顺序求值条件，命中后不再调用后续条件，最后的 `else` 作为 fallback；没有匹配且没有 fallback 时产生 Void。嵌套未选 inline 分支会检查操作，但不调用其条件。带子句参数和方法等路径仍受当前实例化入口的限制。签名各输入及结果类型已经建立的 pure open definition 会额外构造覆盖普通分支的行为 IR，交给 P4 分析器验证；此证据不进入可执行产物。 对已知签名的 pure 开放函数之间的调用，行为证据保留指向普通定义的调用边和原阶段标记，不递归要求实例化定义；P4 因而可以分析自递归与互递归，带类型子句的调用也保留相应绑定。实际调用仍通过正常实例化生成 runtime 程序，纯递归不因此获得 P3 有限完成保证。已完成阶段检查的普通布尔选择器在行为证据中使用抽象输入，避免把 comptime 调用当作 runtime 行为；提供类型事实的条件保留原检查路径。`inline match` 的 subject 也可作为保留其类型的抽象输入进入行为证据，支持 comptime 调用产生的 subject 和模式绑定；定义端的 subject／模式程序按 comptime 阶段检查，分支主体仍按 runtime 阶段检查。模式中的 guard 与 `and ... is ...` 变换计算先保留原程序完成阶段检查、分支类型事实和覆盖检查，再在纯度证据中替换为保留类型的抽象输入；此处理递归覆盖嵌套模式与 or-pattern 的绑定重映射，不调用这些计算，也不凭某次计算结果跳过分支主体的纯度检查。未选分支和空 inline 循环的主体同样参加纯度验证；循环索引作为行为证据的抽象输入保留，不能用某次实际迭代值替代。循环边界已经在 staged ordinary 检查中确认，构造 runtime 行为证据时不再重建或调用边界中的 comptime 计算。含暂缓类型签名的 pure open definition 仍保留明确诊断；其它尚不能建立行为证据的组合也不能仅凭所选实例通过就接受。

inline 选择结果也可继续解引用或取得 `tag_name`：如 `do { inline if flag { left(p) } else { right(q) } }.*`。定义端按已建立的操作数类型检查操作；具体实例保留所选指针或枚举值，不调用未选分支，不以类型检查替代实际内存访问。

复合赋值也支持包含 inline 选择的右值，沿用普通目标准备、运算协议和范围验证；无算术协议时向分支提供目标类型，已绑定协议时保留协议对右操作数类型的决定权。

指针及其字段写入的右侧可以包含 inline 选择，例如 `p.* = do { inline if flag { yes() } else { no() } };`。定义端只形成目标地址以取得存储类型，不为写入额外读取旧值；实际实例仍按顺序计算目标和所选值，各一次，并保留普通写入的资源、内存及纯度检查。

解构绑定的初始化式可以包含 inline 选择，例如 `let (a, b) = do { inline if flag { left() } else { right() } };`。定义端先检查所有分支并建立结果类型，再检查解构模式；具体实例只执行选中的初始化式一次。模式的可反驳性、let 不可重新赋值以及 let-else 的退出检查仍保留。普通 match 的主语也可通过 `do { inline if ... }` 组合，定义端复用主语类型检查模式、覆盖性和所有分支；具体执行只求值所选主语一次。外层为 inline match 时，抽象主语保留已验证的 comptime 阶段，分支主体仍属于 runtime，不能借嵌套选择把 runtime 参数提升为 comptime。

inline 模式分支已接通受支持的不可变模式证据、绑定及嵌套 guard：scrutinee 求值一次，按源码顺序选择 arm，成功绑定作为常量交付 runtime 主体；guard 失败后继续下一候选，选中后不再调用后续 guard。所有 arm 仍检查类型与覆盖性。嵌套 guard 与 `and … is …` 的模式转换通过 required invocation 按匹配顺序执行；失败的候选回退绑定，未到达的计算不调用。无法冻结为受支持不可变值的结果保留明确诊断，不跳过该候选。

inline 类型元组模式读取已绑定 Tuple 协议的元素视图，支持嵌套与数量检查；内部结构元组表示遵循相同规则。模式导出的 Type 和类型列表作为编译期常量交付所选主体，不创建 runtime 存储。例如：

```flurry
typealias Pair = (bool, (bool, bool));

fn has_expected_elements() -> bool {
    inline if Pair is do {
        (A, (B, C)) if A == bool and B == bool and C == bool => true,
        _ => false,
    }
}
```

这里类型比较属于编译期模式 guard，runtime 主体只留下布尔结果。确定的非元组类型可以进入 fallback；对于缺少元组视图的未知 normal-form 头，编译器保留诊断，不能展开函数体或把 Unknown 当作匹配失败而继续候选。

normal-form 头模式自身只比较保留的声明身份。因此它遇到明确的 primitive、指针、函数、结构元组或普通名义类型时，可以判定不匹配，跳过该候选及其 guard；不同的已知 normal-form 头也如此。尚未确定的 Type 参数、洞或关联类型查询不能作为这种否定证据；同头但参数数量损坏也不能当作普通不匹配来选择后续候选。

同样，inline 模式绑定取得的精确 Integer／Float 字面量保持为编译期常量，可以供主体中嵌套的 inline 判断继续计算；即便绑定未被使用，也不要求为它分配 runtime 存储。此规则不改变精确数值的阶段属性：直接把 Integer 或 Float 作为普通 runtime 函数的结果仍被拒绝，聚合值与指针的重建和资源检查也仍保留。

`inline for` 已接通有界 `Integer` 范围（开区间末端 `..`、闭区间末端 `..=`），支持经现有覆盖分析证明不可反驳的模式，包括简单绑定、通配、`_ as index` 和绑定一致的 `or` 模式；边界可来自函数的 comptime 参数。每次展开仅保留 runtime 主体，索引作为编译期常量替换，支持普通条件、状态更新、嵌套循环、带标签的 break/continue 和 return。展开后的迭代包装作用域与源码主体分别保留清理边界；实际取得的局部资源在正常退出、continue、break 和 return 时按作用域次序清理，过滤掉的迭代不构造或析构资源。空范围仍检查主体，但不调用其中嵌套 inline 控制的条件或边界；未选分支中的范围同样不调用边界。展开受编译预算限制。这些不可反驳模式可附带 guard（`inline for i if keep(i) in 0..count { ... }`）：每个索引绑定后在 comptime 判断 guard，仅保留成功迭代的 runtime 主体。空范围不调用 guard；过滤掉的主体仍检查普通操作，但不调用嵌套 inline 条件。pure 定义端检查保留抽象主体，恒假 guard 不能隐藏副作用。开放函数也支持上述不可反驳模式及 `and ... is ...` 的索引计算：定义端检查模式类型、覆盖与计算阶段，不调用计算；每次实际展开才绑定索引并求值，空范围不调用模式中的计算。模式的计算结果可用于迭代 guard 和 inline 条件，不能读取普通 runtime 参数。`tuple_enumerate` 已在 AST lowering 中形成专用 HIR 节点，保留操作数并输出 `(take-tuple-enumerate (value ...))`；操作数检查只取得结构类型证据，识别已绑定的 Tuple normal-form 头并保留原始类型身份；不会展开其它返回 Type 的函数头。非 tuple 值给出类型诊断。已接通结构类型已建立的非空 tuple 遍历，模式目前使用 `(index, element)`，两部分均须不可反驳。索引是 comptime Integer，元素按各字段自己的类型留在 runtime；异构元素不合并为一个统一 runtime 类型。主语只求值一次，原 Tuple normal-form 头保持不变。独立保存该 view、空 tuple 主体检查和一般迭代协议仍未接通，保留明确诊断。

```flurry
fn last(values: (bool, bool)) -> bool {
    var result = false;
    inline for (index, value) in values'tuple_enumerate {
        result = value;
    }
    result
}
```

`index` 可用于 inline 条件与迭代守卫；不能因为遍历在编译期展开，就把 `value` 提升为 comptime 输入。开放定义按每个元素的已知类型检查主体，未调用定义中的阶段错误也会被拒绝。守卫为 false 的主体仍检查类型和纯度，但不执行其 runtime 操作或嵌套 inline 计算。

结构已知不要求字段类型已经是具体机器类型。`(A, B)` 可以逐字段展开，元素调用分别使用 A、B 的类型参数和 trait 约束；调用者实例化后各自选择实现。例如：

```flurry
trait Ready { fn ready(self) -> bool; }

fn last_ready(values: (A, B)) -> bool where A:-Ready, B:-Ready {
    var answer = false;
    inline for (index, element) in values'tuple_enumerate {
        answer = element.ready();
    }
    answer
}
```

通过 `requires T == (A, B)` 建立的结构证据也可用于遍历 T。这不会展开返回 Type 的函数头，亦不意味着尚未确定长度的类型列表已经支持遍历。泛型实例中的资源交付仍按各实际字段类型检查：提前 break 时，已经交付给被调函数的元素由接收者清理，未访问字段由原迭代作用域清理。

两个子模式分别保留自己的计算阶段：索引子模式的 `and ... is ...` 在 comptime 完成，元素子模式的观察在 runtime 执行。元素子模式可以引用前面索引子模式建立的绑定，例如先计算 bool `first`，再交给普通观察函数；不因此允许未展开的 Integer 运算进入 runtime。

```flurry
fn choose(first: bool, value: bool) -> bool {
    if first { value } else { not value }
}

fn visit(values: (bool, bool)) {
    inline for ((index and index == 0 is first),
                (value and choose(first, value) is selected))
        in values'tuple_enumerate {
        let result = selected;
    }
}
```

主语取得后的每个元素分别保留所有权。正常迭代、过滤和 `continue` 在本轮结束时清理本轮尚未交付的资源；`break`、`return` 或跨层退出同时清理剩余未访问元素。循环体中的移动仍遵循普通规则，不能通过展开重复使用仿射元素。索引与元素的作用域仅覆盖本轮，展开不会让不同轮的绑定相互覆盖。

开放定义中的非空、无过滤 tuple 迭代会保留主体的完成性：若各元素的主体均不正常完成，且没有可能到达的本轮 break／continue，整个迭代也不正常完成。例如下面无需在循环后另加一个 bool：

```flurry
fn first(comptime checked: bool, values: (bool, bool)) -> bool {
    inline for (index, element) in values'tuple_enumerate {
        return element;
    }
}
```

普通定义检查与具体 NIR 对字面量 bool、not、and／or 使用一致的保守完成性判断。例如 `if not false { return value; }` 和 `(not true) or do { return value; }` 不会正常继续。该分析不调用函数，也不读取某次实例的 comptime 参数。所有分支仍检查类型；只在计算能否继续及循环退出目标时排除确定不会执行的分支。when 按条件顺序保留这一规则，未知条件仍保留后续可能路径。

对已知字面量主语及不执行观察代码的模式，普通 match 也能按顺序判定所选候选，保留该候选的 Never 并排除其它候选的循环退出。当前支持与普通 NIR 控制流相同的惰性判别范围，包括 bool 和结构 tuple。未知主语、guard 与顺序计算模式不据此猜测候选；覆盖性和所有候选的类型检查仍先完成。已检查片段中的 break／continue 复用同一控制流分析，保留短路、求值顺序及闭包创建环境的边界。

普通 while 在开放定义中也保留完成性。退出证据按循环身份区分 break 与 continue：只有到达本循环的 break 能使无条件重复循环正常结束，continue 不提供出口。常量条件的不可达 break、return 后的 break 和内层循环自己的 break 都不会制造外层出口；未知条件仍保留结束循环的可能。while 条件本身返回函数时，也不再要求循环之后额外交付值。这只是普通控制流类型检查，不把 while 改成 inline 展开。

开放函数调用已经形成签名的普通函数时，也支持有明确 tuple 结构的展开实参，包括泛型目标、默认参数和显式命名覆盖。例如：

```flurry
fn second(first: bool, second: bool) -> bool { second }

pure fn choose(comptime enabled: bool, values: (bool, bool)) -> bool {
    inline if enabled {}
    second(...values)
}
```

这里复用普通调用的参数包路由、元素上下文和准备计划：参数包求值一次，缺省值在显式输入之后准备，显式覆盖跳过对应默认计算。拥有型字段各交付一次，pure 定义仍验证完整调用行为。Any 不提供包结构，错误形状、宽度和字段类型继续拒绝；开放目标现在可以接收字面 tuple 参数包：编译器按元素保留源码表达式，分别使用目标槽位的阶段和类型上下文。因此同一包内可转发 comptime 参数并供应 runtime 值，不需要先把整个包存成 runtime tuple。多个字面包、空包及普通位置实参可以组合；经过局部别名和多层函数的 comptime 转发也会传播实例化需求。计算所得参数包也可调用 staged 输入已经能完成的开放目标，复用其普通实例化、泛型推断和参数准备路径，包括已经能够形成的延迟参数／结果类型。仍未知的 comptime 输入不能被这一途径提前调用；这类调用所需的纯描述路由尚未全部接通。 展开包内部可以使用依赖 comptime 参数的 inline 选择：定义检查验证所有分支及选择条件的阶段，实例化时才选择分支。inline 输入在普通实参检查取得上下文后进行定义检查，因此包内未标注的闭包可以从目标参数取得类型；泛型调用也可利用其他元素和结果上下文。例如：

```flurry
typealias Callback = fn(bool) -> bool;
fn apply(f: Callback, value: bool) -> bool { f(value) }

fn choose_callback(comptime flag: bool) -> bool {
    apply(...do {
        inline if flag { (|x| x, true) }
        else { (|x| not x, true) }
    })
}
```

多个展开包的 inline 分支若共有相同宽度，布局分析可以利用这一宽度向各包传递上下文；布局分析不执行选择条件，也不以某个分支的宽度代替所有分支。命名输入中的 inline 选择同样使用其参数上下文。

调用的实参也按源码顺序提供完成性：先前实参为 Never 时，后面的实参仍检查类型，但不能添加实际上不会发生的循环退出；调用本身保留 Never。命名实参不能按参数声明顺序重排这项证据。泛型推断需要重试输入时，各次成功输入的退出证据独立保存，最后按路由的源码顺序合并；推断的先后不是执行的先后。已经准备的资源在提前返回路径中仍清理一次，被调函数没有执行。非短路二元运算同样在左操作数不完成后停止收集右操作数的退出，普通运算类型检查保留。

聚合构造按实际字段求值顺序传递完成性。tuple 的前序元素不正常完成时，后续元素仍做类型检查，但不再提供运行时退出证据；共享 NIR 分析对结构体、匿名记录、枚举载荷及类型列表同样在第一个 Never 字段之后停止收集转移。已经取得的字段资源仍按普通部分构造规则清理，不能因为构造未完成而遗漏，也不执行未取得字段的清理。

普通 let／var 的初始化式和 if 条件也保留 Never：其中的 return 不会被外层绑定或条件语法抹成 Void。这同样适用于循环体中的组合表达式。后续不可达操作仍检查类型，不以提前返回隐藏非法调用。

内层循环自己的 break 不算退出这个 tuple 迭代；指向本轮的标签退出则参与检查。存在过滤 guard 时仍可能没有主体执行，不能仅凭主体中的 return 省略后续返回值。完成性检查保守覆盖普通分支，不以某次 comptime 参数值替代定义端证据。

函数定义中的局部变量及其字段赋值右侧、`bitvec` 视图与显式转换输入、索引／切片及字段投影接收者、调用输入、元组元素、`not`/负号的一元操作数、BitVec 拼接、算术与比较操作数及 `and`/`or` 操作数可以包含 inline 控制。定义检查递归检查这些位置，实例化时再选择 inline 分支；普通布尔短路仍在 runtime 执行。其它复合表达式的位置尚未全部接通。上述实现限制不改变语言契约。

## 常见示例

### 只在编译时选择结构

```flurry
fn signed_value(comptime negate: bool, value: i32) -> i32 {
    inline if negate {
        -value
    } else {
        value
    }
}
```

`negate` 决定实例中的分支，`value` 在运行时才传入。负号是否出现在实例中由编译时输入决定。

### 多路选择

```flurry
fn mark(comptime kind: Integer, value: i32) {
    inline when {
        kind == 0 => println("plain: {}", value),
        kind == 1 => println("marked: {}", value),
        else => println("other: {}", value),
    }
}
```

选择打印形式发生在编译时，实际打印发生在运行时。

### 块内求值

```flurry
fn print_area() {
    const AREA = comptime {
        12 * 12
    }
    println("{}", AREA);
}
```

块尾的值用于初始化常量；以 `}` 结束的绑定后面不再写分号。

### 展开固定结构

```flurry
fn print_indices() {
    inline for i in 0 .. 3 {
        println("{}", i);
    }
}
```

展开后包含三个打印操作。这些打印不因为位于 inline 循环体中就提前在编译时执行。

## 与 Rust / Zig / C / C++ 的关键差异

| 对照语言 | 对应机制与差异 |
|---|---|
| Rust | const 块用于常量求值；没有等同于这里的逐项 inline 展开语句，代码重复可用宏表达。[Rust 常量上下文](https://doc.rust-lang.org/reference/const_eval.html#const-context)、[宏重复](https://doc.rust-lang.org/reference/macros-by-example.html#repetitions) |
| Zig | `inline for` 同样展开循环；编译时已知条件会跳过未选分支分析。Flurry ordinary 还承担定义端的分支检查。[Zig 参考](https://ziglang.org/documentation/master/#Compile-Time-Parameters) |
| C11 | `#if` 在预处理阶段选择文本，不根据函数的类型契约传播命题。[C11 草案 §6.10.1](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf) |
| C++ | `if constexpr` 的 discarded statement 有模板实例化规则；Flurry 的 inline 分支按 ordinary/loose 契约检查。[C++ constexpr if](https://eel.is/c++draft/stmt.if) |

## 运行时实现设计引导

1. 对已选择的 inline 分支，只生成该分支的 runtime 操作。
2. 对有 n 个展开项的 inline 循环，生成 n 份相应 runtime 主体；每份使用其对应的编译时结构信息。
3. 对主体中的普通 runtime 条件、循环与调用，保留它们的控制和数据依赖。
4. 对已执行完的 comptime 块，不生成重复求值；其结果跨界时仍执行必要的转换。

若主体大小为 B，直接展开的代码规模可达 O(nB)。实现可进行保持语义的优化，但不能把未执行的 runtime 副作用移到编译期。资源不足以完成必要展开时应报告编译失败。

## 编译器实现引导

### 分支检查与选择

1. 在 normalization 建立分支前的事实环境。
2. 分析条件的阶段与依赖；能提取命题时，分别构造对应分支的适用事实。
3. 在 ordinary 下检查分支操作。条件不能提取命题时，保守检查而不制造额外事实。
4. 在条件输入已具备且控制依赖允许时执行条件，选出该实例的分支。
5. 将所选分支继续实例化；保留其中的 runtime 操作。

不变量：检查某支不等于执行某支。未选择分支的计算不得仅因遍历 IR 就产生求值副作用。

### 循环展开

1. 检查被遍历对象满足该遍历操作的 requirement。
2. 取得展开所需的结构；对结构已知、元素值未知的对象，仅提取阶段允许的结构信息。
3. 为每个展开项绑定对应编译时信息，检查并实例化主体。
4. 合并主体的 runtime 控制流，保持原有次序及控制转移目标。

元组的结构遍历不能降为先读取 runtime 元素来猜测类型。普通循环也不能仅因边界已知就按强制展开处理。

范围迭代的不可反驳模式还可以顺序计算新的编译期绑定：

```flurry
typealias Integer = .Integer'builtin;
pure comptime fn increment(value: Integer) -> Integer { value + 1 }
fn visit() {}

fn run() {
    inline for (index and increment(index) is next) if next != 2 in 0..4 {
        inline if next == index + 1 { visit(); }
    }
}
```

每轮先匹配 `index`，再计算 `increment(index)` 并绑定 `next`，随后判断 guard；上例仅留下三次 runtime `visit()`。模式计算不会留到 runtime 重做。空范围以及未选中的外层 inline 分支不调用这些计算，但模式和主体仍检查类型与阶段；不能用空展开掩盖 runtime 参数进入 comptime 的行为。
