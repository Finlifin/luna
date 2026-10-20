# Trait 的 comptime/runtime 两相

## Intro

一个类型在编译时和运行时可以提供不同的接口实现。Trait 成员的 phase 归属与函数自身的执行阶段相互独立：属性决定成员属于接口的哪一相，函数前缀决定具体调用在哪个阶段执行。

检查一次调用时，既要确定所需成员属于哪一相并已由对应的 `impl` 或 `comptime impl` 兑现，也要检查该函数与实参满足各自的执行阶段契约。

## 语法

以下复用[全局语法](../grammar/syntax)中的定义、属性、参数、子句和关联项。

~~~text
trait_def -> trait id (:- expr)? clauses? { (assoc_decl | definition | statement)* }
impl_trait_def -> comptime? impl expr for expr clauses? { (assoc_decl | definition | statement)* }
derive_def -> comptime? derive expr for expr clauses?
attr_def -> ^expr definition
function -> pure? comptime? inline? (unsafe | spec | verified | atomic)? fn id ( parameter* ) (-> result)? clauses? (block | (= expr))?
~~~

Trait 内的成员默认属于 runtime phase；带有 `^attrs.comptime_phase`（经 `use attrs.comptime_phase` 导入后可写为 `^comptime_phase`）属性的关联项或方法属于 comptime phase。普通 `impl` 与 `derive` 兑现 runtime phase，`comptime impl` 与 `comptime derive` 兑现 comptime phase。

## 正式定义

### Phase 归属与执行阶段

成员的 trait phase 归属与函数的执行阶段分开检查：

1. **Trait phase 归属**：无 `^attrs.comptime_phase` 属性的关联项和方法属于 runtime phase；带 `^attrs.comptime_phase` 属性的关联项和方法属于 comptime phase。
2. **函数执行阶段**：`comptime fn` 与普通 `fn` 独立决定函数自身的执行阶段，参数前的 `comptime` 标记决定单个参数的阶段。普通 runtime phase 方法可以含有 `comptime` 参数（例如 `Splice.splice`、`LiteralInto.literal_into` 或 `ConcatPatternView.concat_pattern_view_prefix`），也可以声明为在编译时计算的辅助函数；编译器不会因函数带有 `comptime` 前缀或参数就暗自将其改归另一 trait phase。

两相可以分别由普通 `impl` 与 `comptime impl` 实现。未实现的相不能因另一相已经实现就自动视为可用，也不把试执行失败当作改选另一相的依据。

### 关联项与跨相共享

关联项同样按是否带有 `^attrs.comptime_phase` 区分所属相。默认相中的只读签名级关联依赖（如类型或编译时常量关联项）可被 comptime phase 成员的签名引用。

当同一主体和完整接口实例分别提供 `impl` 与 `comptime impl` 时，跨相共享的关联项必须给出一致的结果；共享签名依赖不使另一相的成员自动可调用，也不使运行时对象内容变成编译时常量。

### 共同实现关系与共享 coherence

Trait 的 runtime phase 与 comptime phase 共享同一个实现世界、结构精度排序与[一致性检查](../traits/coherence.md)。不能仅因实现相不同，就允许两个同精度实现产生重叠冲突。

归一头（normalized head）和适用条件相同的 `impl` 与 `comptime impl` 构成同一实现关系的互补 phase 部分。查询按统一的结构精度选定实现关系后，检查该关系是否已提供所需 phase：若所选最精确实现关系缺少该 phase，直接诊断缺相错误，不会因为缺相而退回到精度较粗的其他实现。

静态作用域内已有实现先完成相应检查；loose 中生成的开放定义按线性可见性使用，并在整体实例化时接受最终 coherence 检查。详细边界见[两阶段检查](./normalization-and-instantiation.md#开放定义)。

### Eq、Hash 与 Splice

| Trait | Phase 与阶段安排 | 在编译时计算中的作用 |
|---|---|---|
| `Eq` / `Hash` 的 comptime 相 | `^comptime_phase` 成员，由 `comptime impl` / `comptime derive` 提供 | 参与编译时值的 query 缓存比较与键处理 |
| `Splice` | 默认 runtime phase，方法 `fn splice(comptime data: Self) -> RuntimeTarget` | 用 comptime 输入实例化 runtime 转换方法 |

`Eq`/`Hash` 的缓存作用不把它们等同于 PTC 逻辑相等证明。`Splice` 的有限自动派生规则见[跨阶段值传递](./quoting-and-splicing.md#有限自动派生)。

## 常见示例

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

需要为两相分别声明不同关联类型时，也可以在关联项上标注 `^comptime_phase`：

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

1. 在 trait 成员表上分别记录成员的 phase 归属（是否带有 `^attrs.comptime_phase`）与函数自身的执行阶段（`comptime fn` 或 `fn`）及参数阶段标记。
2. 普通 `impl`/`derive` 匹配并检查 runtime phase 成员；`comptime impl`/`comptime derive` 匹配并检查 comptime phase 成员。
3. 对只读签名关联依赖检查跨相可见性与互补实现间的一致性。
4. 在统一的索引与 coherence 体系中检查两相实现的包归属、结构精度和同精度重叠；归一头与条件相同的互补 phase 合并为共同实现关系，选定后不因缺相回退到较粗实现。
5. 为 comptime 调用建立求值依赖；为 runtime 调用建立相应实例及结果契约。

不变量：缺少某相的实现即缺少对应能力，不能通过忽略 phase 属性、忽略函数阶段或回退到低精度实现来补足。`Eq`/`Hash` 的特殊处理必须维持正常的比较契约；`Splice` 必须进入普通 runtime 方法实例化路径。
