# newtype 与 typealias

## Intro

typealias 给已有类型另一个名字。newtype 建立需要独立区分的类型，同时保持与底层类型的直接表示关系。这种"零成本抽象边界"的思路源自 Haskell 的 newtype 概念：在不引入运行时开销的前提下获得类型系统层面的身份隔离，从而防止语义不同但表示相同的值被误用。

当程序要区分两个同样编码的概念时，选身份隔离；当程序只想给复杂类型计算结果命名时，别名更合适。

## 语法

~~~text
typealias_def -> typealias id (<parameter*>)? = expr
newtype_def -> newtype id (<parameter*>)? = expr
as_expr -> expr.as(expr)
~~~

参数与表达式沿用全局语法。

## 正式定义

typealias 不增加类型身份。参数化 alias 的目标仍按其表达式的计算与身份规则确定。

newtype 与底层类型及其他 newtype 声明具有不同身份；保持底层本体的大小、对齐和合法值编码。这个保证不推广为任意同大小类型可以互转。

newtype 不继承底层字段使用、构造、方法或 trait 实现。构造先得到底层值再 cast；使用底层接口先 cast 回底层。独立定义的新接口与资源操作遵循自己的契约。

纯类型工厂的重复调用沿用缓存及结果复用规则，alias 不额外改变该模型。

## 常见示例

把用户编号与订单编号都保存成 `u32`，同时要求接口区分二者：

~~~flurry
typealias Count = u32;
newtype UserId = u32;
newtype OrderId = u32;

fn show_user(id: UserId) {
    println("user {}", id.as(u32));
}

fn show_order(id: OrderId) {
    println("order {}", id.as(u32));
}

test {
    let count: Count = 42;
    let user = count.as(UserId);
    let order = count.as(OrderId);

    show_user(user);
    show_order(order);
}
~~~

两个接口分别输出 `user 42`、`order 42`。数值相同，但参数要求不同。两个函数都在内部显式回到 `u32`，供底层数值接口使用；调用者不再使用已经交出的 newtype 值。

以下调用分别是静态错误：

~~~flurry
test {
    let order = 42u32.as(OrderId);
    show_user(order);
}

test {
    let count: Count = 42;
    show_user(count);
}
~~~

`Count` 没有新身份，`OrderId` 有自己的身份；二者都不能凭宽度相同就充当 `UserId`。

表示关系也不带来成员继承：

~~~flurry
struct Point {
    x: i32,
    y: i32,

    fn horizontal(self) -> i32 {
        self.x
    }
}

newtype ScreenPoint = Point;

test {
    let screen = (Point { x: 12, y: 8 }).as(ScreenPoint);
    println("x: {}", screen.as(Point).horizontal());
}
~~~

调用先显式取得 `Point` 视角，才使用该类型的方法。`screen.horizontal()` 不能由 newtype 声明本身获得调用依据。

### 容器内的表示类型

容器内的 newtype 可以在表示中引用外层类型参数或 `Self`。这些引用随所属类型实例一起实例化；不会把不同实例中的内部类型合并成只有声明名的类型。

~~~flurry
struct Holder where T {
    value: T,

    typealias Payload = T;
    newtype Wrapped = Payload;

    fn get(self) -> T {
        let wrapped = self.value.as(Wrapped);
        wrapped.as(Payload)
    }
}

fn run() -> bool {
    Holder<bool> { value: true }.get()
}
~~~

这里 `Wrapped` 没有自己的显式泛型参数，使用时不必补写外层 `T`。编译器内部的名义实参仍保留所属实例的类型环境，供表示实例化与类型身份比较使用。若表示写成 `Self`，它指向所属的完整 `Holder<T>`，包装和拆包仍各自只跨越一条直接表示边。

公开的内部类型也可以通过参数化容器投影，外层实参与成员自身实参分别填写：

~~~flurry
struct Tagged where Context {
    pub typealias Payload = Context;
    pub newtype Wrapped<T> = T;
}

fn accept(value: Tagged<bool>.Payload) -> Tagged<bool>.Wrapped<bool> {
    value.as(Tagged<bool>.Wrapped<bool>)
}

fn run() -> bool {
    accept(true).as(bool)
}
~~~

`Tagged<bool>.Wrapped<bool>` 与 `Tagged<char>.Wrapped<bool>` 即使拥有相同表示，也保留不同的外层实例环境。类型成员的可访问性仍由成员声明和公开入口决定，提供参数不会绕过 package 或 private 边界。

透明别名同样保留容器的成员和实例环境：

```flurry
typealias FlagTag = Tagged<bool>;

fn preserve(value: FlagTag.Wrapped<bool>) -> Tagged<bool>.Wrapped<bool> {
    value
}
```

这里的两种参数类型相同。别名不会重新声明 `Wrapped`，也不会把原本不可访问的成员变成公开成员；成员默认参数仍在 `Tagged<bool>` 的环境中解释。

别名也可以把计算得到的 Type 传给容器，或串接多个类型成员：

~~~flurry
typealias Tag<Context> = Tagged<Context>;

comptime fn WrappedType(context: Type, value: Type) -> Type {
    Tag<context>.Wrapped<value>
}

struct Catalog where Context {
    pub typealias Selected = Tagged<Context>;
}

comptime fn SelectedType(context: Type, value: Type) -> Type {
    Catalog<context>.Selected.Wrapped<value>
}
~~~

每一层投影使用该层实际选定的容器环境。展开别名不会重复调用产生外层 Type 的函数，也不会省略别名或容器的形成检查；外层检查通过后才求值下一层的成员参数。

当类型来自函数调用时，类型位置先完成该调用，再从结果选取成员：

~~~flurry
comptime fn SelectTag(flag: bool) -> Type {
    if flag { Tagged<bool> } else { Tagged<char> }
}

fn character(value: SelectTag(false).Payload) -> char {
    value
}
~~~

这里需要完成的结果是 Type 值，成员查找使用返回容器的身份和原有访问权限。它不等同于展开 normal-form 头：即使某个 normal-form 函数的主体返回 `Tagged<T>`，也不能由此把该头当作 `Tagged<T>` 查找成员。

已完成的局部函数常量也保留工厂的 comptime 调用类别：

~~~flurry
fn preserve_flag(value: bool) -> bool {
    const Factory = SelectTag;
    let preserved: Factory(true).Payload = value;
    preserved
}
~~~

外层和成员的 Type 实参都可以由 comptime invocation 中的计算产生：

~~~flurry
comptime fn Wrapped(context: Type, value: Type) -> Type {
    Tagged<context>.Wrapped<value>
}
~~~

先求得并检查外层 `Tagged<context>`，再构造成员。每一层的显式实参按源码顺序各求值一次，随后按声明次序补默认参数；默认参数可复用前面已经求得的输入。外层检查失败或控制流提前返回时，不执行成员实参。名义构造保留形成条件，依赖动态输入的 trait bound 在实际 Type 已知后检查，通过检查才产生类型值。

显式成员实参在调用者的词法环境中解释；成员默认值和主体使用选定的容器环境。因此在 `Tagged<T>` 内写 `Tagged<bool>.Wrapped<T>`，显式 `T` 仍指调用者的参数，而不是被选定容器的 `bool` 替换。成员中的 `Self` 对应已经构造并检查的外层类型值。

外层的值参数也能决定成员参数的类型：

```flurry
struct Config where enabled: bool { flag: bool }
struct Holder where enabled: bool {
    pub newtype Wrapped<config: Config<enabled>> = bool;
}

comptime fn Build() -> Type {
    Holder<true>.Wrapped<Config<true> { flag: true }>
}
```

这里成员参数的类型是选定实例给出的 `Config<true>`。改传 `Config<false>` 不满足要求，即使两个结构的字段类型相同。若外层布尔实参由 comptime 调用取得，则先求值一次，再依据实际值完成成员参数检查；不会把调用者自身的 `enabled` 当成所选实例的参数。

完整结构值也可以作为 newtype 的参数，并由固有 impl 的主体读取：

```flurry
struct Settings { enabled: bool }
newtype Packet<settings: Settings> = bool;

impl Packet<settings> where settings: Settings {
    fn read(self) -> bool { self.as(bool) and settings.enabled }
}

fn run() -> bool {
    true.as(Packet<Settings { enabled: true }>).read()
}
```

`settings` 是从接收者类型身份推导出的完整值。候选匹配与最终证据检查都验证它对应的声明布局和字段类型；方法主体使用该实例的参数，不重新执行构造表达式。



内部定义的形成条件也继承外层已声明的类型前提。调用者恰好传入满足要求的类型，不足以弥补泛型定义本身缺失的前提。

### 透明别名的动态形成检查

透明别名的参数约束不会因为实参来自 comptime 计算而消失：

~~~flurry
trait Allowed {}
impl Allowed for bool {}

typealias Accepted<T :- Allowed> = T;

comptime fn AcceptedType(value: Type) -> Type {
    Accepted<value>
}
~~~

`AcceptedType(bool)` 得到 `bool`，没有额外的别名类型身份。若实际输入不能建立 `Allowed` 证据，invocation 在展开别名主体前失败。实参和默认项先各求值一次，形成检查通过后才继续主体；这与只检查最终结果类型不同，主体即使不使用某个受约束参数，也不能省略该参数的形成条件。

## 作为 comptime 输入

newtype 值可作为 runtime 函数的显式 comptime 输入。省略的默认值、引用前项的默认别名和计算型展开包保留包装身份；实例化消除参数槽时，实例身份仍记录包装声明、类型实参及其表示值。

~~~flurry
newtype Flag = bool;
newtype Policy = Flag;

comptime fn make_policy() -> Policy {
    false.as(Flag).as(Policy)
}

fn enabled(
    comptime .policy: Policy = true.as(Flag).as(Policy),
    comptime .alias: Policy = policy,
) -> bool {
    alias.as(Flag).as(bool)
}

fn default_enabled() -> bool { enabled() }
fn configured_enabled() -> bool { enabled(.policy = make_policy()) }
~~~

两个入口分别得到 true 和 false。Policy、Flag 和 bool 在冻结的输入中仍有不同身份，包装层次不因底层都是布尔值而消失。泛型实例取得类型实参后，也可以执行返回 newtype 的编译期工厂，再交给 runtime 实例。

staging 读取已检查的新类型表示声明来区分包装和解包，核对每一步的表示类型。没有声明布局、表示类型不符或转换方向不成立时不能构造实例键；不能仅因目标看起来是 nominal 类型就把 cast 当成包装。表示查询也不展开普通 normal-form 头。

包装具名 comptime 回调的 newtype 可以参与调用点类型计算，回调依赖递归保留；这不使捕获可变环境的闭包获得不可变快照表示。可变闭包、指针和资源的交付限制仍由各自的内存与阶段规则决定。

## 与 Rust / Zig / C / C++ 的关键差异

Rust tuple struct 和 C++ 包装类可建立独立身份，但表示保证需看各自规则。Rust alias、C typedef 和 C++ alias 则是命名手段；Zig 可给类型值命名。Flurry newtype 的直接表示与接口隔离同时成立。[Rust alias](https://doc.rust-lang.org/reference/items/type-aliases.html)、[C++ alias](https://eel.is/c++draft/temp.alias)、[Rust 布局](https://doc.rust-lang.org/reference/type-layout.html)

## 运行时实现设计引导

允许复用底层存储布局，但不复用未经定义的行为入口。转换是否需要运行时指令由合法降码决定；零指令转换也必须保留静态身份变化。

## 编译器实现引导

alias 解析到目标，newtype 保存独立声明身份及底层表示边。结构和接口查询不能无条件穿透该边。

类型等式、布局查询、cast 和成员解析分别访问所需信息。验收应同时检查：alias 相等、newtype 不等、布局一致、字段不继承、trait 不自动继承以及合法双向转换。


当前共享 NIR 的 newtype 描述已保存有序参数域，能区分 Type 与 comptime value binder。后续参数域可以引用前面的 Type 参数；未声明引用、前向引用、错误 sort 和域数量不匹配均被拒绝。表示替换保留值索引与 opaque head；即使底层表示相同，两个不同索引的 wrapper 也不会获得彼此之间的直接 cast。参数域参与 S-expression dump 与复制预算，已完成实参另有域校验入口。

源码 newtype 已接入自身声明的值子句、依赖前序 Type 的值域和默认值；具体 IIR 布局及 comptime 值校验使用相同参数域。转换只沿声明的直接表示边，不把不同值索引的 wrapper 视为相等：

```flurry
newtype Flag<tag: bool> = bool;

fn read(value: Flag<true>) -> bool { value.as(bool) }

fn run() -> bool { read(true.as(Flag<true>)) }
```

`Flag<false>` 不能传给 read，即使两个实例的底层表示都是 bool。`newtype Tagged<T, tag: T> = T;` 的第二个实参按已选定的 T 检查；整数值域不允许通过截断接收越界索引。comptime 常量中的 wrapper 可在 comptime 解包，运行时使用仍遵守原有 Splice 协议，不能借名义转换绕过跨阶段规则。

嵌套 newtype 已接入父级 Type／值子句捕获；父级参数参与类型身份，也可用于自身参数的域、默认值和底层表示。显式实参在调用者环境中检查，省略的默认值在选定父级实例的环境中计算。

```flurry
struct Holder where T, tag:T {
    pub newtype Wrapped<.value:T=tag> = T;
}

fn read(value:Holder<bool,false>.Wrapped) -> bool {
    value.as(bool)
}

fn run() -> bool {
    read(true.as(Holder<bool,false>.Wrapped<.value=false>))
}
```

这里显式指定 `.value=false` 与省略该参数得到相同的身份；改成 `.value=true`，或改用另一个父级实例，不能传给 `read`。内部身份仍按“自身参数、父级捕获”排列，域检查则按词法作用域先检查父级捕获，再检查自身参数。一般程序前置规格不因这一支持自动进入类型求解器。


调用时的 nominal 类型构造现在也接入值参数，而不只支持静态类型位置。普通 comptime 函数可以根据调用实参返回不同身份的类型：

```flurry
comptime fn Wrapped(tag:bool) -> Type {
    Holder<bool,tag>.Wrapped
}

fn use(value:Wrapped(false)) -> bool {
    value.as(bool)
}
```

父级构造先于成员输入执行；各显式输入只求值一次，再按声明顺序补齐默认值。解释器在返回构造出的类型前检查完整值域及形成条件，失败不会导出半完成类型。该路径已覆盖 struct、enum、newtype 的独立值域、静态 Type 实参及成员父级值捕获。计算所得的混合 tuple 参数包已支持展开，保留各字段的 Type／值类别；包的生产者只求值一次，空包的副作用也会执行。多个展开包与具名参数按照源码顺序准备，然后补齐默认值。结构视图仅用于解构，不展开绑定的 Tuple normal-form 头。值域依赖尚未求得的 Type 输入时，已有独立类型的值表达式会保留到调用阶段检查；这包括展开包中的字段、引用前序值的默认值，以及稍后才提供的具名 Type 实参。位置参数、具名参数、字面量展开和省略默认项已接入依赖目标类型的整数字面量、条件、match 和块输入；值实参可以出现在决定其域的具名 Type 实参之前，仍按源码顺序各执行一次。默认项在显式输入之后按声明顺序求值，能够引用前项的结果；默认 Type 就绪后，先前暂存的字面量完成定型而不重放副作用。计算所得展开包也可与这些上下文输入组合：生产者按源码顺序执行一次，字段保留其自身类型；包中的 Type 字段可以为此前暂存的上下文输入完成定型。空包保留副作用，提前返回会跳过后续输入。计算包中的普通已定型整数不会因展开而重新解释成目标整数类型，不能把当前支持理解为任意输入均可延迟 elaboration。


```flurry
newtype Flags<a:bool,b:bool> = bool;

comptime fn packet(pointer:*bool) -> (bool,bool) {
    pointer.* = not pointer.*;
    (pointer.*,not pointer.*)
}

comptime fn Selected() -> Type {
    var flag=false;
    Flags<...packet(flag.ref)>
}
```

`Selected()` 返回 `Flags<true,false>`；展开不会重复调用 `packet`。包宽度不匹配、字段值域不匹配和非 tuple 展开均被拒绝。


```flurry
newtype Tagged<T,value:T> = T;

comptime fn TaggedBool(ty:Type,value:bool) -> Type {
    Tagged<ty,value>
}
```

`TaggedBool(bool,true)` 得到 `Tagged<bool,true>`；`TaggedBool(char,true)` 在 comptime invocation 中失败。构造入口既检查已完成索引是否属于最终值域，也核对值表达式的实际类型，不能因为不同定宽整数都编码为整数索引而忽略原来的 `u8`／`u16` 区别。normal-form 头不因此展开，值域检查也不成为 PTC 的程序命题。


嵌套成员的参数域使用选定父级实例的输入进行替换，而显式实参表达式仍在调用者环境中检查和求值。父级 Type 本身尚未确定时，成员的依赖值域延续到同一次 comptime invocation 中校验：

```flurry
struct Container where T, enabled:bool {
    pub newtype Entry<value:T,.copy:T=value> = T;
}

comptime fn SelectedEntry(ty:Type,enabled:bool,value:bool) -> Type {
    Container<ty,enabled>.Entry<value>
}
```

`SelectedEntry(bool,false,true)` 得到 `Container<bool,false>.Entry<true>`；将 `ty` 改成 `char` 会因成员值域不匹配而失败。调用者自身属于哪个 `Container` 实例，不会替代这里显式选定的父级实例。


调用时的参数展开保留提前退出：`Flag<...do { return result; (false,) }>` 中的 `return` 退出当前函数，后续参数和构造都不执行。退出后的尾表达式仍接受类型和宽度检查；没有尾表达式时，可以使用无歧义的目标参数宽度完成检查，不会任意划分多个未知展开包。


```flurry
newtype Indexed<T,value:T> = bool;

comptime fn ByteIndexed(ty:Type,choose:bool) -> Type {
    Indexed<ty,if choose {7} else {255}>
}
```

`ByteIndexed(u8,true)` 按 u8 检查两个整数字面量，得到 `Indexed<u8,7>`。将未选中的 `255` 改成 `256` 仍会失败；字面量的合法性不取决于执行时选中了哪一条分支。普通已具备类型的值保留其类型，不能借上下文输入把一个 u16 值隐式当作 u8。NIR 的 nominal 构造现在复用上下文输入表示，dump 保留字面量拼写、块、分支和源码位置；延迟处理不会重复执行已经发生的副作用。


省略的值参数也保留上下文输入，不先把整数字面量固定为某种默认整数类型。例如：

```flurry
newtype Indexed<T, choose: bool, .value: T = if choose { 7 } else { 8 }> = bool;

comptime fn DefaultIndexed(ty: Type) -> Type {
    Indexed<ty, true>
}
```

`DefaultIndexed(u8)` 得到 `Indexed<u8,true,.value=7>`；若将未选中分支的 `8` 改成 `256`，调用仍失败。默认值还可直接引用前面的值参数，或形成连续默认项；这些引用读取已求得的输入，不重新执行其源码表达式。


直接名义父级的成员投影已复用完整的上下文构造路径。例如 `Parent<ty,7>.Wrapped` 可以先按实际 `ty` 为 `7` 定型、补齐引用它的父级默认参数，再选择 `Wrapped`。计算参数包仍只执行一次。父级形成条件检查失败时，成员实参不会执行；投影读取已完成类型值的参数，不重新执行父级输入。

透明别名的抽象参数会继续保持开放：例如 `typealias Alias<T> = Parent<T>` 可以保留父级依赖 `T` 的默认字面量，`typealias Alias<T> = Parent<T,7>` 也可以保留显式字面量。声明检查不提前选择整数表示；具体调用再检查全部分支及值域。多层透明别名沿用此规则，不展开 normal-form 头，也不在不同具体调用间共享定型结果。


源码中的透明别名现已支持值参数和命名默认参数。例如下面的别名保留 `Indexed` 的身份：

```flurry
newtype Indexed<flag: bool, .copy: bool = flag> = bool;
typealias Alias<flag: bool, .copy: bool = flag> = Indexed<flag, .copy = copy>;

comptime fn Build(flag: bool) -> Type {
    Alias<flag>
}

fn read(value: Indexed<true, .copy = true>) -> bool {
    value.as(bool)
}

fn example() -> bool {
    read(true.as(Build(true)))
}
```

显式实参按源码顺序各求值一次，默认参数读取已绑定的前序输入；别名主体可以重新排列或多次使用这些输入，但不会重新执行实参。未被主体使用的输入也必须满足声明域。声明检查拒绝默认参数的自引用、前向引用及已确定的类型错误；形成条件在展开主体前检查。

当前实现也支持 `typealias Alias<T, value: T> = Indexed<T, value>` 的动态类型输入。依赖字面量、条件分支和默认值保留上下文结构，等对应类型输入完成后检查全部分支及整数范围；不会先把整数字面量固定为默认整数类型。


成员别名的值默认参数也可以读取父级已完成的输入：

```flurry
newtype Indexed<T, value: T> = bool;

struct Parent where T, value: T {
    pub typealias Alias<.copy: T = value> = Indexed<T, copy>;
}

comptime fn Build(value: u8) -> Type {
    Parent<u8, value>.Alias
}
```

这里先构造并检查 `Parent<u8, value>`，再从完成的类型中取得参数。`Alias` 的默认值读取父级 `value`；显式 `.copy` 可以覆盖它。已明确的父级类型输入会在投影时保留，父级的动态值输入仍只求值一次。经由透明父级别名访问该成员也遵循相同规则。

成员别名形成条件中的外层值索引由捕获环境提供。例如父级的 `flag: bool` 可以出现在成员的 `Allowed<flag>` trait 约束中。形成条件记录捕获域及其依赖，先取得捕获参数，再检查成员自己的输入；这仍是类型命题检查，不将程序状态规格交给 PTC。


动态父级输入也会带入成员别名主体。把上例的构造函数改为：

```flurry
comptime fn Build(ty: Type) -> Type {
    Parent<ty, 7>.Alias
}
```

调用 `Build(u8)` 时，父级先完成上下文字面量 `7` 的检查与构造；成员别名读取已完成的值，检查自己的形成条件，再以同一组具体类型实例化主体，得到 `Indexed<u8, 7>`。嵌套的透明别名沿用这组类型信息。主体实例化只修改本次调用的私有 IR 副本，失败不污染后续调用；形成条件失败时不会提前复制或执行主体。别名自身的输入也使用同一套上下文求值规则：`Alias<ty, 7>` 及依赖 `ty` 的默认字面量可以等待类型输入完成后定型。


别名的展开参数同样遵循源码求值顺序。例如命名值参数出现在提供类型的计算型参数包之前时，先保存值参数中已执行的控制流和副作用，再读取参数包；类型域就绪后只完成字面量定型，不重放前面的计算。实参提前返回时，后续参数、默认值和别名主体均不执行。局部参数绑定在正常完成、错误或提前返回时释放，不进入调用者作用域。


```flurry
newtype Indexed<T, value: T> = bool;
typealias At<T, .value: T = if true { 7 } else { 255 }> = Indexed<T, value>;

comptime fn Build(ty: Type) -> Type {
    At<ty>
}
```

`Build(u8)` 和 `Build(u16)` 分别形成各自的 `Indexed` 类型。把未选中的 `255` 改为 `256` 后，`Build(u8)` 仍然失败；`Build(u16)` 可以成功。不同调用不会共享已定型的默认值。成员别名也适用，包括类型域来自动态父级的显式字面量和默认字面量；显式实参仍读取调用者环境，默认值读取选中的父级环境。
