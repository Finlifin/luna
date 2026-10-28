# 模式匹配

## Intro

模式描述如何拆开、识别一个值。

若干模式与表达式形态成对出现：`pat?` 对偶于 option 的消除/传播，`pat!` 对偶于错误成功值一侧，`error pat` 对偶于 `error expr` 构造。多臂匹配须穷尽（或用 `_`）；单臂 `is` 不强制穷尽。自定义匹配视图通过 **`case`** 声明；控制流形态见 [基本控制流](../basic.md)。

## 语法

```text
-- 使用处（与 basic 一致）
if_is         -> if expr is pattern do block (else ...)?
if_is_arms    -> if expr is do { case_arm* }
post_match    -> expr match { case_arm* }
while_is      -> while label? expr is pattern do block
while_is_arms -> while label? expr is do { case_arm* }
case_arm      -> pattern => (block | statement)

let_decl      -> let pattern (: type_expr)? = expr (else block)?
var_decl      -> var pattern (: type_expr)? = expr (else block)?

-- 模式（局部；与 grammar 对齐的增量以确认后的为准）
pattern -> ... | (pattern) | pattern? | pattern! | error pattern
         | pattern or pattern
         | pattern and expr is pattern
         | pattern as id
         | pattern if expr
         | pattern (pattern*)
         | pattern { ... }
         | list / range / bitvec / symbol / literal / id / _

-- case 定义（常见于 struct / impl 体内）
case_def -> pub? case id (parameter*) -> expr clauses* block
```

`parameter` 常见为 `self` 或 `*self`。`clauses` 为 `where` 等子句。`case_arm*` 等列表的分隔符见 [语法风格：分隔符](../../grammar/index.md)（臂用逗号；语句用分号；**仅当项的最后 token 为 `}` 时**省略其后分号）。

## 正式定义

### 使用位置与穷尽性

| 位置 | 说明 |
|------|------|
| `if expr is pattern do block` | 单模式；失败可走 `else`；不强制穷尽 |
| `if expr is do { case_arm* }` | 多臂；须穷尽或含 `_` |
| `expr match { case_arm* }` | 同上，后缀写法 |
| `while … is … do` | 见基本控制流 |
| `let` / `var` | 左侧为模式（可解构）；可接 `else`（见下） |

### `let` / `var` 解构与 `else`

`let` 与 `var` 的左侧都是**模式**，因此可以直接解构：

```flurry
let (x, y) = position;
var { name, age } = student;
```

当模式**可能失败**（可反驳）时，可在末尾接 `else` 块，规则**类比 Rust 的 `let … else`**（此处引用 Rust 仅为锚定行为描述，不构成对 Rust 语义的承诺）：

- 写法：`let pattern = expr else block` / `var pattern = expr else block`
- 模式匹配成功：绑定引入后方作用域，继续执行。
- 匹配失败：进入 `else` 块；该块必须**发散**（如 `return` / `break` / `continue` 等，使控制流不再回到「使用这些绑定」的后方代码）。

不可反驳的模式（如单纯标识符、穷尽的元组拆开）不必写 `else`。

### 基本模式

- **字面量**：与值相等则匹配。 数值字面量可带负号，例如 `-3`、`-3.5`；`(-3) or -4` 与 `-3..= -1` 分别使用普通的替代模式和范围模式规则。负号只接数值字面量，不引入任意表达式模式。字面量扩展仍遵循对应协议。
- **标识符**：绑定到该值。
- **`_`**：匹配且不绑定。
- **符号**：`.ok` 等，见字面量章。
- **元组 / 列表**：按位置拆开；列表可用 `...rest` 收尾。
- **结构体 / 记录**：`{ field: pat, … }` 或缩写字段。
- **应用**：`pat(pat*)`、`.variant(pat*)`、`.variant { … }` 等，匹配变体或可应用头。
- **范围**：`a..b`、`a..=b`、`..=b`、`a..` 等（与表达式范围形态对应的模式侧）。
- **`or`**：`p1 or p2`；两侧绑定结构须兼容。
- **`and is`**：`pat and expr is pat`（可连锁）；先完整匹配左侧，再对 `expr` 继续匹配。
- **`as`**：`pat as name`，匹配 `pat` 同时把整体绑到 `name`。
- **守卫**：`pat if cond`；`cond` 为真才成功（模式层用 `if`，与语句守卫 `while` 不同）。

范围模式支持 f32 和 f64，并使用与普通比较表达式一致的有序比较。`lower..upper` 要求 `lower <= value and value < upper`，`lower..=upper` 的上界改为 `value <= upper`；单侧范围只检查所写的一侧。主语或被检查的端点为 NaN 时，相应比较失败。右开上界不能实现为 `not (upper <= value)`，因为 NaN 下两者不同。正负零按比较规则相等，无穷值仍受开闭边界约束。

~~~flurry
typealias Float = .f64'builtin;

fn in_unit_interval(value: Float) -> bool {
    value match {
        0.0..1.0 => true,
        _ => false,
    }
}
~~~

`0.0` 与 `-0.0` 匹配，`1.0` 与 `Float.NaN` 不匹配。计算型端点按源码顺序各求值一次；端点中的提前返回仍遵循普通控制流，不执行其后的端点。取反反转完整匹配结果，因此 `not (0.0..1.0)` 会接纳 NaN；嵌套取反逐层执行。仅用 `..0.0` 与 `0.0..` 不能覆盖浮点域，NaN 仍未匹配。NaN 常量的相等模式也不能补上这一缺口，因为 NaN 不等于自身；可使用 `_` 明确覆盖剩余值。对常量端点、纯比较和其布尔组合（包括有完整 else 分支的布尔 if），穷尽性分析保留 NaN 类别，可以证明范围与其取反共同覆盖整个域，也保留元组字段之间的对应关系。函数调用或其他尚不能符号分析的计算不会作为覆盖证据；运行时能够匹配不代表任意计算型端点均能被静态证明。

也可以用纯比较区分有序数值和 NaN，不需要用 NaN 常量做相等匹配：

~~~flurry
fn is_ordered(value: Float) -> bool {
    value match {
        _ as number if number == number => true,
        _ as number if number != number => false,
    }
}
~~~

分析只利用比较行为，不把正负零的表示位或不同 NaN 的载荷当作相等；这些表示差异仍可由其他合法操作观察。这里的覆盖证明不执行用户代码，也不交给 PTC。整数、字符、精确 comptime Float 及 f32/f64 的覆盖分析也保留纯守卫中局部布尔 let 的真值证据：别名读取、not、布尔结果之间的 == / != 和条件分支继续使用绑定初始化时的证据，再与当前分支相交；同名遮蔽不会改变先前绑定的含义。未知函数计算仍不能通过先保存到局部变量来变成覆盖证据。整数、字符、精确 Float 和 f32/f64 的纯守卫还可按布尔结果继续 match：每个 arm 只分析此前未匹配的情况，模式绑定保留这个布尔结果的证据；arm 的守卫失败时，相应情况继续交给后续 arm。精确 Float 还可对原数值主语进行这种嵌套决策。数值常量也可作为嵌套 match 的主语；其中的模式绑定代表该常量，不能与外层匹配值混同。f32/f64 的分析也允许短路、if 和顺序 match 跳过未执行的普通布尔函数调用，但可达调用仍保持未知；`x < 0.0 or x >= 0.0` 不能据此跳过 NaN 路径上的调用。这些未知调用的实参还可包含 tuple、结构体、枚举、匿名记录、newtype 和函数声明身份；构造字段递归保留，不能因实参是聚合值就提前否定已经由短路排除的路径。分析不执行或推导被调用函数的正文，NaN 等仍能到达调用的路径保持 Unknown；复制输入结构同样消耗分析预算，其他未支持的调用结构仍保守拒绝。这是覆盖分析的能力；guard 实参的资源交付和失败路径仍须另行检查。临时构造的无指针聚合实参已接入观察区域检查；消费候选主语、外部所有者或可能拒绝的清理仍不因此获得许可。精确 Float 分析允许短路、if 或顺序 match 跳过未执行分支中的未知条件；一旦分析路径实际到达未知条件，就不能据此证明真或假，也不能借取反或自相等绕过。分支按顺序选择，后面的匹配不能覆盖前面已选中的结果；若分析发现嵌套决策没有匹配的 arm，则无法给出证明，不能当作 false 再取反。

整数和字符的区间分析也支持顺序计算模式：左侧匹配产生的绑定可用于纯比较，计算得到的布尔值可继续匹配或交给守卫。例如：

```flurry
typealias i32 = .i32'builtin;

fn nonnegative(input: i32) -> bool {
    input match {
        value and (value < 0) is true => false,
        value and (value >= 0) is true => true,
    }
}
```

两个分支分别覆盖负数与非负数；把第二个比较改成 `value > 0` 会留下零的缺口。分析中的计算结果始终受左侧已匹配范围约束，不能通过取反扩大到当前候选之外。只使用可分析的主语、常量和纯布尔计算；未知调用不能充当覆盖证据，编译器不会为了证明覆盖而调用它。

comptime Float、f32 和 f64 也支持这种顺序计算。机器浮点的比较仍然保留 NaN 路径，不能把“小于零或大于等于零”当作整个浮点域：

```flurry
typealias Number = .f64'builtin;

fn nonnegative_float(input: Number) -> bool {
    input match {
        value and (value < 0.0) is true => false,
        value and (value >= 0.0) is true => true,
        value and (not (value == value)) is true => false,
    }
}
```

第三分支覆盖 NaN；正负零进入第二分支。计算结果也可绑定后用于守卫，或把浮点主语继续交给后续数值模式。继续利用先前的数值绑定推导守卫时，即使计算后的模式是 `_`，可达的未知布尔计算也不能被当作无影响的步骤跳过；短路确实跳过的未知分支则不妨碍证明。 同样，守卫中的局部 let 初始化和嵌套 match 主语必须保留求值顺序：`do {let unused=false or decide(x);true}` 与 `(false or decide(x)) match {_=>true}` 都不能通过丢弃结果来隐藏可达的未知调用。将其中的 `false or` 改为 `true or` 才会实际跳过调用。

精确 Float 的 or-pattern 还可由不同候选计算同名布尔绑定，再交给外层守卫：

```flurry
typealias Float = .Float'builtin;

comptime fn nonnegative_exact(input: Float) -> bool {
    input match {
        (((value if value < 0.0) and false is selected) or
         ((value if value >= 0.0) and true is selected))
            if selected == (value >= 0.0) => selected,
    }
}
```

分析保留首次成功候选与导出值的对应关系，也支持嵌套 or-pattern。外层守卫失败不会重试下一个候选；若把第一候选的范围条件移除，正数会先得到 `selected=false`，守卫失败后整个匹配并不穷尽。

### 观察模式与整体绑定的所有权

模式只检查字面量、变体或不绑定的字段时，不会凭空取得字段的第二份所有权。`as` 可以在检查成功后交付整个仿射值：

~~~flurry
struct Packet where T {
    ready: bool,
    payload: T,
}

fn retain(packet: Packet<T>) -> Packet<T> where T {
    packet match {
        {ready: true} as whole => whole,
        whole => whole,
    }
}
~~~

第一个候选只观察 `ready`，没有绑定 `payload`。成功时移动整个 Packet；失败时下一个候选仍接收完整 Packet。带外层守卫的候选同样只在守卫成功后提交交付。若整个值未交付而需要清理，仍保留父对象及实际载荷的清理责任。

这也适用于没有内部绑定的 `or`，例如 `(.one(true, _) or .two {flag: true}) as whole`。相反，`{payload: item} as whole` 同时要求内部载荷与完整对象，必须另外证明允许重叠交付，不能由 `as` 自动获得 Copy。

当额外绑定的字段具有 Copy 能力时，可以复制该字段，同时移动整个仿射对象：

~~~flurry
fn inspect_ready(packet: Packet<T>) -> Tuple<bool, Packet<T>> where T {
    let {ready} as whole = packet;
    (ready, whole)
}
~~~

这里只要求 `ready` 的 bool 值可以复制，不要求 `T` 或整个 Packet 可以复制。Packet 的字段没有被移出，父对象的析构函数仍可读取原来的字段；复制指针也保留对其所指对象的依赖。嵌套 `as` 中，每个额外取得的值分别检查 Copy；仿射载荷不能借多层绑定绕过此要求。当前所有权实现已接通这种额外复制与整体交付，并在外层守卫失败时保留原主语的责任。

带绑定的 `or` 在每个候选中分别检查模式，再将成功候选的名字对应到同一组导出绑定。例如：

~~~flurry
fn unpack(packet: Packet<T>) -> T where T {
    packet match {
        {ready: true, payload: item}
            or {payload: item, ready: false} => item,
    }
}
~~~

两个候选的字段书写次序不同，但 `item` 都来自同一个 `payload` 位置。前一候选失败不会移动该载荷；后一候选成功后只交付一次。外层守卫失败时，同样不提交这组绑定。

当前所有权实现也支持在同一分解布局中选择不同字段的仿射交付，例如：

~~~flurry
struct Ticket {}

fn take(value: Ticket) {}

fn choose(flag: bool) {
    let pair = (flag, Ticket {}, Ticket {});
    pair match {
        (true, item, _) or (false, _, item) => take(item),
    };
}
~~~

成功候选记录 `item` 实际来自哪个字段，守卫成功后才提交移动。剩余主语按实际初始化状态清理；不会把未选中的 Ticket 一起移走，也不会重复清理交付给 `take` 的 Ticket。具名字段、嵌套结构、嵌套 `or` 和 `let … else` 使用同一机制。失败候选撤销绑定及其字段来源，外层守卫失败时仍可用完整主语尝试下一 arm。

不同枚举变体也可以交付同一个仿射绑定，即使载荷布局不同：

~~~flurry
struct Ticket {}

enum Packet {
    pair(Ticket, Ticket),
    tagged(bool, Ticket, Ticket),
}

fn take(value: Ticket) {}

fn choose(value: Packet) {
    value match {
        .pair(item, _) or .tagged(_, _, item) => take(item),
    };
}
~~~

`.pair` 移走第一个 Ticket，`.tagged` 移走最后一个 Ticket。两种剩余布局分别通过所有权和 Drop 契约检查，执行时按实际标签选中唯一清理计划。父对象先清理，再按该布局清理剩余字段；交付出去的 `item` 由其新所有者负责。嵌套枚举从外层标签开始检查，未选中变体中的字段路径不会被访问。同一标签内部还可以继续使用按初始化状态判断的条件清理。

`(.one(_) as whole) or (.two {} as whole)` 交付的都是整体，可以继续共享计划。不能由互斥标签区分的不同残余布局，以及一般副作用候选计算和具有独立资源的顺序匹配结果仍需进一步实现；这些是实现边界，不是对 `or` 语言语义的新限制。条件结构分析有候选数量与燃料预算，运行时仍执行原始模式，不重放观察表达式。

候选也可以按不同次序引入同一组仿射绑定：

~~~flurry
fn inspect(value: Tuple<bool, Ticket, Ticket>) {
    value match {
        (true, first, second) or (false, second, first) => (),
    };
}
~~~

第一候选成功时，退出 arm 先清理 `second`，再清理 `first`；第二候选成功时则先清理 `first`，再清理 `second`。名字对应到共同绑定，不会把成功候选的声明次序也改成第一候选的次序。已经移动的绑定跳过，覆盖赋值保留该绑定原有的清理位置；其他词法绑定与残余主语的清理位置不变。

所有可能的清理次序都必须满足内存依赖。若 `second` 的析构要访问 `first`，却存在先清理 `first` 的成功候选，编译器不能靠交换析构来使程序通过。解释执行记录成功候选的绑定次序；失败候选及失败守卫不安装这份记录，调用和循环分别保持各自的有效记录。

守卫可以调用函数来判断尚未提交的绑定中的标量字段：

```flurry
fn accept(ready: bool) -> bool { ready }

fn inspect(packet: Packet<T>) -> bool where T {
    packet match {
        item if accept(item.ready) => true,
        _ => false,
    }
}
```

这里沿用前文具有 `ready: bool` 字段的 Packet。`accept` 的实参只复制字段，返回 false 时原 Packet 仍由后续候选接收；调用不会重放，短路表达式仍保持短路。callee 自己创建的资源按正常调用规则清理。当前实现支持直接函数及已实例化函数、标量和具备 Copy 证据的代码实参与结果、合法默认实参、已知代码指针调用及嵌套调用；调用仍经过普通调用检查。指针实参还可用于具有已知来源的只读调用：先检查实参表达式本身，再检查选定 callee 的内存摘要，拒绝写入调用者内存、未验证清理和可能发生规格拒绝的路径。借条件表达式选取地址也不能隐藏写操作。消费外部仿射值、未知回调、带环境闭包以及一般可拒绝调用仍需完整的候选失败路径资源事务，当前明确拒绝。这是实现范围，不是语言对守卫表达式的永久限制。

只观察绑定的守卫也可位于 `or` 的内部或单个字段上。括号明确划定候选范围：

```flurry
fn inspect_packets(value: Tuple<Packet<T>, Packet<T>>) -> bool where T {
    value match {
        ((first, second) if accept(first.ready)) or
        ((second, first) if accept(first.ready)) => first.ready,
        _ => false,
    }
}
```

第一候选的守卫失败后，撤销它的绑定及来源，再按第二候选自己的名字环境求值。这里第二次 `first` 来自原 tuple 的第二个位置；统一候选绑定名字不能改变该读取。成功后仍按实际绑定取得次序逆序清理。字段守卫、`as whole` 内部的 Copy 观察和 `let-else` 复用此规则；守卫中的仿射消费仍不能绕过失败路径检查。

tuple 模式按字段顺序建立候选绑定，后续字段中的守卫和顺序计算可以读取前面字段的绑定。嵌套 tuple 继承外层此前的绑定；前向引用不成立，同一完整模式也不能重复声明同名绑定。

枚举的位置载荷模式遵循相同规则：`.pair(first, (second if first == second))` 的第二个字段可以观察 first。嵌套 tuple 与嵌套变体继续使用此前已建立的候选绑定；整段模式失败后，这些名字不会进入下一候选或 let-else 的 else 分支。or-pattern 的每个候选独立建立字段绑定，成功后才统一导出名字。该作用域规则也适用于 comptime 生成器的匹配，不因阶段不同而改用外层同名绑定。

```flurry
fn same_pair(value: Tuple<bool, bool>) -> bool {
    value match {
        (first, (second if first == second)) => true,
        _ => false,
    }
}
```

这些名称仍属于当前候选。匹配失败时不泄露到下一 arm 或 let-else 的 else 分支；它们也不遮蔽初始化表达式中的旧绑定。候选观察和仿射资源限制保持不变。

结构体、匿名记录和枚举命名载荷也可以在后续字段中观察此前绑定：

```flurry
struct Pair { first: bool, second: bool }

fn same_record(value: Pair) -> bool {
    value match {
        { first: a, second: (b if a == b) } => true,
        _ => false,
    }
}
```

当前实现先按名字定位字段，再按类型的字段顺序检查和执行模式。名字在源码中从前向后可见，因此字段 guard 所引用的绑定还必须在实际匹配顺序中已经建立。仅将字段写到前面不会提前执行它；例如 `{ second: b, first: (a if b) }` 会被拒绝。省略字段相当于通配符，不建立绑定。此处仍保留候选失败时的资源观察约束，不能借字段 guard 提前消费原值。


顺序匹配可以使用前一步的绑定计算新的标量，并继续解构或检验该结果：

```flurry
fn flip(value: bool) -> bool { not value }

fn inspect_computed(packet: Packet<T>) -> bool where T {
    let item and flip(item.ready) is inverted
        and flip(inverted) is restored = packet;
    restored and not inverted
}
```

`item` 接收原 Packet；`inverted`、`restored` 来自各自的计算结果，不是 Packet 中被移出的字段。只有左侧匹配成功后才执行下一段计算；右侧匹配失败时撤销整个候选的绑定，下一个候选仍接收原主语。链式计算、`or` 内的顺序匹配和 `let-else` 已接入资源检查；运行时保持原始模式树与单次求值。当前中间结果支持内建标量及具有 Copy 依据的无捕获环境代码值，计算遵循上述观察表达式规则。带拥有型资源、指针或副作用的中间结果仍需独立的失败路径清理与来源运输，当前明确拒绝。

布尔主语的覆盖分析会关联前项绑定、计算结果和后项模式。例如 `flag and not flag is true` 与 `flag and not flag is false` 共同穷尽 bool。多个候选由 `or` 连接后再附加 guard 时，guard 使用首个成功候选导出的绑定；guard 失败不会回头尝试同一个 `or` 的后续候选。未知调用的计算结果仍不能用于精确覆盖证明。

数据主语的顺序匹配也参与函数内存摘要：计算表达式中的读取与调用足迹保留下来，计算绑定按自身结果类型建立，不继承主语字段的来源。`let-else` 的失败分支使用匹配前的环境，其返回指针来源与成功路径一并汇总。摘要不替代候选观察规则和仿射检查，也不会在运行时重放候选计算；含数据指针的聚合主语仍需独立的来源运输契约。含可调用值的聚合使用另一条身份传播路径：枚举字段以 variant 加字段位置区分，match、or 和 let-else 解构保留具名代码及已知闭包捕获。不同 variant 中不存在的字段与来源未知的实际字段分别处理；未知回调不能借某个空分支获得调用权限。 资源检查向内存求解器导出这些证据时同样保留 variant 路径，包括匿名回调载荷及闭包中再次捕获的枚举。调用点仍以真实所有者核验读写足迹和返回别名，已移动的所有者不会因包装进枚举而恢复可用。

闭包可以把捕获的枚举从一个 variant 替换为另一个包含固定回调定义的 variant。摘要保留初始的不活动载荷事实，并汇总后续调用可能遇到的回调身份；实际运行只调用当前 variant 的回调。不活动载荷的空来源在 dump 中显示为 `(absent-variant)`，它必须来自构造或已经验证的枚举标签，不能用来表示未知回调。来自本次调用输入的动态回调替换仍需要环境状态转移契约，不能被当作可跨调用复用的固定定义。 对可恢复的 Copy 捕获，递归工厂返回可更新闭包时，可以使用经过固定点检查的跨调用候选集合重建代码证据；它不会把已经执行过的闭包恢复为构造时状态。固定定义捕获纯数据枚举时，标签和字段值按运行时数据抽象，所有可能的读取目标仍进入摘要。


候选计算也可以用块组织局部步骤：

```flurry
fn inspect_block(packet: Packet<T>) -> bool where T {
    let item and do {
        let original = item.ready;
        var result = not original;
        result = not result;
        accept(result)
    } is true = packet else { return false; };
    item.ready
}
```

块中的标量绑定只在块内有效；块结束后，中间值由后续模式接收。局部声明、嵌套块、分支、标量调用和当前块局部变量的赋值已接通。失败候选不会留下这些局部变量；后续候选执行自己的计算，不会为了构造清理计划而重放前一候选的步骤。当前不允许这种观察块写入外部变量、建立活动数据指针绑定、消费候选所有者或向外层 return；这些操作需要一般候选事务承担失败路径责任。块内私有的无指针资源可建立并交付，前提是调用与清理均有不拒绝依据。

guard 内的构造和属性调用按普通表达式解析，例如 `_ if Probe { flag: value }.flag`，无需为避免被当作模式而额外加括号。这也适用于 let-else、if-is 的 guard 和 inline for 的过滤条件。解析仍保留外层语句的大括号边界及 if-is 的 `do` 分隔符；确有边界歧义时可用括号明确表达式范围。

guard 的直接调用也可交付本次表达式构造的拥有型实参，不要求额外包一层 `do`：

~~~flurry
struct Probe { flag: bool }

fn inspect(probe: Probe) -> bool { probe.flag }

fn choose(value: bool) -> bool {
    value match {
        _ if inspect(Probe { flag: value }) => true,
        _ => false,
    }
}
~~~

临时聚合结果的字段也可以直接用于观察，例如 `make_probe().flag` 或 `inspect_bool(make_probe().flag)`。工厂结果仍有完整的 owner 和清理责任，取出标量字段不会删掉 owner。复合条件 `left().flag and right().flag` 按一个完整表达式保留临时责任：左侧为 false 时不调用 right；两侧均求值时，先完成两次观察，再按逆构造顺序清理，最后进入选定分支。直接写法与包在 `do` 中的写法、内联展开前后均保持这个顺序。

相同的临时观察规则也覆盖 `make_flag().as(bool)` 和 `make_choice()'tag_name`。newtype 按既定转换规则交付表示值；读取枚举名称只取得名称，枚举载荷仍按临时 owner 的责任清理。它们嵌入 `and`／`or` 条件时继续保留完整表达式边界。临时包装不会授予外部绑定 Copy：对已有非 Copy 值先包装、再解包，仍不能绕过候选失败后必须保留该值的要求。

实参可以是构造值、工厂结果、默认实参或其 tuple 包装。调用前完成普通实参准备，调用结束后按责任清理；guard 成功与失败都不会重放这些步骤。观察检查保护进入候选时已有的资源，因此不能将候选主语或外部拥有者移动进实参，即使 guard 最终返回 true。工厂、被调用目标及析构若可能拒绝，仍需一般失败路径契约，当前明确拒绝；内联展开前后遵守同一规则。

这些标量调用和局部计算块也可用于 guard 的内存摘要。调用仍取得普通函数契约，参数中的指针读取计入足迹；块内赋值只允许修改该块声明的标量变量。摘要保留调用可能拒绝的事实，是否允许它出现在可回退候选中仍由资源检查决定，不能用摘要成功绕过失败路径检查。

观察块还可建立和改写块内私有的 Copy 代码绑定。guard 可直接调用具有已知具名目标的 Copy 代码值；若分支合并了多个目标，每个目标都必须满足观察契约且不会拒绝。未知目标、带捕获环境的调用，以及尚未取得不拒绝依据的目标仍不能借这一入口绕过失败路径交付。

```flurry
trait Copy {}
typealias Action = fn(bool) -> bool;
impl Copy for Action {}

fn invert(value: bool) -> bool { not value }
fn produce() -> Action { invert }

fn inspect_code() -> bool {
    false match {
        _ and produce() is action if action(false) => action(false),
        _ => false
    }
}
```

这里先调用一次 `produce`，再在 guard 中调用一次返回的 `action`。守卫成功才执行分支体中的第二次 `action` 调用；失败时保留已经执行的观察及临时清理记录，撤销候选绑定后继续匹配。

代码身份也可以来自工厂调用、条件表达式或局部块的结果。因此 guard 可以直接写 `produce()(false)`，也可以将 `produce()` 作为已知高阶函数的代码实参。检查使用实际结果的身份；工厂本身不拒绝，并不代表它返回的目标不会拒绝，两层都需检查。临时被调用值在本次观察表达式的边界完成清理，工厂不会因契约检查而再次执行。

观察计算内部还可使用 `match` 选择标量或 Copy 代码结果。每个可达分支在自己的模式绑定环境中检查；内层失败候选的计算照常执行，成功分支才交付结果。例如，沿用上面的 `Action`、`produce` 和 `invert`：

```flurry
fn unchanged(value: bool) -> bool { value }

fn inspect_choice(flag: bool) -> bool {
    false match {
        _ and (flag match {
            true => produce(),
            false => unchanged
        }) is action if action(false) => true,
        _ => false
    }
}
```

`flag` 为 true 时调用一次 `produce`，再调用它返回的 `invert`；为 false 时直接调用 `unchanged` 并进入外层兜底分支。内层 match 不允许借分支隐藏外部写入、拒绝或向外层返回。当前该观察入口的主语和结果限于内建标量及 Copy 代码值；带拥有型资源的主语仍需一般候选事务处理。

观察块内还可使用无 `else` 的局部模式绑定，复用前端已经检查的不可失败性。模式计算的结果可以继续用于下一段计算或作为块结果交付：

```flurry
fn inspect_local_pattern() -> bool {
    false match {
        _ and do {
            let _ and produce() is action = false;
            action
        } is next => next(false)
    }
}
```

这里的 `action` 只在观察块内存在，`next` 接收块交付的 Copy 代码。局部模式计算仍受观察规则约束，不能通过它写外部变量；观察块中的 `let-else` 及其向外层退出仍需一般失败路径契约。普通 `let-else` 已执行的模式计算和 guard 记录则必须保留，即使 guard 恒假，也不能连同成功绑定计划一起丢弃。

观察调用可以省略具有合法默认值的标量或 Copy 代码参数。显式实参先在调用者环境中准备，默认值再按声明顺序在独立的形参环境中求值，只能读取已经准备好的参数：

```flurry
fn opposite(value: bool) -> bool { not value }
fn select(value: bool,
          .second: bool = opposite(value),
          .third: bool = opposite(second)) -> bool {
    third
}

fn inspect_defaults() -> bool {
    false match {
        _ if select(true) => true,
        _ => false
    }
}
```

这里两次 `opposite` 按参数声明顺序各执行一次。显式指定 `.second` 会跳过它自己的默认表达式，`.third` 随后读取实际传入的值。默认值仍须通过 P3 检查；目标调用是否可能拒绝继续单独检查。默认回调的代码身份属于该次默认求值，显式传入未知回调时不能沿用被跳过默认值的依据。

后续字段不匹配不能取消前面已经执行的计算模式。例如：

```flurry
struct Pair { left: bool, right: bool }

fn inspect_pair(gate: *bool) -> bool {
    Pair { left: true, right: true } match {
        { left: _ and gate.* is _, right: false } => false,
        _ => true,
    }
}
```

第一候选先读取 `gate.*`，随后 `right: false` 不匹配，最终进入第二候选。编译器仍须保留前一次读取的内存安全义务。这个顺序也适用于枚举标签不匹配、主语先保存到局部变量以及 guard 失败。已知不可能执行的分支体可以排除；纯结构匹配本身没有计算步骤时，也可利用后续字段的确定不匹配剪枝。

所有导出绑定均可 Copy 时，普通 `or` 的候选也可以从不同位置复制，不要求外层 `as`：

~~~flurry
enum Signal where T {
    left(bool, T),
    right(T, bool),
}

fn enabled(value: Signal<T>) -> bool where T {
    value match {
        .left(flag, _) or .right(_, flag) => flag,
    }
}
~~~

这里两个 `flag` 的位置不同，但各自都是 bool 副本。当前实现分别检查候选，按对应绑定合并指针来源、回调信息及可用性；不会把一个候选的来源当成所有候选的来源。主语的实际变体及完整载荷保留到对应清理边界，因此不需要把父对象拆成两种残余布局。需要把整个主语另行交付时，仍可在外层加 `as whole`。仿射载荷使用前述条件移动与残余清理计划；不会沿用 Copy 路径来免除移动责任。

### 匿名结构体的字段模式

属性收集产生的匿名结构体也可以按字段名解构。模式中的字段顺序不必与构造顺序相同，省略字段不参与匹配；字段交付与剩余资源清理仍遵循普通移动规则。

~~~flurry
fn build(...properties: T) -> T where T { properties }

fn read() -> bool {
    let options = build {enabled: true, nested: build {value: false}};
    let {nested: {value}, enabled} = options;
    enabled and not value
}
~~~

嵌套字段可以继续使用字面量、绑定或其他模式。未知字段、重复字段以及未覆盖全部可能输入的 match 都会被拒绝。

### 模式优先级与括号

优先级决定模式的组合范围，不决定 match 各个 arm 的尝试顺序。下表由紧到松排列；同一层连续出现时按从左到右接合，括号先建立一个完整子模式。

| 结合层次 | 形式 | 组合方式 |
|---|---|---|
| 名称投影 | E.variant | 先确定完整模式头 |
| 应用 | P(...)、P{...}、E<...> | 先组成载荷或类型模式 |
| 成功包装 | p?、p! | 作用于左侧已形成的模式 |
| 选择 | p or q | 两侧分别检查，成功绑定须对应 |
| 整体绑定 | p as name | 绑定左侧整个模式匹配的值 |
| 守卫与顺序匹配 | p if expr、p and expr is q | 接在已经形成的左侧模式之后 |

因此 1 or 2 as value 解释为 (1 or 2) as value；n? as whole 中 n 接收内部值，whole 接收整个成功的 option。守卫后面的 expr 按表达式规则解析，不能把其中的 and、or 当成模式选择。

前缀 not、error、ref、async 取得当前范围内其后的完整模式。范围端点同样按当前语法边界读取；范围与其他组合相接时，用括号明确完整范围，例如 (10..20) as value、(10..20) or 30。前缀或范围不能仅凭“看起来像表达式运算符”套用表达式的结合方式。

括号划定模式的范围，与表达式分组作用相同。它不新增解构层：

| 形式 | 含义 |
|---|---|
| (p) | 以 p 匹配原值 |
| (p,) | 单元素元组模式 |
| (p, q) | 两元素元组模式 |
| () | unit 模式 |

例如 (1 or 2)? 将选择放在 option 内部；1? or 2? 分别匹配两个存在分支。not (1 or 2) 否定整组选择，而 (not 1) or 2 把否定限制在第一支。

`not` 也可观察仅含受支持标量字段的结构体和 enum，包含它们的嵌套组合。例如：

```flurry
struct Flag { value: bool }
enum State { idle, ready(Flag) }

fn is_not_ready(state: State) -> bool {
    state match {
        not .ready({ value: true }) => true,
        .ready({ value: true }) => false,
    }
}
```

取反先完成内部候选匹配，包含标签检查和 guard，再反转结果；内部绑定不向成功的外层候选导出。执行仍须为被观察的 State 和 Flag 提供 Copy 依据，并递归检查清理责任；类型结构可观察本身不构成 Copy 证明。含指针字段的聚合仍不属于当前取反实现的支持范围。泛型字段可使用当前作用域中已证明的类型等式，但不会展开 normal-form 头以寻找字段类型。

上例两个互补分支可证明穷尽。覆盖分析支持布尔字段、空载荷以及 tuple、结构体和 enum 的嵌套构造模式，也可对这些模式的 `or` 求补集，保留字段之间的对应关系。该转换只服务于覆盖证明，运行时仍执行原来的候选和 guard。未知 guard、提取器调用及当前不能精确求补的结构不会被当成“不匹配”再取反；分析预算耗尽时返回 Unknown，不因此接纳缺少覆盖的 match。

布尔字段的候选局部 guard 也可精确分析。例如 `not .ready({ value: flag if flag })` 与 `.ready({ value: flag if flag })` 互补；直接匹配 bool 时，`flag if flag` 与 `flag if not flag` 穷尽两个值。分析支持布尔等值和逻辑组合，但不能用外部变量或未知函数调用代替候选自身的证据。即使覆盖已经证明，运行时仍按源码顺序执行 guard，并在某个 `or` 候选成功后停止尝试后续候选。

布尔、整数和字符的 guard 块还可以在块内声明并更新局部标量，例如 `x if do { var selected = x < 10; selected = not selected; selected }`。覆盖分析保留赋值前已保存的值，并按赋值后的状态检查结果；它不把对外部绑定或投影位置的写入视为可证明的局部计算，未知调用也不会因先赋给局部变量而获得证据。

精确 Float、f32 和 f64 的 guard 块使用相同的局部赋值规则。浮点阈值与布尔判断均可保存后更新；f32/f64 的比较仍保留 NaN 和正负零语义。先调用未知函数再覆盖其结果，不会消除必须执行该调用的事实，因而不能借后续赋值制造精确覆盖证据。

整数和字符字段也可使用已有区间分析，例如 `not (.number(-5..=5) or .letter('a'..='z'))` 与括号内的原模式共同覆盖对应 enum。求补必须先取得精确的字段匹配区域；仅有保守的正向覆盖估计时，不能反转它。范围检查使用候选局部的主语观察，不导出内部临时绑定，因此多个范围分支的 `or` 不要求这些临时绑定相同。读取外部变量或调用未知函数的 guard 仍需额外的覆盖分支。

浮点字段沿用各自的分析域：精确 comptime Float 保留精确端点，f32/f64 保留 NaN 的无序类别并将正负零视为同一个相等类。例如 `not .some(0.0..1.0)` 接纳载荷为 NaN 的 `.some`，不能改写成两个反向比较的并集。NaN 字面量模式本身不匹配任何值；对它求补也不能遗漏 NaN。自相等、非自相等或未知 guard 仍按完整浮点域检查，不因嵌在 enum 里就获得额外的覆盖证据。

Symbol 字段的补集包含未在源码中列出的符号。例如 `not (.some(.red) or .some(.blue))` 不仅匹配其他 enum 变体，也匹配 `.some(.green)` 以及其他名字。分析始终保留“其他符号”的相等类；仅列出几个 Symbol 字面量不会穷尽开放的 Symbol 域。自包含的符号比较 guard 可以精确求补，依赖外部变量或未知调用的字段 guard 当前不提供这种聚合补集证据。

错误结果也保留成功与错误分支的区别：

```flurry
enum Failure { bad(bool) }
typealias Outcome = ![Failure] bool;

fn not_bad(value: Outcome) -> bool {
    value match {
        not (error .bad(true)) => true,
        error .bad(true) => false,
    }
}
```

第一个分支接纳成功结果和 `error Failure.bad(false)`。`not true!` 则接纳成功值 false 以及所有错误分支。覆盖分析逐一保留完整错误类型身份，例如 `Failure<bool>` 与 `Failure<u8>` 不能合并；成功载荷为 void 时同样可以匹配和取反。这些是错误结果的分支规则，不改变错误与代数效应各自的 ABI。执行仍要求结果聚合及其非内建载荷具有 Copy 依据，并检查所有分支的清理责任，不能根据当前选中的标签跳过其他载荷的检查。

括号只改变分组，不能使原本不合法的模式绑定变合法。例如 or 的两支导出不兼容绑定时，加括号仍然拒绝。

### 与表达式对偶的模式

| 模式 | 对偶的表达式侧 |
|------|----------------|
| `pat?` | option 的消除 / 传播（`?`） |
| `pat!` | 错误限定类型成功值一侧的消除 / 传播（`!`） |
| `error pat` | 错误分支构造 `error expr` |

### 位向量模式

仅适用于 `BitVec`。见 [位向量模式](bitvec.md)。

### `case` 定义的自定义模式

在类型或 `impl` 体中声明：

```text
case name(self_or_ref_self, ...) -> ReturnType
where ...
{ ... }
```

- 参数一般含 `self` 或 `*self`。
- **返回类型写在签名上**；解构类 case 一般为 **option 包一层元组**（如 `?Tuple<...>`）：`null` 表示「本 case 不匹配」，有值则载荷供匹配臂绑定。
- 匹配侧写作 `.name` 或 `.name(pat*)`，与符号 / 变体风格一致。
- 未在模式中用到的字段，实现不必强制读取——这是用 `case` 做视图、而不是普通函数强行返回完整元组的原因之一。
- **纯度**与**可分析性**各有分级，但本章不定义具体分级表（纯度细节见内存管理相关章节；可分析性的分级规则另行制订）。

## 常见示例

```flurry
-- 字面量 / 范围 / or / _
fn classify_score(score: i32) -> str {
    score match {
        100 => "perfect",
        90..=99 => "excellent",
        60..=89 => "pass",
        0 or 1 => "near_zero",
        _ => "other",
    }
}

-- 符号与枚举变体
fn status_message(status: Status) -> str {
    status match {
        .ok => "ok",
        .pending => "wait",
        .error => "fail",
    }
}

fn handle_msg(msg: Msg) {
    if msg is do {
        .quit => shutdown(),
        .write(text) => write_all(text),
        .move { x, y } => move_to(x, y),
    }
}

-- 结构体、列表、as、守卫
fn on_y_axis(point: Point) {
    if point is { x: 0, y } do {
        println("{}", y);
    }
}

fn split_head(items: List<i32>) {
    items match {
        [] => println("empty"),
        [only] => println("{}", only),
        [first, second, ...rest] => println("{}, {}, {}", first, second, rest),
    }
}

fn same_xy(point: Point) {
    if point is { x, y } if x == y do {
        println("diagonal");
    }
}

fn keep_move(msg: Msg) {
    if msg is .move { x, y } as whole do {
        record(whole);
        move_to(x, y);
    }
}

-- and is：链式继续匹配
fn enroll(student: Student, course: Course) {
    if student is { class_id: id? }
        and get_class(id) is class?
        and get_major(class.major_id) is major?
        if course.satisfied(major)
    do {
        accept(student, course);
    }
}

-- 与表达式对偶：? / ! / error
fn from_option(opt: ?i32) -> i32 {
    if opt is n? do {
        n
    } else {
        0
    }
}

fn from_result(result: !IoErr i32) -> i32 {
    if result is v! do {
        v
    } else if result is error e do {
        eprintln("{}", e);
        0
    } else {
        0
    }
}

-- let / var：解构；可反驳模式可接 else（else 须发散）
fn unpack_pair(pair: Tuple<i32, i32>) -> i32 {
    let (x, y) = pair;
    x + y
}

fn require_adult(student: ?Student) -> str {
    let s? = student else {
        return "missing";
    }
    let .adult(display, _) = s else {
        return "minor";
    }
    display
}

fn mut_coords(start: Point) {
    var { x, y } = start;
    x = x + 1;
    y = y + 1;
    save(x, y);
}

-- case：自定义匹配视图（返回 ?Tuple；null 表示本 case 不匹配）
struct Student {
    name: str,
    age: usize,

    pub case adult(*self) -> ?Tuple<str, usize> {
        if self.age >= 18 {
            (self.name, self.age)
        } else {
            null
        }
    }
}

fn greet(student: Student) {
    student match {
        .adult(name, age) => greet_adult(name, age),
        _ => greet_minor(),
    }
}


```

（`Status` / `Msg` / `Point` / `Student` 等为示意；`?Tuple<>` 表示无元素载荷的 option，若标准库写法不同以库为准。谓词型 `case` 是否改用 `-> bool` 仍可选待定。）

### 用括号控制匹配范围

~~~flurry
fn selected(value: ?u32) -> bool {
    value match {
        (1u32 or 2u32)? => true,
        _ => false,
    }
}

fn selected_number(value: u32) -> u32 {
    value match {
        ((10u32..20u32) or 30u32) as number => number,
        _ => 0u32,
    }
}

fn grouped_value(value: u32) -> u32 {
    let (number) = value;
    number
}

test {
    println("{} {}", selected(2u32), selected(null));
    println("{} {}", selected_number(15u32), selected_number(25u32));
    println("{}", grouped_value(9u32));
}
~~~

依次输出 true false、15 0、9。最后一个 let 对 u32 使用分组，不要求输入是单元素元组。

## 与 Rust / Zig / C / C++ 的关键差异

| 点 | Flurry |
|----|--------|
| 多臂入口 | `if … is do { }` 与后缀 `match`；前者可 `inline` |
| 必须 `do` | 与 `expr { }` 类构式消歧 |
| `?` / `!` / `error` | 模式与表达式消除/构造对偶 |
| 自定义匹配 | `case` 声明视图，匹配侧 `.name` |
| `let` / `var` 解构与 `else` | 模式可反驳时接 `else`；规则类比 Rust `let … else` |

## 运行时实现设计引导

1. 将模式编译为检验与绑定序列；`or` 为选择，`and is` 为顺序依赖。
2. `?` / `!` / `error` 与对应类型的标签/载荷布局一致。
3. 多臂：按序尝试，穷尽性由静态检查保证；`_` 为收尾。
4. `case`：调用（或内联）定义体；`null` 则试下一臂；有值则按元组载荷绑定臂参数。未提及的字段可不上来。
5. 失败：非穷尽多臂、绑定形状不一致、无效位段 → 静态错误。

## 编译器实现引导

1. 按优先级建立模式树；分组括号先递归解析完整子模式，再参与外层后缀或中缀组合。区分 (p)、(p,) 与 ()，保留 guard 的表达式边界以及前缀和范围的读取范围。
2. `if is do` / `match` 共享臂类型检查与穷尽性分析。
3. `case`：解析签名上的返回类型与子句；按 option+元组约定检查体；匹配侧 `.name` 解析到 case 声明。
4. 与效应 / 纯度 / 可分析性分级的接口预留；**分级表本章不写**（待定）。
5. `let` / `var`：复用同一模式检查器；可反驳模式的 `else` 须检查发散。

降低 `or` 候选时，绑定对应使用同时重命名：所有来源先从本候选成功导出的绑定中取得，再发布目标。环境中原有的同号局部、前一失败候选撤销后恢复的局部，都不能充当本候选的来源。来源和目标各自唯一，也不能覆盖本候选中未参与重命名的另一个导出绑定。解释执行与编译期折叠遵循同一规则；守卫失败或求值预算耗尽时，撤销候选中新建及改名的绑定。

---

## 纯度与可分析性（待定）

- **纯度**：分级存在；细则见内存管理相关章节。
- **可分析性**：分级存在；定义待定。

### ListPatternView 的当前实现入口

有限列表模式 `[p0, p1, ...]` 已接入显式绑定的 ListPatternView。编译器先调用长度方法，长度相等才从索引 0 开始逐项调用元素方法并匹配；前一步失败就跳过余下步骤。下一候选重新进行自己的观察，不缓存前一候选的方法结果。后一个元素模式可以使用前面元素的候选绑定。

以下是用于当前编译器入口的完整协议及实现示例；`Size` 在本例中明确使用 u64：

```flurry
typealias Type = .Type'builtin;
typealias Size = .u64'builtin;

trait ListView {
    assoc Element: Type;
    fn list_pattern_view_get_len(*self) -> Size;
    fn list_pattern_view_get_element(*self, comptime index: Size) -> Element;
}

struct Repeated { length: Size, value: bool }

impl ListView for Repeated {
    assoc Element: Type = bool;
    fn list_pattern_view_get_len(*self) -> Size { self.length.* }
    fn list_pattern_view_get_element(*self, comptime index: Size) -> bool {
        self.value.*
    }
}

fn same_pair(value: Repeated) -> bool {
    value match {
        [a, (b if a == b)] => true,
        _ => false,
    }
}
```

驱动通过 `Protocols.list_pattern` 或包限定的 `Selectors.list_pattern` 绑定 trait 身份；sysroot 配置键为 `list-pattern-protocol`。仅将 trait 命名为 ListPatternView 不会自动注册。检查器验证 `Element`、两个方法的引用接收者以及编译期索引参数。完整 sysroot 已提供并默认绑定 `builtin.ListPatternView`，同时经 builtin.prelude 导出；其长度与索引使用 `usize`，主机编译路径提供实际目标宽度。用户只需实现该协议，不必重复声明或传入绑定参数。显式驱动绑定仍可选择替代协议，长度与索引须使用同一个受支持的无符号类型，生成常量时按该类型检查范围；没有目标宽度依据时，不能猜测 `usize` 的布局。

这些方法仍是普通受检调用。资源检查要求候选中的指针输入具有已知来源，调用的内存摘要不能写入调用者内存，也不能包含尚未验证的清理；规格拒绝不能被吞成“模式不匹配”。主体在候选失败后保持可用，成功或退出后按正常规则清理。覆盖检查不会执行用户方法，使用该协议的值匹配通常仍需 `_` 后备臂。

使用默认协议的完整示例见 [sysroot-list-patterns.fl](../../../development/examples/sysroot-list-patterns.fl)，
通过 `python3 -B development/library_check.py --release --stage sysroot-patterns` 执行。
该用例覆盖精确长度、`..._` 前缀、前项绑定用于后项守卫、空列表、失败候选重试，
以及带 Drop 的仿射主体只清理一次。协议不自动授予主体或提取结果 Copy，
也不代表尚未实现的 TuplePatternView、映射或字符串模式协议已经可用。

默认协议还提供独立的 comptime 相：`ElementComptime` 与
`list_pattern_view_get_len_comptime(self)`、
`list_pattern_view_get_element_comptime(self, index: usize)` 都标有
`comptime_phase` 属性。两个方法是 comptime fn，接收值主语；索引是普通值参数，
不再额外标记 comptime。运行时相仍借用主语，索引使用显式 comptime 参数。
模式所处阶段决定选择哪组方法和元素类型；quoted runtime code 使用运行时相。

```flurry
struct PairView { value: bool }

comptime impl ListPatternView for PairView {
    assoc Element: Type = bool;
    assoc ElementComptime: Type = Integer;
    comptime fn list_pattern_view_get_len_comptime(self) -> usize { 2 }
    comptime fn list_pattern_view_get_element_comptime(self, index: usize) -> Integer {
        if self.value { 11 } else { 22 }
    }
}

comptime fn inspect(value: PairView) -> bool {
    value match { [11, 11] => true, _ => false }
}
```

这里 `Element` 是共同关联声明，comptime contribution 仍提供它；实际编译期提取
返回的是 `ElementComptime`。只实现运行时相的类型不必提供 comptime 相成员，
运行时 IIR 也不会为未提供的编译期成员物化签名。两相的实现分别选择，不能仅凭
方法名带有 `_comptime` 就取得对应的 phase。

显式绑定的单相协议仍使用前述方法名与已声明的阶段。完整 sysroot 为
`meta.List<T>` 提供 comptime contribution，元素类型为 T，长度和索引通过
`meta.to_usize`／`meta.from_usize` 精确转换。负数或超出目标 usize 范围的
转换会失败，不发生截断；没有目标依据时不能猜测 usize 宽度。该实现不会
赋予 comptime 列表 runtime 表示，也不将两个整数类型视为同一类型。

列表模式的内部主体观察名只在本次候选匹配中可见，不是成功分支导出的变量。or-pattern 可以据此独立尝试各分支；成功后只统一导出用户绑定。该规则也适用于嵌套 tuple、`as whole` 和 inline／quote 展开。内部主体地址不得通过提取绑定逃逸，候选失败不清理原主体。

泛型函数可以通过协议约束确定元素类型，并把关联结果交付给调用者；不同实现的 `Element` 分别实例化，不要求所有列表使用同一种元素类型：

```flurry
fn first_of_pair(value: T, fallback: T.as(ListView).Element)
    -> T.as(ListView).Element
where T :- ListView {
    value match {
        [first, _] => first,
        _ => fallback,
    }
}
```

这里借用上一例的 `ListView` 协议名。匹配仍要求元素具有相应的 Copy 依据。

`pure` 不会跳过模式中的隐式调用。纯度分析检查长度与提取器的行为，并只在内部候选作用域安装观察绑定。当前可接受已有独立 P4 依据的提取器；只读指针访问本身并不构成 P4，跨函数传播私有目标来源仍有保守限制。

编译期列表观察也已接通：将协议及实现的两个方法都声明为 `comptime fn`，即可在 comptime 函数中使用列表模式。长度不匹配、前项模式失败时，后续提取器不会执行；前项绑定可用于后项 guard。提取结果也可以是 Type，并用于后续类型计算。读取存储的 Type 元素仍须有 Copy 依据，comptime 不免除资源检查。

编译期协议的长度与索引还可使用 `Integer`，与 `List.length` 和列表索引的编译期域一致；编译器生成的长度和索引始终非负。runtime 协议仍要求无符号机器大小类型。下面的协议可显式绑定为列表模式入口，并通过普通泛型实现观察 `meta.List<T>`：

```flurry
trait ComptimeListView {
    assoc Element: Type;
    comptime fn list_pattern_view_get_len(*self) -> Integer;
    comptime fn list_pattern_view_get_element(*self, comptime index: Integer) -> Element;
}

impl ComptimeListView for meta.List<T> where T {
    assoc Element: Type = T;
    comptime fn list_pattern_view_get_len(*self) -> Integer { self.*.length() }
    comptime fn list_pattern_view_get_element(*self, comptime index: Integer) -> T {
        self.*[index]
    }
}

comptime fn First(types: meta.List<Type>) -> Type {
    types match {
        [first, ..._] => first,
        _ => bool,
    }
}
```

这里 `Type`、`Integer` 来自默认 prelude；驱动仍需显式绑定上述协议身份。`List<T>` 的用户实现资格来自已绑定的内置 List 契约，不展开其函数体，也不使其他返回 List 的 opaque factory 自动获得该资格。列表元素观察使用不可变快照，不向 runtime 授予 List 的 Copy 能力。嵌套列表继续逐层调用同一协议，各层都先检查长度再提取元素。

两个方法必须属于同一执行阶段。runtime 匹配与 quoted pattern 中的列表模式要求 runtime 协议；在 comptime 中生成 quoted pattern，不会把该模式将来执行的提取器变成 comptime 调用。当前同一显式绑定不能同时提供这两个阶段的协议。

元素也可以是 `Tuple<bool, bool>` 等聚合。此时嵌套模式仍写作 `[(a, b), (c, d)]`；协议的 Element 类型与模式解构的结构视图保持区别。具有 Copy 依据的标量 tuple 可作为提取结果，后项 guard 可以读取前项成功绑定；只有元素为标量，并不自动授予整个 tuple 的 Copy 证据。

仅含标量字段的用户结构体和 enum 也可作为提取结果，支持嵌套模式和 guard 失败后的候选重试。其 Copy 检查递归覆盖内层聚合及 enum 的所有变体：外层声明 Copy 不能代替内层所需的依据，也不能复制带 Drop 的清理责任。含指针字段的结构体仍不在这条路径的支持范围内；Copy 声明不能替代指针来源与候选失败路径上的资源传递检查。

这些结构可以组合。例如提取结果为 `Tuple<Item, bool>` 时，可用 `[({value: first}, flag)]` 匹配；检查会同时要求外层 Tuple 与内层 Item 的 Copy 依据，并递归检查 Item 的字段。结构检查使用 Tuple 的布局视图，不展开其 normal-form 头。

忽略尾部的 `[first, second, ..._]` 已接通：先检查长度至少为 2，再依次提取前两项；不会提取或构造被忽略的尾部。`[..._]` 只执行长度观察，不提取元素。runtime 与 comptime 使用相同的前缀规则，失败候选仍按上述规则撤销绑定并重试。rest 必须位于列表末尾，且只能出现一次。

当前尚未完成 endless 模式、具名 `...rest` 所需的尾部切片协议和拥有资源的提取结果。具名 rest 会保留到 HIR，并明确报告尚缺切片协议。未经过资源检查的解释结果仍不能当成可执行承诺。
