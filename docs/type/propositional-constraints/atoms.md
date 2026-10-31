# PTC 原子与性质目录

## Intro

一个原子表达一个不可再由布尔连接词拆分的检查关系。类型相等可以是原子，字段类型则是一个观察结果项；“字段类型等于 u32”由观察与等式共同表达。这种将检查关系分解到原子粒度的方法，使求解器能够独立追踪每个基本事实的依赖与冲突来源，从而支持精确的冲突解释和回溯。

这种区分使公开查询能够持续增加，同时保持求解器核心清晰。新增一个查询名字，不必新增一套相等或整数比较逻辑。

## 语法

公开性质使用普通调用与布尔语法：

~~~text
application -> expr(argument*)
bool_matches -> expr matches pattern
~~~

Type.predicates.is_tuple 与 Type.predicates.is_not_sized 是类型性质入口。以下 A01–A19 是实现说明中的原子模式编号，绝不是公开函数名或新的 Flurry 语法。

## 正式定义

### 性质的解释边界

公开查询必须有确定的输入域与含义。否定表示逻辑否定；未完成查询不能冒充否定。带前提的结构或布局观察，在前提不成立时不返回一个任意结果。

类型分类按不同轴观察：结构类别、标量类别、限定和表示性质不构成一个互斥枚举。无符号整数可以属于整数；struct 也可以具有固定大小。newtype 不因底层是整数就自动获得整数接口或分类。

固定大小性质是表示性质。对有相应合法表示的类型，is_not_sized 为真表示大小除了类型与目标配置，还依赖值相关信息。尚未确定参数或尚未计算大小不是该命题为真；不适用的 comptime-only 类型不能被判成真或假来掩盖前提错误。

### 公开性质覆盖地图

下表的中文描述是语义对象，不额外规定 API 拼写。

| 性质组 | 观察范围 | 接合方式 |
|---|---|---|
| 标量 | bool、整数、有符号/无符号、浮点、Symbol；位宽与精度类别 | 分类及参数等式 |
| 结构 | tuple、struct、enum、union、newtype、函数、指针、模块类型 | 分类、合法结构观察 |
| 类型描述 | Type、Any、any、已定义的描述对象 | 宇宙、等式、相容 |
| 参数化 | 保留头、实参位置、实参关系 | 头形状、匹配、等式 |
| 限定 | optional、error、effect、closure、reachability | 限定观察及各自载荷 |
| 字段与变体 | 存在、数量、类型、声明位置、tuple 元素 | 槽观察、序列和等式 |
| 类型关系 | 相等、匹配、相容、子类型、trait 实例 | 各自独立关系 |
| 函数 | 参数数量与类型、结果、阶段、行和环境 | 合法签名观察 |
| 表示 | 固定大小、大小、对齐、偏移、位宽、表示种类 | 布局观察与整数关系 |
| 形状 | 长度、头尾、元素、拼接、广播契约 | 序列、整数、predicate |
| 行 | 成员、缺失、相等、错误行包含与不相交 | 行理论；效应路由另给证据 |
| 位模式 | 固定位、掩码、范围、相交、覆盖 | 位向量与组合查询 |
| 阶段交界 | 宇宙、已定义 proxy、Splice 实例及前提 | 类型观察与实现证据 |
| 行为能力 | Copy、Clone、Drop、Deref、Reference 等既有接口 | 对应 trait 与操作契约 |

“可按位复制”“所有位串是否合法”“某 ABI 兼容”必须先由相应表示或资源契约定义。不能从“大小已知”自动推出，也不能用通用 predicate 节点补出一条语言规则。

## 常见示例

先把几种性质放进一个实际数据声明的检查中：

~~~flurry
typealias Word = u32;
newtype RegisterId = u32;

struct InstructionOperands {
    destination: RegisterId,
    sources: Tuple<u32, u32>,
}

comptime {
    assumes Word == u32;
    assumes RegisterId != u32;
    assumes Type.predicates.is_tuple(Tuple<u32, u32>);
    assumes not Type.predicates.is_tuple(u32);
    assumes not Type.predicates.is_not_sized(InstructionOperands);
    assumes Type.size_of(*InstructionOperands) == Type.size_of(usize);
}

test {
    let operands = InstructionOperands {
        destination: 1u32.as(RegisterId),
        sources: (2u32, 3u32),
    }
    let (left, right) = operands.sources;
    println("sources: {}, {}", left, right);
    println("operand storage: {}", Type.size_of(InstructionOperands));
}
~~~

输出源寄存器 `2`、`3`，以及目标相关的结构大小。编译时检查分别涉及类型等式、分类、表示性质和布局观察。最后一项先取得两个大小结果，再比较它们；“大小相等”不需要独立于 Eq 的新原子。

| 源码检查 | 内部关系 |
|---|---|
| `Word == u32` | 类型域 Eq |
| `RegisterId != u32` | 名义类型不等事实 |
| `is_tuple(...)` | 合法类型分类 |
| `not is_not_sized(...)` | 适用域内的表示性质否定 |
| 两个 `size_of` 结果相等 | 带前提的观察项 + 数值等式 |

可执行 predicate 提供另一个入口：

~~~flurry
comptime predicate valid_slot(index: Integer, length: Integer) {
    0 <= index and index < length
}

pure comptime fn select_slot(index: Integer, length: Integer) -> Integer
where requires valid_slot(index, length) {
    index
}

comptime {
    assumes select_slot(2, 4) == 2;
    assumes not valid_slot(4, 4);
}
~~~

predicate 调用按其稳定定义接入命题；展开后由布尔合取与整数比较处理。`valid_slot` 是本例定义的谓词，不需要为它新增一种专属核心原子。第二项的否定来自具体边界关系，不来自“没有查到成功结果”。

若把布局查询的输入换成不适用的 comptime-only 类型，应先报告观察前提错误，不能让 `is_not_sized` 的真假掩盖这一错误。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 的 Sized 和 C++ 类型性质提供真实的表示或分类查询；Zig 提供类型反射，C 提供 sizeof、对齐和静态断言。Flurry 将固定大小性质交给 PTC，并区分未知与值相关大小，不能复制 Rust 对泛型参数的默认 Sized 约束。[Rust Sized](https://doc.rust-lang.org/reference/special-types-and-traits.html#sized)、[C++ 类型性质](https://eel.is/c++draft/meta.unary.prop)

## 运行时实现设计引导

查询结果为静态事实时可消去；布局观察的结果用于分配、字段寻址或调用约定等场景。因此，必须记录结果对应的目标配置和必要环境，不能跨目标或对象依赖错误复用。

成员存在事实不是成员地址；效应包含事实不是 handler 入口。需要执行信息的操作应携带实际证据。

## 编译器实现引导

### 核心原子模式全集

参数名为内部逻辑记号。Eq 的 sort 不新增语言宇宙；world 表示查询所依赖的合法实现环境。

| 编号 | 模式 | 具体含义与主责 | 否定与残余边界 |
|---|---|---|---|
| A01 | Eq(sort,a,b) | 该逻辑域的语义相等；EUF 与专门理论 | 节点不同不等于值不同 |
| A02 | Kind(T,k) | 已定义的类型分类；TYPE | 仅正式互斥类别可互相排除 |
| A03 | InUniverse(T,u) | 既有宇宙归属；TYPE | 不默认两个宇宙互为否定 |
| A04 | Qualified(T,q) | 规范描述具有指定限定；TYPE | 载荷另作项；只用该限定的消去规则 |
| A05 | HasSlot(T,s) | 指定字段、位置、变体存在；TYPE/SEQ | s 区分选择器种类；开放信息不能判缺失 |
| A06 | HasHeadShape(n,h) | 保留对象 n 具有来源头 h；MATCH | 不是任意工厂逆运算；不沿语义等式盲传 |
| A07 | Matches(d,x,p) | 在域 d 中 x 满足 p；MATCH/SEQ/BV | 成功绑定另给证据；否定不产生绑定 |
| A08 | Compatible(d,a,b) | 类型或值域的部分信息相容；REL | 不是等式，不取传递闭包 |
| A09 | Subtype(A,B) | 已定义的方向性类型关系；REL | 不从可转换推导，不自动生成协变 |
| A10 | Implements(x,t,world) | 主体满足指定 trait 实例；TRAIT | 否定需要完整有效的排除依据 |
| A11 | NotSized(T,target) | 适用表示依赖值信息决定大小；LAYOUT | 未知输入不为真；非法表示是前提错误 |
| A12 | IntLe(a,b) | 数学整数小于等于；INT | 与机器整数语义分开 |
| A13 | IntCongruent(a,r,m) | 正常数模数下同余；INT | 未知/非正 m 不在域内；可降整数见证 |
| A14 | BVUnsignedLe(a,b,w) | 已知正宽度、同宽无符号比较；BV | 宽度或操作契约不足则残余 |
| A15 | BVSignedLe(a,b,w) | 同宽有符号比较；BV | 不能与无符号解释混用 |
| A16 | RowContains(k,R,l) | 行含标签 l；ROW | 开放尾部未知不判缺失；不提供路由 |
| A17 | RowSubset(error,R,S) | 唯一错误行的包含；ROW | 不自行产生值转换；不是通用效应嵌入 |
| A18 | RowDisjoint(error,R,S) | 唯一错误行没有共同标签；ROW | 可选派生关系；不用于决定 handler 层 |
| A19 | PredicateCall(def,args,deps) | 稳定的准入可执行命题；PRED | 可用事实、展开或执行；无规则不是假 |

A13 与 A18 可以降到其他公式；列入目录是为了稳定实现职责，不声称它们在逻辑上不可定义。A17 不用于 scoped effect 行的通用子集公理；效应嵌入必须保留 occurrence 路由。

### 项与操作

字段类型、布局大小、列表长度、限定载荷和关联类型都是项。它们的形成依赖各自前提；形成后才参与 Eq 或其他原子。

实现选择、接收者适配、构造、行 split 和效应 embedding 是操作。它们可以生成布尔事实，但还要返回来源、替换、载荷映射或路由，不能只留下真假。

### 原子注册与审计

每种原子登记：sort、良定义域、具体解释、正负传播规则、依赖身份、冲突解释、覆盖片段、残余条件。新的公开谓词首先尝试展开到已有公式；只有语义需要独立理论关系时才增加核心类别。

验收至少覆盖正例、负例、输入不足、前提非法、作用域变化和回退。不能用只调用一次成功路径的测试证明原子可靠。
