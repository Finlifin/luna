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

- **字面量**：与值相等则匹配。
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
fn inspect_ready(packet: Packet<T>) -> (bool, Packet<T>) where T {
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
fn inspect(value: (bool, Ticket, Ticket)) {
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

这里沿用前文具有 `ready: bool` 字段的 Packet。`accept` 的实参只复制字段，返回 false 时原 Packet 仍由后续候选接收；调用不会重放，短路表达式仍保持短路。callee 自己创建的资源按正常调用规则清理。当前实现支持直接函数及已实例化函数、标量和具备 Copy 证据的代码实参与结果、合法默认实参、已知代码指针调用及嵌套调用；调用仍经过普通调用检查。传递仿射值或指针、未知回调、带环境闭包以及可能发生契约拒绝的调用尚需完整的候选失败路径资源事务，当前明确拒绝。这是实现范围，不是语言对守卫表达式的永久限制。

只观察绑定的守卫也可位于 `or` 的内部或单个字段上。括号明确划定候选范围：

```flurry
fn inspect_packets(value: (Packet<T>, Packet<T>)) -> bool where T {
    value match {
        ((first, second) if accept(first.ready)) or
        ((second, first) if accept(first.ready)) => first.ready,
        _ => false,
    }
}
```

第一候选的守卫失败后，撤销它的绑定及来源，再按第二候选自己的名字环境求值。这里第二次 `first` 来自原 tuple 的第二个位置；统一候选绑定名字不能改变该读取。成功后仍按实际绑定取得次序逆序清理。字段守卫、`as whole` 内部的 Copy 观察和 `let-else` 复用此规则；守卫中的仿射消费仍不能绕过失败路径检查。

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

块中的标量绑定只在块内有效；块结束后，中间值由后续模式接收。局部声明、嵌套块、分支、标量调用和当前块局部变量的赋值已接通。失败候选不会留下这些局部变量；后续候选执行自己的计算，不会为了构造清理计划而重放前一候选的步骤。当前不允许这种观察块写入外部变量、创建资源或指针绑定、消费候选所有者或向外层 return；这些操作需要一般候选事务承担失败路径责任。

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
