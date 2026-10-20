# 实现选择与结构索引

## Intro

当多个 implementation 同时适用于一个查询时，语言需要一种确定性的选择规则。Flurry 采用结构精度作为选择依据：实现头中暴露的类型结构越具体，优先级越高。这种方式基于类型结构本身，而非依赖声明顺序或附加注解。

同一个接口可以为整个类型族提供实现，也可以为其中一种形态提供更精确的实现。选择依据是类型已经暴露的结构；要求负责判断实现是否适用。

更长的 requires 不会使实现优先。先找到一个能够工作的实现，也不意味着编译器可以忽略尚未排除的更精确实现。

## 语法

~~~text
impl_trait_def -> comptime? impl expr for expr clauses? { (assoc_decl | definition | statement)* }
impl_def -> comptime? impl expr clauses? { (definition | statement)* }
requires_clause -> requires expr
normal_form_head_application -> expr<argument*>
~~~

## 正式定义

### 匹配与适用性

查询包含主体、完整 trait 实例、所需 phase 和合法实现环境。实现头先与查询的已知形态匹配，再检查参数关系和 requires。定义端的量化参数保持刚性；该次查询新建的推断变量可以求解。

匹配只观察合法的归一形态和已证明关系。普通类型工厂按实际结果处理，不因名字出现在实现头中就成为可逆、单射的构造子。

候选适用性首先受用户实现的主体范围约束，见[接口声明](declarations.md)。替换后的主体若不允许用户 impl，该用户候选被排除；主体仍未知时保留资格义务。通配实现不能无条件证明所有类型都具备该接口。语言内建的能力按各自规则处理，不经由禁止的用户实现绕行。

候选有三种适用性：已经适用、已被排除、仍未决。没有证明要求，不等于证明要求为假。尚未交付环境的开放实现，还需要完成[依赖绑定](../type/substitution-inference.md)。

### 结构精度

结构精度来自 normalized 类型 IR 的统一深度分解。每次增加深度，展开所有尚未到达具体叶子的同层结构；已经到达的具体叶子保留。实现头中的未知参数限制能够统一展开到的深度。

用 ◇ 表示分析中的通配槽，它不是源语言 Any：

~~~text
实现头                         结构键                         精度
T                              ◇                              0
Box<T>                         Box<◇>                         1
Box<Pair<T, U>>                 Box<Pair<◇, ◇>>                 2
Box<Pair<i32, u32>>             Box<Pair<i32, u32>>             3
Pair<Box<T>, U>                 Pair<◇, ◇>                     1
Pair<Box<T>, i32>               Pair<Box<◇>, i32>               2
~~~

这里 Box、Pair 是分析用的名义构造子。精度计算保留声明身份与参数位置，具体值叶子也按其合法身份保留。某个分支较深，不允许越过另一分支的未知槽单独继续提高整棵头的精度。

重复变量、逻辑 refine、额外 trait 要求不增加结构精度。`Pair<T, T>` 与 `Pair<T, U>` 因而同精度；前者多出的等式只影响适用性。

### 选择与冲突

对同一主体和完整接口实例，选择适用候选中结构精度最高者。同精度的独立实现若有共同适用实例，就是冲突；同桶但真实头互斥的实现可以共存。

比当前适用者更精确的候选仍未决时，不能提交当前候选。同精度候选也必须全部排除或完成冲突判断。ordinary/loose 的检查责任沿用[两阶段检查](../comptime/normalization-and-instantiation.md)；允许延迟不允许最终交付留下未决选择。

接口实参尚未知时，应把候选的替换分开求解。不同完整接口实例之间的推断歧义，不等于同一接口实例内部的实现冲突；不能让第一次试探永久固定接口参数，再排除其他解。

### 名称来源

实现精度仅在已经确定的接口来源内比较。两个不同 trait 的同名方法，不以实现头更精确者获胜。接收者视角和解引用路径的次序见[适配](../type/receiver-adaptation.md)。

## 常见示例

整个类型族有一个实现，i32 元素还有更精确的实现：

~~~flurry
trait Describe {
    fn describe(*self) -> str;
}

struct Sample
where T
{
    value: T,
}

impl Describe for Sample<T>
where T
{
    fn describe(*self) -> str {
        "sample"
    }
}

impl Describe for Sample<i32> {
    fn describe(*self) -> str {
        "signed sample"
    }
}

test {
    let signed = Sample<i32> { value: 7 }
    let flag = Sample<bool> { value: true }
    println("{}", signed.describe());
    println("{}", flag.describe());
}
~~~

输出 `signed sample`、`sample`。第二份实现的头保留了具体 i32 叶子，因此精度更高。两份实现都在 Describe 定义包中，包归属不构成额外障碍。

下例独立展示应被拒绝的重叠：

~~~flurry
trait Classify {
    fn classification(*self) -> u32;
}

struct Pair
where A, B
{
    first: A,
    second: B,
}

impl Classify for Pair<T, T>
where T
{
    fn classification(*self) -> u32 {
        1
    }
}

impl Classify for Pair<A, B>
where A, B
{
    fn classification(*self) -> u32 {
        2
    }
}
~~~

两份头都归入精度 1 的 `Pair<◇, ◇>`；`Pair<i32, i32>` 同时满足它们。重复 T 不提高优先级，因此这两份声明不能共存。诊断应展示共同实例，而不是仅报告“落入同一桶”。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 的普通 trait 实现要求避免不允许的重叠；Flurry 明确定义不同结构精度的优先关系。C++ 模板特化和受约束声明也有选择机制，但其约束偏序不是这里的统一深度精度。[Rust implementations](https://doc.rust-lang.org/reference/items/implementations.html)、[C++ constraints](https://eel.is/c++draft/temp.constr)

C 无对等实现选择机制，可显式选择函数或生成代码；Zig 可在 comptime 代码中检查类型并分支。Flurry 的选择规则由语言统一规定。[C11 §6.5.2.2](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf)、[Zig comptime](https://ziglang.org/documentation/master/#comptime)

## 运行时实现设计引导

已完成选择的静态调用不需要运行时枚举实现。requires 若是实例化前提，必须在相应检查边界兑现，不能偷偷降成“运行时不满足就尝试下一个 impl”。

依赖对象内容可以影响方法执行，但不能借此改变已选 implementation 身份。

## 编译器实现引导

### 建立结构键

以下伪代码中 cut 只操作准入的归一结构：

~~~text
cut(term, depth):
    if depth == 0: return ◇
    if term is unresolved parameter: return ◇
    if term is concrete leaf: return term.identity
    return term.head(cut(child, depth - 1) for child in term.children)

index_depth(head):
    if head contains unresolved parameters:
        return minimum root distance of any unresolved parameter
    return maximum root distance of concrete leaves + 1
~~~

对于有未知参数的头，只生成 `cut(head, index_depth(head))` 作为最精确的统一键；完整头另存，用于真正匹配。封闭头的相同末端键去重。已知但不可进一步观察的项可作为身份叶子，不能擅自展开其私有结构。

### 查询完整性

具体查询的分解例：

~~~text
Tuple<Vec<Tuple<i32, i32>>, i32>

0  ◇
1  Tuple<◇, ◇>
2  Tuple<Vec<◇>, i32>
3  Tuple<Vec<Tuple<◇, ◇>>, i32>
4  Tuple<Vec<Tuple<i32, i32>>, i32>
~~~

第 2 层保留较浅的 i32。否则 `Tuple<Vec<T>, i32>` 的键将不在查询链中，导致漏检。

对具体查询，依次检索其深度键和全类型候选。对含推断变量的查询，必须同时检索与未知位置兼容的更细键：例如 `Sample<?T>` 既要看到 `Sample<◇>`，也要看到 `Sample<i32>`。只检索祖先键可以帮助发现成员，但不足以最终排除更精确实现。

全类型 target 单独按接口声明、成员等可用信息建立索引，不把所有实现挤入一个全局 ◇ 桶。若 trait 头本身开放，也需有开放 trait 候选的补充索引，不能因缺少具体 trait 键而漏检。

完整性不变量：对任意合法替换 σ，若某实现真实头可适用于查询，检索集必须包含该实现。索引可以多取，不能少取。

### 候选检查与提交

1. 取得候选集合，按接口实例约束分组；每个候选建立独立回退点。
2. 匹配头，求解允许赋值的推断变量，检查最终主体的用户实现资格；向 PTC 提交类型关系和要求。资格尚未确定则保留义务，不将通配头视为自动适用。
3. 保存适用、排除、未决及各自理由；回退未提交的试探赋值。
4. 按精度从高到低处理；较高层全部排除后才进入下一层。
5. 当前层唯一适用且其余候选均排除，才能提交；未决阻止最终选择。
6. 生成实现证据；关联项规范化与后续调用必须使用同一来源和替换。

内存效果、runtime 可达关系与程序状态规格交给程序验证层；即使它们伴随类型或接口交付，也不作为 PTC 的类型命题提交。

递归查询使用目标与替换状态检测环。重复目标不是成功证明；持续生成新目标的搜索受预算控制，耗尽返回未决或诊断，不返回“不实现”。

### 成本与缓存

预计算结构键可使用共享子树和哈希，避免每层重新复制整棵树。具体查询访问 O(h) 个深度层及 k 个候选，真实匹配和求解成本另计。未知查询可能访问大量兼容后代，最坏仍可退化为全候选检查。

候选缓存依赖实现世界和归一形态；选择缓存还依赖接口实参、phase、约束与替换。候选新增、关联项变化或推断变量得到新值时，受影响查询必须失效。不得用基于 Eq/Hash 的缓存相等替代逻辑等式。

### 泛型调用者的值参数与 inherent 方法

inherent 方法选择同样使用调用者值参数的声明域。调用者的参数与实现头的参数具有各自的声明身份；匹配建立本次替换，并检查替换后的域，而不是要求两者来自同一声明。依赖域也按这个规则处理：

~~~flurry
struct Tagged where T, tag: T {}
impl Tagged<T, tag> where T, tag: T {
    fn read() -> T { tag }
}
struct Caller where U, value: U {
    pub fn forward() -> U { Tagged<U, value>.read() }
}
fn example() -> bool { Caller<bool, true>.forward() }
~~~

`Tagged` 的值域在本次匹配中是 `U`，与调用者 `value: U` 的声明一致。缺少域证据或域不匹配时不能选择实现。成员名仍先限定候选集合；取得方法作为函数值与直接调用保留同样的参数实例身份。域证据不赋予额外成员，也不把 inherent 方法变成 trait 实现。

### 由类型等式建立值域相容性

值参数的域可以依赖类型参数。如果当前定义的类型上下文已经证明 `T == U`，声明为 `value: U` 的输入可以用于要求 `tag: T` 的 trait 查询：

~~~flurry
trait Read where T, tag: T { fn read() -> T; }
struct Tagged where T, tag: T {}
impl Read<T, tag> for Tagged<T, tag> where T, tag: T {
    fn read() -> T { tag }
}
struct Caller where T, U, value: U, requires T == U {
    pub fn forward() -> T { Tagged<U, value>.as(Read<T, value>).read() }
}
fn example() -> bool { Caller<bool, bool, true>.forward() }
~~~

这里证明的是 Type 域的相等性，值仍保留原声明身份。没有等式证据、求解结果未确定或预算不足时，不得把两个域当成相同。匹配证据回放时也必须在相应上下文中重新建立域等式；域相等不证明两个不同的值索引相等。

### 带符号字段的记录索引

记录索引在泛型定义中可以包含尚未取具体值的字段，只要字段各自的声明域已经确定：

~~~flurry
comptime fn settings(...properties: T) -> T where T { properties }
typealias Settings = (settings {enabled: true})'type;
trait Read where mode: Settings { fn read() -> bool; }
struct Host {}
impl Read<(settings {enabled: flag})> for Host where flag: bool {
    fn read() -> bool { flag }
}
struct Caller where enabled: bool {
    pub fn forward() -> bool {
        Host.as(Read<(settings {enabled: enabled})>).read()
    }
}
fn example() -> bool {
    Caller<true>.forward() and not Caller<false>.forward()
}
~~~

实现也可以把整个记录绑定为 `mode: Settings`，再在主体中读取 `mode.enabled`。两种写法都检查记录形状、字段名及字段域；不能仅凭形状相同就认为符号字段已有具体值。涉及未知字段的重复索引关系仍可能保持待决，同一字段声明的重复引用则保留同一身份。

部分未知不妨碍使用已经确定的冲突。例如，两份记录的 `enabled` 尚未知，但已知标签分别为 `true` 与 `false`，它们就不能满足“重复使用同一个记录索引”的实现头。若只有未知字段或 Type 关系尚未证明，则保留待决；检查预算不足也不能当作不相等的证据。
