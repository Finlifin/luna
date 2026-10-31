# Trait 的 comptime/runtime 两相

## Intro

一个类型在编译时和运行时可以提供不同的接口实现。Trait 成员的 phase 归属与函数自身的执行阶段相互独立：普通 trait 用成员属性区分两相，`pure comptime trait` 则只包含 comptime phase 的成员；函数前缀仍独立决定具体调用在哪个阶段执行。

检查一次调用时，既要确定所需成员属于哪一相并已由对应的 `impl` 或 `comptime impl` 兑现，也要检查该函数与实参满足各自的执行阶段契约。

## 语法

以下复用[全局语法](../grammar/syntax)中的定义、属性、参数、子句和关联项。

~~~text
trait_def -> (pure comptime)? trait id (:- expr)? clauses? { (assoc_decl | definition | statement)* }
impl_trait_def -> comptime? impl expr for expr clauses? { (assoc_decl | definition | statement)* }
derive_def -> comptime? derive expr for expr clauses?
attr_def -> ^expr definition
function -> pure? comptime? inline? (unsafe | spec | verified | atomic)? fn id ( parameter* ) (-> result)? clauses? (block | (= expr))?
~~~

普通 trait 内的成员默认属于 runtime phase；带有 `^attrs.comptime_phase`（经 `use attrs.comptime_phase` 导入后可写为 `^comptime_phase`）属性的关联项或方法属于 comptime phase。普通 `impl` 与 `derive` 兑现 runtime phase，`comptime impl` 与 `comptime derive` 兑现 comptime phase。

`pure comptime trait` 将整个接口限定为 comptime phase；其中 `pure` 的意思是 **only**，不是函数纯度或无副作用承诺。该修饰不引入单独的 `comptime trait` 或 `pure trait` 声明形式。

## 正式定义

### Phase 归属与执行阶段

成员的 trait phase 归属与函数的执行阶段分开检查：

1. **Trait phase 归属**：普通 trait 中，无 `^attrs.comptime_phase` 属性的关联项和方法属于 runtime phase，带属性的成员属于 comptime phase；`pure comptime trait` 的所有接口成员都属于 comptime phase。
2. **函数执行阶段**：`comptime fn` 与普通 `fn` 独立决定函数自身的执行阶段，参数前的 `comptime` 标记决定单个参数的阶段。普通 runtime phase 方法可以含有 `comptime` 参数（例如 `Splice.splice`、`LiteralInto.literal_into` 或 `ConcatPatternView.concat_pattern_view_prefix`），也可以声明为在编译时计算的辅助函数；编译器不会因函数带有 `comptime` 前缀或参数就暗自将其改归另一 trait phase。

两相可以分别由普通 `impl` 与 `comptime impl` 实现。未实现的相不能因另一相已经实现就自动视为可用，也不把试执行失败当作改选另一相的依据。

### 仅含 comptime phase 的接口

`pure comptime trait` 为只在 comptime 一侧提供的协议建立独立名义身份。它的
方法、关联项和默认成员都归入 comptime phase，无需逐项写
`^attrs.comptime_phase`；重复标注该属性不改变含义。不能通过成员属性向该接口
添加 runtime phase 成员。

实现仍显式使用 `comptime impl`，派生仍使用 `comptime derive`。普通 `impl`
或 `derive` 不能为它建立空的 runtime 相，也不自动改写为 comptime 形式。
成员声明和实现必须符合其接口签名与契约；父接口要求在所需 comptime phase
下取得证据，不因被此接口引用就为普通父 trait 自动补出 comptime 实现。

这一修饰**不隐式改写函数执行阶段**。方法要在 comptime 执行，仍须在签名中
声明 `comptime fn`；参数阶段、资源约束和实际副作用继续按函数本身检查。
`pure comptime trait` 不给方法增加 `pure`，也不要求方法或实现无副作用，
更不使其结果自动可缓存。普通 trait 的两相机制继续保留，不必把所有混合接口拆开。

### 同名方法的 comptime 来源优先级

一个类型同时具有 runtime 与 comptime 用途时，两套同名方法放在**两个独立
trait 的实现**中。名字不必加 `comptime_` 前缀；也不在同一个 trait 的成员表中
用两个 phase 注册同名方法。两份 trait 的身份、关联项和契约彼此独立。
可选类型和错误限定类型的两阶段同名方法采用这一组织方式，而不是为它们建立
同名的跨阶段固有方法。

对于未显式指定接口的成员查找，在当前表达式的 comptime 上下文中：

1. 按普通可见性、泛型能力、主体与完整接口实例收集合法的 trait 成员来源。
   `pure comptime trait` 的可用实现组成优先层；其余 trait 来源组成后续层。
   不按接口名称、`Comptime` 前缀、导入次序或定义次序决定优先级。
2. 优先层有唯一合法来源时，固定该来源及其完整接口实例；若同层仍有多个不同
   来源，则保留歧义，要求显式视角或足够的类型证据。不同 trait 之间不比较
   implementation 结构精度来消歧。
3. 优先层的竞争者尚未决时，查询等待；不能把 Unknown、预算耗尽或尚未完成
   的实例化当作“不存在”。只有这一层没有合法来源且没有未决竞争者时，才按
   原有规则继续考虑后续层。
4. 来源选定后，同一接口内部仍按统一的结构精度与 coherence 规则选择实现。
   缺少所需 phase、签名不匹配、契约失败或执行失败都直接按该来源诊断；不能
   通过改选普通 trait 的同名方法使调用通过。

这里是**trait 成员来源**的确定性规则，不是跨 trait 的实现特化，也不改变
字段、类型自身成员和词法名称的原有查找层次。普通 trait 中逐项标注为
comptime phase 的成员不自动获得“整个 trait 仅限 comptime”的优先身份。

`value.as(Trait).method()` 已明确来源，不参加上述优先层竞争；其 phase 与
函数执行阶段仍须合法。`T :- Trait`、父 trait 要求、具名关联项投影和已经绑定
到具体接口的语法协议同样保持原接口身份，不能暗中替换成另一个同名协议。
缓存 Eq/Hash、运算符等入口如需采用新的接口，应明确绑定对应的完整 trait，
不能仅凭方法名字自动配对两份接口。

这里的上下文由**当前表达式的求值阶段**决定，不只看最外层函数名称。
runtime 函数中的 comptime 子表达式按 comptime 规则解析；编译期生成但实际
留待 runtime 执行的代码仍按 runtime 规则检查其成员调用。runtime 成员查找
不使用 pure comptime trait 的实现来填补缺失能力。

泛型定义只使用定义处合法可见、已知或待完成的能力。成员来源一旦固定，后续
实例化只兑现这一来源；不能因为具体类型后来暴露了另一份 pure comptime
trait 实现，就重新解释已检查的调用。

### 关联项与跨相共享

关联项同样按是否带有 `^attrs.comptime_phase` 区分所属相。默认相中的只读签名级关联依赖（如类型或编译时常量关联项）可被 comptime phase 成员的签名引用。

当同一主体和完整接口实例分别提供 `impl` 与 `comptime impl` 时，跨相共享的关联项必须给出一致的结果；共享签名依赖不使另一相的成员自动可调用，也不使运行时对象内容变成编译时常量。

### 共同实现关系与共享 coherence

Trait 的 runtime phase 与 comptime phase 共享同一个实现世界、结构精度排序与[一致性检查](../traits/coherence.md)。不能仅因实现相不同，就允许两个同精度实现产生重叠冲突。

这说的是同一 trait 内部的两相。pure comptime trait 只有 comptime 相；它与另一份普通 trait 的同名成员不构成互补实现关系，不要求两份独立接口的关联项结果相同。

归一头（normalized head）和适用条件相同的 `impl` 与 `comptime impl` 构成同一实现关系的互补 phase 部分。查询按统一的结构精度选定实现关系后，检查该关系是否已提供所需 phase：若所选最精确实现关系缺少该 phase，直接诊断缺相错误，不会因为缺相而退回到精度较粗的其他实现。

静态作用域内已有实现先完成相应检查；loose 中生成的开放定义按线性可见性使用，并在整体实例化时接受最终 coherence 检查。详细边界见[两阶段检查](./normalization-and-instantiation.md#开放定义)。

### Eq、Hash 与 Splice

| Trait | Phase 与阶段安排 | 在编译时计算中的作用 |
|---|---|---|
| `Eq` / `Hash` 的 comptime 相 | `^comptime_phase` 成员，由 `comptime impl` / `comptime derive` 提供 | 参与编译时值的 query 缓存比较与键处理 |
| `Splice` | 默认 runtime phase，方法 `fn splice(comptime data: Self) -> RuntimeTarget` | 用 comptime 输入实例化 runtime 转换方法 |

`Eq`/`Hash` 的缓存作用不把它们等同于 PTC 逻辑相等证明。`Splice` 的有限自动派生规则见[跨阶段值传递](./quoting-and-splicing.md#有限自动派生)。

同名的编译时与运行时比较／哈希操作可以分别放在普通 trait 与
`pure comptime trait` 中，例如两份独立接口都声明 `eq` 或 `hash`。
这不使两个接口等同，也不改变缓存所需的 Eq/Hash 契约。

内置库分别由 `builtin.Eq` / `builtin.Hash` 提供 runtime 接口，由
`meta.ComptimeEq` / `meta.ComptimeHash` 提供独立的 `pure comptime trait` 接口。
两阶段的方法名均为 `eq`、`ne`、`hash`，meta 的 prelude 导出编译时接口。
缓存消费者显式绑定编译时接口的声明身份；绑定到 runtime 接口不会自动替换为
另一接口。所绑定成员的 comptime phase、函数阶段和实现条件仍须检查。

## 常见示例

同一种可选值在两个宇宙中使用相同的方法名：

~~~flurry
trait OptionalOps where T {
    fn is_some(self) -> bool;
}

pure comptime trait ComptimeOptionalOps where T {
    comptime fn is_some(self) -> bool;
}

impl OptionalOps<T> for ?T where T {
    fn is_some(self) -> bool = self match {
        _? => true,
        null => false,
    };
}

comptime impl ComptimeOptionalOps<T> for ?T where T {
    comptime fn is_some(self) -> bool = self match {
        _? => true,
        null => false,
    };
}

fn runtime_check(value: ?bool) -> bool { value.is_some() }
comptime fn compiletime_check(value: ?bool) -> bool { value.is_some() }

comptime fn explicit_check(value: ?bool) -> bool {
    value.as(ComptimeOptionalOps<bool>).is_some()
}
~~~

`runtime_check` 使用 OptionalOps；`compiletime_check` 在 comptime 查找中优先
选中 ComptimeOptionalOps。`explicit_check` 直接指定该来源，不经过同名优先级
选择。两份 `is_some` 都保持自己的阶段契约；不能因为输入 bool 可在两阶段存在，
就让 runtime 方法在 comptime 被隐式执行。

如果再引入另一份适用的 pure comptime trait，也提供 `is_some`，非限定调用
仍有歧义；“优先 comptime-only 接口”不意味着任意选择一份这样的接口。
上述声明是本次确认的设计示例，当前 parser、成员选择及内置包仍需按此迁移。


下面的 `ReadCode` 同时包含默认 runtime phase 成员、带 `^comptime_phase` 的 comptime phase 成员，以及跨相共享的只读签名关联类型 `Code`：

~~~flurry
use attrs.comptime_phase;

trait ReadCode {
    assoc Code: Type = u32;

    fn read_code(*self) -> Code;

    ^comptime_phase
    comptime fn static_code(self) -> Code;
}

struct DeviceTag {
    code: u32,
}

impl ReadCode for DeviceTag {
    fn read_code(*self) -> u32 {
        self.code.*
    }
}

comptime impl ReadCode for DeviceTag {
    ^comptime_phase
    comptime fn static_code(self) -> u32 {
        self.code
    }
}

test {
    let runtime_tag = DeviceTag { code: 7 }
    let comptime_tag = comptime {
        let tag = DeviceTag { code: 42 }
        tag.static_code()
    }
    println("{} {}", runtime_tag.read_code(), comptime_tag);
}
~~~

输出 `7 42`。`read_code` 属于默认的 runtime phase，由 `impl ReadCode for DeviceTag` 提供；`static_code` 带有 `^comptime_phase` 属性，属于 comptime phase，由 `comptime impl ReadCode for DeviceTag` 提供。两份互补实现共享关联类型 `Code = u32`。若移除 `comptime impl ReadCode for DeviceTag`，编译时调用 `tag.static_code()` 会因缺少 comptime phase 实现而被拒绝，不会拿 runtime phase 顶替。

需要为两相分别声明不同关联类型时，也可以在关联项上标注 `^comptime_phase`。
下面是自定义混合协议示例；内置列表模式协议已分别采用 runtime trait 和
`pure comptime trait ComptimeListPatternView`，两者使用同名方法与 `Element`：

~~~flurry
use attrs.comptime_phase;

trait ListPatternView {
    assoc Element: Type;
    fn list_pattern_view_get_len(*self) -> usize;
    fn list_pattern_view_get_element(*self, comptime index: usize) -> Element;

    ^comptime_phase
    assoc ElementComptime: Type;
    ^comptime_phase
    comptime fn list_pattern_view_get_len_comptime(self) -> usize;
    ^comptime_phase
    comptime fn list_pattern_view_get_element_comptime(self, index: usize) -> ElementComptime;
}
~~~

注意 `list_pattern_view_get_element` 虽然带有 `comptime index: usize` 参数，但未标注 `^comptime_phase`，因此它属于默认 runtime phase 的方法，在普通 `impl ListPatternView for ...` 中实现。

## 与 Rust / Zig / C / C++ 的关键差异

| 对照语言 | 对应机制与差异 |
|---|---|
| Rust | trait 可包含方法、关联类型与关联常量，无独立的 trait phase 属性与 `comptime impl` 分相机制。[Rust traits](https://doc.rust-lang.org/reference/items/traits.html) |
| Zig | 无对等的名义 trait 两相体系；通过 `comptime` 参数与编译时结构检查组织所需操作。[Zig 参考](https://ziglang.org/documentation/master/#Compile-Time-Parameters) |
| C11 | 无对等的 trait；接口可用函数表和约定实现，类型相关元数据另设常量。[C11 草案 §6.7.6.3](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf) |
| C++ | concepts 与受约束模板表达操作要求，成员函数可有 `constexpr`/`consteval` 限定；Flurry 将成员 phase 归属（`^comptime_phase`）与函数执行阶段分开，并令两相共享 coherence。[C++ constraints](https://eel.is/c++draft/temp.constr)、[constexpr](https://eel.is/c++draft/dcl.constexpr) |

## 运行时实现设计引导

1. 为已检查的 runtime phase 方法调用使用对应实例及其调用约定；擦除仅用于实例化的 `comptime` 参数，保留 runtime 参数与返回值。
2. comptime phase 成员供编译时查询与求值使用，不进入动态接口（`dyn Trait`）的运行时分派表。
3. runtime 方法所需的常量、类型布局或代码结构必须已由编译时过程确定。

不能因一个 trait 含有 `^comptime_phase` 成员就为普通调用引入运行时 comptime engine；也不能因两相共享 coherence 就假定所有 trait 调用采用同一种派发方式。

## 编译器实现引导

1. 在 trait 声明上记录是否仅含 comptime phase，并据此建立成员的 phase 归属；与函数自身的执行阶段（`comptime fn` 或 `fn`）及参数阶段分别记录。此身份进入稳定接口描述，不能靠成员名或可变查询缓存猜测。
2. 普通 `impl`/`derive` 匹配并检查 runtime phase 成员；`comptime impl`/`comptime derive` 匹配并检查 comptime phase 成员。
3. 对只读签名关联依赖检查跨相可见性与互补实现间的一致性。
4. 在统一的索引与 coherence 体系中检查两相实现的包归属、结构精度和同精度重叠；归一头与条件相同的互补 phase 合并为共同实现关系，选定后不因缺相回退到较粗实现。
5. 非限定的 comptime 成员查找按上述两层来源规则消歧，保留未决候选，选定后固定接口身份；显式接口查询跳过此排序。
6. 为 comptime 调用建立求值依赖；为 runtime 调用建立相应实例及结果契约。

不变量：缺少某相的实现即缺少对应能力，不能通过忽略 phase 属性、忽略函数阶段或回退到低精度实现来补足。`Eq`/`Hash` 的特殊处理必须维持正常的比较契约；`Splice` 必须进入普通 runtime 方法实例化路径。
