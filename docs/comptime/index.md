# 编译时计算

comptime 没有指针。comptime 方法的 `self` 使用值引用语义，接收者更新不通过 `*self`、`.ref` 或 `.*` 表达。解释器的内部存储引用不是源语言指针。

本文下方实现进展中涉及 comptime 指针读写、`*Integer`、`*List<T>` 或通过 runtime Copy 取得 comptime 数据的描述，均属于待纠正的旧实现记录，不是有效语言规则或可仿照的用法。当前 `List.append`、列表范围追加及相关 VM 路径尚需迁移到值引用语义；不能仅删除指针语法而保留按值复制行为。

## Intro

传统编译时计算（如 C 的预处理器宏或 C++ 的 constexpr）在表达能力和类型安全之间存在取舍。Flurry 将编译时计算设计为语言的一等公民：编译时函数使用与运行时相同的语法和类型系统，同时通过执行阶段标记区分两者的边界。

Flurry 程序中的一部分计算由编译器执行，其余计算留给生成的程序。编译器先处理已知的信息，让程序保留需要运行时数据的工作。

输入尚未确定时，编译器仍能利用操作的契约检查代码。知道一个结果满足什么条件，往往已经足以检查后面的操作，而不必先把这个结果完整算出来。

## 当前局部 const 实现

runtime 正文中的局部 `const` 已支持编译时求值的标量、Type、元组及名义聚合数据，并可作不可反驳的解构绑定。后续使用取得完成的数据，不留下 runtime initializer；编译时初始化不能读取 runtime 参数或局部状态，也不能调用 runtime 函数。

~~~flurry
pure comptime fn invert(value: bool) -> bool {
    const result = not value;
    result
}

fn configured() -> bool {
    const Flag = bool;
    const (enabled, disabled): Tuple<Flag, Flag> = (invert(false), invert(true));
    const code = '(enabled and not disabled);
    ~<code>
}
~~~

comptime 函数中的数据 const 按每次 invocation 的输入建立绑定，不要求在检查函数定义时提前知道实参。runtime 函数中的 Type 常量可以保留当前泛型参数；normal-form 的头保持不透明，局部别名不会使 Identity<T> 等于 T。quoted code 仍走独立的代码值路径，普通数据与代码别名可组合使用。

comptime 函数中的不可变普通 `let` 也可保留已经检查出的 Type 结构信息，供后续类型位置使用：

~~~flurry
comptime fn local_type() -> bool {
    let (T, enabled) = (bool, true);
    let Alias = T;
    let result: Alias = enabled;
    result
}
~~~

观察可穿过已知元组／记录结构及其字段；一个字段尚待调用期间求值，不会丢掉其它字段已经知道的类型。普通绑定仍保留存储、initializer 和局部读取，不因此变成特化输入，也不提前调用 initializer 中的函数。只读别名链及块内遮蔽遵循原来的作用域规则。可变 Type 绑定不提供这类固定证据；runtime 中的普通 `let` 也不因此获得编译期阶段。类型头保持不透明。这条实现路径不等于已支持任意调用期间才产生的 Type 作为后续局部类型标注。

未知条件的 `if` 和 `match` 可以保留各分支共同提供的结构信息：

~~~flurry
comptime fn select_flag(flag: bool) -> bool {
    let (T, value) = if flag { (bool, true) } else { (bool, false) };
    let result: T = value;
    result
}
~~~

这里两个分支的 T 都是 bool，而 value 仍由本次调用的条件决定。合并逐字段保留共同证据；不同 Type 或不同聚合形状不会被当作相同。观察不调用条件、guard 或分支函数，也不改变它们在 invocation 中的顺序与次数。已知条件仍可使用所选分支的证据。这不允许 runtime 条件进入要求 comptime 执行的 initializer，也不代替尚未接通的动态 Type 标注。

局部代码块也可通过不可变声明、解构和别名传递这些部分已知的信息：

~~~flurry
comptime fn observed_flag(flag: bool) -> bool { flag }

comptime fn block_local_type() -> bool {
    let (T, flag) = do {
        let descriptor = (bool, observed_flag(true));
        descriptor
    };
    let result: T = flag;
    result
}
~~~

块内临时变量离开作用域后，已检查出的 Type 信息仍可随结果交付；它不携带 VM 存储。嵌套块和分支中的同名绑定各自独立。上例的 `observed_flag` 在每次 invocation 中执行一次，观察不调用它。结构观察不把赋值、控制转移或未知 Type 函数调用当作静态求值步骤；这些情况仍须由相应的编译期执行与类型检查路径处理。

受支持的不可变 `meta.List<T>` 也保留已知元素的结构。索引本身已知且在范围内时，取出的 Type 可以继续用于局部类型标注：

~~~flurry
comptime fn indexed_type() -> bool {
    let types: meta.List<Type> = [bool, u8];
    let T = types[0];
    let value: T = true;
    value
}
~~~

观察核对列表的完整类型身份，不将元组当作列表。嵌套列表和记录元素可以继续传递已知字段；其它元素尚待 invocation 求值时，不会抹掉所选元素已有的信息。列表和索引的原始求值仍按顺序执行，包括未选元素的初始化与失败。未知、可变、负数或越界索引不产生固定 Type 证据；这不把非法索引改成默认元素，也不展开取得的 normal-form 类型头。

分支返回同类型的列表时，共同已知的元素也可保留。例如两个分支分别返回 `[bool, u8]` 和 `[bool, u16]`，第 0 项仍是 `bool`，第 1 项则没有固定类型证据。此规则递归用于嵌套列表与记录字段，也允许一侧的其它字段尚待 invocation 求值。条件、match guard 和选中分支的初始化仍正常执行；观察不会执行未选分支。不同长度列表只保留所有分支都存在的前缀元素证据；这不截断实际列表，也不将前缀长度当成实际长度。

局部 const 也可保存 runtime 函数声明身份、固定的泛型函数实例，以及包含函数值的元组／记录；保存身份本身不调用该函数。闭包拥有的捕获环境可按当前值重建，包含调用后已更新的环境字段。图内同一闭包的多个引用及嵌套共享引用保持共享；分别构造的按值捕获闭包仍有独立环境。每次常量物化重新分配内部局部身份，并建立独立的环境快照。const initializer 中创建的闭包仍属于 comptime，后续 const 可调用它，但不能直接在 runtime 调用。外部活跃指针捕获与循环闭包对象图仍被拒绝。runtime 正文中的数据 const 已支持模式和 guard 的编译期选择；失败时进入 runtime 的发散 else 分支。quoted code 绑定仍要求无条件绑定。这些是实现边界，不缩减后文的完整阶段契约。

~~~flurry
comptime fn enabled(flag: bool) -> bool { flag }

fn configured(comptime flag: bool) -> bool {
    const found if enabled(found) = flag else { return false; };
    found
}
~~~

initializer、模式中的计算和 guard 均在编译期完成。模式失败时不执行 guard；只有模式及 guard 都成功才导出绑定，guard 对所持闭包环境的合法更新随成功绑定保留。编译期选择不能读取 runtime 参数或调用 runtime 函数。else 在原来的 runtime 控制流位置执行，不能访问当前声明的成功绑定，且必须发散；return、break 和 continue 保留原目标与作用域清理。即使本实例匹配成功，也检查 else 的类型和作用域。带普通 comptime 参数的 runtime 函数先检查定义，再对各实例执行选择。 定义检查保留 initializer 结构中已经存在的 Type 证据，包括 Tuple／record 中的 Type 字段，使成功绑定可用于后续类型注解；这不展开 normal-form 头，也不将缺乏证据的类型相等判断当作 false。

已知的 normal-form 类型也可以直接解构其输入。前一个类型输入为后续值参数提供域，成功导出的类型绑定可用于普通注解：

~~~flurry
pure comptime fn Descriptor<T, value: T> -> .Type'builtin { T }

fn configured_from_description(comptime flag: bool) -> bool {
    const Descriptor<T, value> = Descriptor<bool, true> else { return false; };
    let checked: T = value;
    checked and flag
}
~~~

这里匹配的是 Descriptor 的头和显式参数，不调用其函数体。嵌套 normal-form 和其中的 Tuple 类型模式按同样的已知结构传递候选证据；另一个头或未知结构不能提供这些证据。此实现不补齐任意未知 Type 主语的依赖模式推导。

普通 Type 常量也保留定义端证据，并可作为名义容器的成员访问接收者：

~~~flurry
struct Settings where T {
    pub typealias Value = T;
}

fn configured_from_alias(comptime flag: bool) -> bool {
    const Container = Settings<bool>;
    const T = Container.Value;
    let value: T = flag;
    value
}
~~~

成员仍按使用位置的可见性检查。普通数据字段访问不会因为字段与类型成员同名而改变含义。容器的值参数尚待 invocation 时，定义检查可以保留已检查的构造及其外层阶段输入，供成员选择使用；这不是已经完成的 Type，也不允许把 runtime 参数提升到编译期。

~~~flurry
fn invert_runtime(value: bool) -> bool { not value }

fn selected_callback() -> bool {
    const action = invert_runtime;
    action(false)
}

fn computed_with_closure() -> bool {
    const action = do { let enabled = true; |value: bool| enabled and value };
    const result = action(true);
    result
}
~~~

同一快照内的闭包状态更新可观察，而不会串到另一份快照：

~~~flurry
fn toggled_twice() -> bool {
    const actions = do {
        var flag = false;
        let toggle = || { flag = not flag; flag };
        (toggle, toggle)
    };
    const result = do {
        let (first, second) = actions;
        first() and not second()
    };
    result
}
~~~

两次调用操作同一闭包环境，所以结果为 true。若分别写两个 lambda，各自按值捕获 flag，它们不会因此共享 flag 的存储。通过外部指针建立共享状态属于另一种依赖，不能用上述快照替代。

当前实现支持 comptime 内部的临时标量指针：局部取址、字段指针投影、读写，以及同阶段函数和闭包间的指针传递。例如：

```flurry
comptime fn set(target: *bool, value: bool) {
    target.* = value;
}

comptime fn compute() -> bool {
    let cell = false;
    let pointer = cell.ref;
    set(pointer, true);
    cell
}
```

这里的 intrinsic 标量也包括仅编译时的任意精度 Integer 和精确 Float。它们可以在 comptime 局部存储中取址、读取和更新，不需要因为解释器使用大数表示就额外声明 Copy：

```flurry
typealias Integer = .Integer'builtin;
comptime fn increment(value: *Integer) { value.* = value.* + 1; }
comptime fn count() -> Integer {
    var value: Integer = 1000000000000000000000000000000000000000000;
    increment(value.ref);
    value
}
```

大数复制与运算继续消耗执行预算；这不允许 Integer、Float 或指向它们的指针作为 runtime 存储，也不允许临时存储指针逃逸到物化结果。

指针使用解释器的存储身份，不暴露宿主地址，也不持有存储所有权。离开作用域后访问该存储会在 comptime invocation 中报告失效指针；失败后的下一次 invocation 仍可正常执行。结果物化继续拒绝存储指针，包括闭包捕获中的指针，因此不能把临时存储带入 runtime 常量。intrinsic 标量及指针值可直接传送。聚合值的指针读取与替换要求同时绑定 Copy 和 Drop 协议，并取得具体的 Copy 能力、表示及清理合法性证据；不能仅凭字段都是标量便默认可复制。更一般的存储提升和非 Copy 传送仍未完成。这些支持不取消 pure、阶段边界或现有资源契约的检查。

编译期内置 `meta.List<T>` 的不可变数据表示另有专门的读写路径，不要求 runtime Copy。通过指针读取取得独立列表值，替换列表槽位不会改变先前读出的列表或已经传入遍历函数的输入。此路径核验可信 List 声明身份、元素域及完整的不可变数据准入条件；`List<Type>` 和嵌套列表也适用。读写及验证都受 invocation 预算限制，写入前的验证失败保留原槽位。这里不允许列表夹带存储指针，也不授予任何 runtime 复制能力。

聚合传送的资源证据使用独立、不可变的声明与布局快照，不把 Drop 等 runtime 方法体引入 comptime 执行包。嵌套字段逐项检查：外层声明 Copy 不能掩盖缺少 Copy 的子对象或非平凡清理责任。普通常量结果与 Type 结果的 invocation 都使用该证据，证明开销计入求值预算。未绑定协议或证据不成立时，受需的 invocation 拒绝执行传送；原始解释器也不会自行猜测协议身份。

包含 Type 字段的 schema 同样逐项检查。仅为 `struct Schema {word: Type}` 声明 Copy 不够；Type 自身也须有明确的 Copy 能力。两者的证据和清理检查成立后，可以在 comptime 通过指针复制整个 schema，复制保留字段的 Type 身份。先前单独取得的 Type 值不会随 schema 字段的后续修改而改变。这不授予 Type 自动 Copy，也不允许把 Type 或该 schema 带入 runtime 存储。

泛型 comptime 函数可以先形成 `p.*` 读取和写回，在 invocation 中使用实例化后的类型检查资源证据。例如 `comptime fn read(p:*T)->T where T :- Copy {p.*}` 不要求定义时就知道 T 的字段布局。类型约束不能替代具体的表示与清理检查；即使泛型外层满足 Copy，内部字段承担 Drop 责任时也不能复制。诊断保留证据失败原因，以区分协议未绑定、具体类型缺少 Copy 和清理责任不满足等情况。

证据查询直接消耗当前 invocation 的剩余预算，扣除后才执行传送和后续表达式。读取还需支付实际复制表示的开销。复制包含指针字段的聚合值保留这些指针的目标身份，不会复制目标存储或延长它的寿命；目标失效后仍不可访问，含指针的聚合值也不能借助 Copy 逃逸到物化常量中。

指针上的 BitVec 写回也可以在 comptime 执行。求值先固定目标指针，再按顺序计算选择范围和右值，最后读取仍需保留的位并写回；因此范围表达式重新绑定指针变量不会改变已选定目标，右值对原存储其它位的更新也不会丢失。空范围仍求值右值。宽度、边界、表示合法性和目标寿命继续检查，提前 return 不执行后续右值或写入。

## 语法

本节列出阅读入口；`expr`、`parameter`、`result`、`clauses`、`statement` 和 `block` 复用[全局语法](../grammar/syntax)。

```text
comptime_function -> pure? comptime fn id(parameter*) (-> result)? clauses? block
comptime_parameter -> comptime parameter
comptime_block -> comptime { (definition | statement)* }
inline_if -> inline if expr block (else block)?
defer_expr -> defer expr
```

`comptime` 修饰函数、参数和块时，分别约束函数的执行阶段、参数的可用阶段和块内计算。`defer` 保留尚待求值的表达式；其位置规则见[两阶段检查](./normalization-and-instantiation.md)。

## 正式定义

### 两个执行阶段、两个编译过程

| 名称 | 对程序的含义 |
|---|---|
| comptime | 计算在编译时发生，结果可以参与类型与代码的确定 |
| runtime | 计算由生成的程序执行 |
| normalization | 检查定义，利用已知输入完成相应编译时计算 |
| instantiation | 代入具体输入，继续编译时计算与检查，得到对应实例 |

normalization 与 instantiation 都是编译过程。runtime 函数的定义要经过它们；comptime 函数的定义与调用也要经过它们。不能把 normalization 等同于 comptime、把 instantiation 等同于 runtime。

一个表达式暂时算不出，可能只是缺少 comptime 输入。这不使它变成 runtime 表达式。反过来，即使实参写成常量，普通 runtime 函数也不会因此自动成为 comptime 函数。

### 操作与检查

ordinary 上下文要求定义端完成相应的合法性检查。检查依据包括参数约束、当前路径上的命题和先前操作提供的结果契约。loose 上下文允许指定检查留到实例化；它不免除最终检查。

`comptime` 函数与块采用 loose；在其中定义的 runtime 函数仍采用 ordinary。这些规则见[normalization 与 instantiation](./normalization-and-instantiation.md)。

### 跨阶段传递

运行时上下文中的子表达式没有 runtime 义务、且所需输入已知时，优先在 comptime 求值。结果实际进入 runtime 数据流时，遵守该类型的跨阶段规则。

comptime 参数在运行时调用约定中擦除。普通值的 `Splice` 转换则可以保留运行时调用；两者见[跨阶段值传递](./quoting-and-splicing.md)。

### 阅读路线

| 顺序 | 章节 | 主要问题 |
|---|---|---|
| 1 | [Normalization 与 instantiation](./normalization-and-instantiation.md) | 输入未知时能检查什么？defer 和 PTC 如何配合？ |
| 2 | [Comptime 函数、纯性与范式](./purity-and-normal-form.md) | 如何编写编译时计算？pure 承诺什么？ |
| 3 | [Comptime 块与 inline 控制流](./inlining-comptime-code.md) | 怎样在编译时决定分支和展开结构？ |
| 4 | [跨阶段值传递](./quoting-and-splicing.md) | comptime 值如何用于 runtime？ |
| 5 | [Trait 的 comptime/runtime 两相](./the-comptime-phases-of-traits.md) | 调用哪一相？开放定义如何检查？ |
| 6 | [案例：静态形状的张量加法](./static-shaped-tensors.md) | 形状、契约和元素计算如何协作？ |

[属性](./attribute.md)使用编译时表达式描述元数据。[编译流水线](../compilation/phases.md)给出编译过程的输入与输出。

## 常见示例

```flurry
pure comptime fn square(value: Integer) -> Integer {
    value * value
}

fn print_values() {
    println("{}", 1 + 23);
    println("{}", square(12));
}
```

第一行打印中的加法在编译时得到 24；第二行的 `square` 在编译时得到 144。打印操作仍在运行时发生。

```flurry
fn select_value(comptime first: bool, left: i32, right: i32) -> i32 {
    inline if first {
        left
    } else {
        right
    }
}
```

`first` 决定生成哪个分支；`left` 与 `right` 是运行时参数。编译时选择分支不需要读取这两个值。

## 与 Rust / Zig / C / C++ 的关键差异

| 对照语言 | 对应机制与差异 |
|---|---|
| Rust | `const fn` 可在常量上下文求值，也可像普通函数一样被调用；Flurry 的 comptime 函数属于独立执行阶段。[Rust Reference](https://doc.rust-lang.org/reference/const_eval.html#const-functions) |
| Zig | comptime 参数同样要求调用处取得编译时值；Flurry 还要求 ordinary 定义依操作契约完成相应检查。[Zig 语言参考](https://ziglang.org/documentation/master/#Compile-Time-Parameters) |
| C11 | 无对等的编译时函数机制；实际手段是常量表达式、宏和外部代码生成。[C11 草案 §6.6、§6.10](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf) |
| C++ | `constexpr` 函数可参与常量或运行时求值；Flurry 的 comptime 函数只属于 comptime 阶段。[C++ 工作草案](https://eel.is/c++draft/dcl.constexpr) |

## 运行时实现设计引导

生成的程序需要满足以下边界：

1. 接收实例的 runtime 参数，不接收已擦除的 comptime 参数。
2. 执行实例中保留的控制流、调用与数据操作。
3. 需要普通值跨界转换时，执行对应 `Splice` 方法的 runtime 实例，或执行保持其行为的优化结果。

不变量：运行时不承担“重新猜测编译时类型”的工作；必需的编译时检查失败时，不得交付依赖该失败结果的实例。运行时代价由保留的操作决定，不能仅凭源码含 comptime 就断言为零。

验证 runtime 实例时，函数体与实例键必须一起取得。普通 `comptime` 输入可能已经从残余签名中消失，但仍属于实例身份；不能把这些输入重新当作原始模板的子句参数代入。当前 cleanup 验证视图保留按声明及完整实例参数索引的残余函数体，内存摘要和回调结果分析使用同一查找规则。实例身份不会因为参数已擦除就合并，也不会因此展开 normal-form 类型头。

## 编译器实现引导

编译器应分别记录三个问题：表达式属于哪个执行阶段、它的合法性检查是否完成、它的具体值是否已取得。

1. 依据定义与上下文确定阶段及 ordinary/loose 检查强度。
2. 用当前契约检查操作；能够求值的 comptime 部分参与计算。
3. 保存依赖未知输入的计算，以及后续检查可用的结果契约。
4. 实例化时提供输入，继续计算和检查，输出完整的 runtime 实例或 comptime 结果。

检查通过不等于操作已经执行；一个具体值未知也不等于所有下游检查都必须等待。详细的 query 算法与失败处理见[两阶段检查的编译器引导](./normalization-and-instantiation.md#编译器实现引导)。

代码值的完整规则见 [CMTT](../cmtt/index.md)：代码引用与普通值的 Splice 分别检查，quoted expr、pattern 和 arm 保留各自上下文及接入契约。

### 泛型浮点字面量的形成阶段

泛型函数可用类型约束说明字面量的目标域。下面同一份十进制字面量既能形成精确的编译时 Float，也能直接形成指定宽度的机器浮点值：

```flurry
comptime fn decimal() -> T
where T, requires T == .Float'builtin or T == .f32'builtin or T == .f64'builtin {
    1.0000000596046447753906250000000001
}

fn run() -> bool {
    const exact = decimal<.Float'builtin>();
    const rounded = decimal<.f32'builtin>();
    const precise = exact == 1.0000000596046447753906250000000001;
    precise and rounded'bitvec == 1065353217
}
```

编译器保留精确十进制及符号，直到实参 T 确定后再形成值；f32 不经过 f64 中转，因此避免二次舍入。负零的符号也在形成过程中保留。类型约束必须证明目标属于允许的浮点域；无约束 T、包含 bool 的联合域以及仅凭函数体推测表示的 normal-form 头均不足以接纳字面量。

精确 Float 的阶段限制不因此改变：它可以在 comptime 中形成、比较和返回，不能作为普通 runtime 函数的结果存储。机器浮点实例的范围检查独立进行；某个 f32 实例溢出，不会使同一泛型函数的合法 f64 实例失效。
