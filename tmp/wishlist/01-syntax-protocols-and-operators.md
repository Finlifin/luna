# 01. 语法制导协议与运算符全集（Syntax Protocols & Operators Wishlist）

> 本文档是 Flurry 语言标准库（`builtin`、`meta`、`std`、`attrs`）与 `luna` 编译器前端/elaboration/降码对齐的 **Wave 1 规格愿望单 #01**，定义全部运算符、字面量后缀、索引与空间应用、解引用/取址、范围、比较、哈希、转换、`Splice`、`|>`/`==>`/`and`/`or`/`not`、两相（runtime / comptime）协议策略及 `language-items.conf` 语言项注册表。

---

## 1. 范围、权威来源与归属包

### 1.1 文档范围与边界分工

本文档完整覆盖 Flurry 所有语法制导协议（syntax-guided protocols）与运算符语义：

1. **二元/一元算术与复合赋值**：`Add`、`Sub`、`Mul`、`Div`、`Rem`、`Neg`、`Not`，以及复合赋值 `+=`、`-=`、`*=`、`/=`、`%=`（`AddAssign`、`SubAssign`、`MulAssign`、`DivAssign`、`RemAssign`、求值顺序与 `*self` 写回契约）。
2. **位运算协议对账**：`library/builtin/protocols/ops.fl` 现存 `BitAnd`/`BitOr`/`BitXor`/`Shl`/`Shr` 与 `docs/grammar/index.md`（无传统位运算符、`BitVec` 上的急切 `and`/`or`/`not`、`builtin.math.bit_*`）的冲突调和。
3. **拼接运算符 `++`**：`Concat` 与 `meta.ops.ComptimeConcat`（服务 `BitVec`、`Array`、`str`、`meta.List`、`Tuple`、`Slice`、`std.collections.Vec`、`std.string.String`）。
4. **Range 全系列与 `RangeBounds`**：`..`、`..expr`、`..=expr`、`expr..`、`expr..expr`、`expr..=expr` 对应的六种范围类型、`Bound<T>`、`RangeBounds<T>`、`ComptimeRangeBounds<T>` 及整数范围 `Iterator` 接入。
5. **比较与排序**：`Eq`、`Ord`、`Order`、接收者引用语义修正、`PartialEq`/`PartialOrd` 路线取舍、IEEE 754 浮点 NaN 边界与 `TotalOrd`。
6. **哈希协议族**：`Hash`、`Hasher`、`BuildHasher`、`std.hash.DefaultHasher`、`std.hash.RandomState`，以及 `meta.hash` 两相对偶。
7. **下标、空间应用与可调用边界**：`Index`、`IndexMut`、`SpaceApply`、`VariadicCollector`（含多参 `[...args]` 语法补齐），以及“不引入 `Fn`/`FnOnce`/`FnMut`，由 `%c fn(...) -> R` 与资源契约承担”的闭包边界。
8. **解引用与引用**：`Deref`、`DerefMut`、`Reference` 与 `.*`/`.ref`/接收者适配三层顺序。
9. **字面量后缀协议**：`LiteralInto<suffix, R>`（`Integer`、`Float`、`char`、`str` 四类主体、上下文 `R` 推断与隔离试探、负字面量边界）。
10. **类型转换、默认值与阶段拼接**：`From`、`Into`、`TryFrom`、`TryInto`（**强制迁移至 `!Err T`，全面废除 `?T` 与严禁 `Result`**）、`AsRef`、`AsMut`、`Default`、`Splice`。
11. **管道、蕴含与布尔逻辑**：`|>`（`PipePrepend`）、`==>`（`BoolImplies`）与 `and`/`or`/`not`（`bool` 短路与 occurrence typing vs `BitVec` 急切位求值）。
12. **两相协议与注册表**：`pure comptime trait` 对偶策略 vs `^attrs.comptime_phase`，以及全量 `library/language-items.conf` / `SysrootBindings` / `Protocols` 绑定表。

**跨文档职责边界**（避免重复与越界）：

- 模式匹配视图协议（`ListPatternView`、`MapPatternView`、`RangePatternView`、`ConcatPatternView`、`TuplePatternView`）及 `expr'id` 取视图目录见 `02-pattern-matching-and-views.md`；本文档仅在 `Range` 与 `Concat` 处标注交互接口。
- `Once`/`Clone`/`Copy`/`Drop` 资源基线、原语固有数学/位方法、`Iterator`/`IntoIterator` 完整适配器族见 `04-builtin-types-math-iterators-fmt.md`；本文档仅定义运算符所需的协议绑定与整数范围 `Iterator` 对接。
- `meta.data.List`、CMTT 与 `Type.predicates` 详见 `05-meta-cmtt-macros-reflection.md`；本文档定义所有运算符/转换协议在 `meta` 中的 `pure comptime trait` 对偶。
- `std.mem.Allocator` 范式、`Vec`/`HashMap`/`String` 完整 API 见 `07-std-mem-collections-io-os.md`；本文档规定支撑容器哈希与比较的 `BuildHasher`/`DefaultHasher`/`Eq`/`Hash`/`IndexMut` 契约，并严守三层包架构：不在六个内置包中引入 `regex`、`serialization`、加密哈希（SHA/BLAKE3）等 Tier 2/3 内容（`Tensor` 等仅作 Tier 2/3 使用者案例演示 `SpaceApply`）。

### 1.2 权威依据等级

按 `AGENTS.md` 规定严格排序：

1. **最高权威**：作者确认结论与 `AGENTS.md`（§5 不得破坏的语义边界、六个内置包规则、无 `Option`/`Result`、comptime 无指针值引用语义）。
2. **正式规范文档（`docs/`）**：
   - 协议与阶段：[docs/traits/protocols.md](../../docs/traits/protocols.md)、[docs/comptime/the-comptime-phases-of-traits.md](../../docs/comptime/the-comptime-phases-of-traits.md)、[docs/traits/declarations.md](../../docs/traits/declarations.md)、[docs/traits/implementation-resolution.md](../../docs/traits/implementation-resolution.md)、[docs/traits/coherence.md](../../docs/traits/coherence.md)、[docs/traits/derivation.md](../../docs/traits/derivation.md)
   - 类型与接收者适配：[docs/type/receiver-adaptation.md](../../docs/type/receiver-adaptation.md)、[docs/type/member-projection.md](../../docs/type/member-projection.md)、[docs/type/propositional-constraints/index.md](../../docs/type/propositional-constraints/index.md)
   - 语法、字面量与基础类型：[docs/grammar/index.md](../../docs/grammar/index.md)、[docs/grammar/syntax](../../docs/grammar/syntax)、[docs/basics/literals.md](../../docs/basics/literals.md)、[docs/basics/primitives.md](../../docs/basics/primitives.md)、[docs/program-synthesis/bitvec-algebra.md](../../docs/program-synthesis/bitvec-algebra.md)
   - 函数、闭包、控制流与错误/可选：[docs/functions/higher-order-functions.md](../../docs/functions/higher-order-functions.md)、[docs/functions/closure-environments.md](../../docs/functions/closure-environments.md)、[docs/functions/calls-and-returns.md](../../docs/functions/calls-and-returns.md)、[docs/control-flow/basic.md](../../docs/control-flow/basic.md)、[docs/control-flow/pattern-matching/index.md](../../docs/control-flow/pattern-matching/index.md)、[docs/algebraic-effect-and-delimited-continuation/error-handling.md](../../docs/algebraic-effect-and-delimited-continuation/error-handling.md)、[docs/type/qualified-types/optional-type.md](../../docs/type/qualified-types/optional-type.md)
   - 资源与阶段：[docs/resource-management/index.md](../../docs/resource-management/index.md)、[docs/resource-management/bindings.md](../../docs/resource-management/bindings.md)、[docs/resource-management/pointers.md](../../docs/resource-management/pointers.md)、[docs/comptime/quoting-and-splicing.md](../../docs/comptime/quoting-and-splicing.md)、[docs/comptime/official-attribute-keys.md](../../docs/comptime/official-attribute-keys.md)、[docs/code-organization/builtin-packages.md](../../docs/code-organization/builtin-packages.md)
3. **已确认讨论记录（`docs_chat_history/`）**：`037`（早期立场，已被 `047`/`048` 推翻）、`044`、`046`、`047`、`048`（协议表、`LiteralInto`、`SpaceApply`、`Deref`/`Reference` 适配扩展）、`056`（不引入 `FnOnce`/`FnMut`）。
4. **非权威现状/近期草案（仅用于盘点缺口，不作语义权威）**：`library/`、`compiler/`、`proposal_trace/` 中**仅限 `2026-5-*` 至 `2026-10-*` 期间的近期草案**（`proposal_trace/2026-9-7-tensor-algebra.fl`、`proposal_trace/2026-9-13-receiver-adaption.fl`、`proposal_trace/2026-9-14-literal-variant-without-extension.fl`）；严禁引用任何 `2024-*`、`2025-*`、`2026-1..4-*` 草案或 `flurry_tutorial/`。

### 1.3 内置包归属矩阵

严格遵守六个内置包（`builtin`、`meta`、`build`、`attrs`、`std`、`verification`），以及`.symbol'builtin` 集中绑定原则：

| 协议 / 类型族 | Runtime 归属模块 | Comptime 归属模块 | Prelude 导出策略 | `.symbol'builtin` 绑定位置 |
|---|---|---|---|---|
| `Add..Rem`, `Neg`, `Not`, `AddAssign..RemAssign`, `Concat`, `Index`, `IndexMut`, `SpaceApply`, `VariadicCollector`, `Deref`, `DerefMut`, `Reference` | `builtin.protocols.ops`（建议重导出为 `builtin.ops`） | `meta.ops` (`ComptimeAdd..ComptimeRem`, `ComptimeNeg`, `ComptimeNot`, `ComptimeAddAssign..ComptimeRemAssign`, `ComptimeConcat`, `ComptimeIndex`) | `builtin.prelude.*`（自动导入）；`meta.prelude.*`（自动导入） | 仅集中于 `library/builtin/primitives.fl`、`library/builtin/bindings.fl` 与 `library/meta/bindings.fl` |
| `BitAnd`, `BitOr`, `BitXor`, `Shl`, `Shr`（待裁决保留为纯方法 trait 或移除） | `builtin.protocols.ops` | 无独立运算符对偶（若保留则添 `Comptime*`） | 若移除则退出 prelude | 无语言项绑定 |
| `RangeFull`, `RangeFrom`, `RangeTo`, `RangeToInclusive`, `Range`, `RangeInclusive`, `Bound`, `RangeBounds` | `builtin.range` | `meta.range.ComptimeRangeBounds`（Range 结构体本身为跨阶段数据，可在两相构造） | `builtin.prelude.*` / `meta.prelude.*` | 若需内置整数步进辅助由 `builtin/bindings.fl` 集中绑定 |
| `Eq`, `Ord`, `Order`, `TotalOrd` | `builtin.numeric.comparison`（建议重导出为 `builtin.cmp`） | `meta.cmp` (`ComptimeEq`, `ComptimeOrd`, `ComptimeTotalOrd`) | `builtin.prelude.*` / `meta.prelude.*` | `builtin/bindings.fl` |
| `Hash`, `Hasher`, `BuildHasher` | `builtin.hash`（取代 `builtin.protocols.traits.Hash`） | `meta.hash` (`ComptimeHash`, `ComptimeHasher`, `ComptimeBuildHasher`, `DefaultComptimeHasher`) | `builtin.prelude.*` / `meta.prelude.*` | 无 |
| `DefaultHasher`, `RandomState`（SipHash-1-3 随机种子实现） | `std.hash` | 无（comptime 不需随机 DoS 防护，使用 `meta.hash.DefaultComptimeHasher`） | 需显式导入 `std.prelude.*` 或 `std.hash.*` | `std` 若需 OS 熵源经 `std.os`/`builtin` 桥接，不使用 `.symbol'builtin` |
| `From`, `Into`, `TryFrom`, `TryInto`, `AsRef`, `AsMut`, `Default`, `LiteralInto`, `InfallibleErr`, `ConversionErr` | `builtin.protocols.conversion`（建议重导出为 `builtin.convert`） | `meta.convert` (`ComptimeFrom`, `ComptimeInto`, `ComptimeTryFrom`, `ComptimeTryInto`, `ComptimeDefault`) | `builtin.prelude.*` / `meta.prelude.*` | 无（`LiteralInto` 由 `language-items.conf` 绑定） |
| `Splice` | `builtin.protocols.staging`（建议重导出为 `builtin.staging`） | 跨阶段桥梁（参数 `comptime data: Self`，产出 `RuntimeTarget`） | `builtin.prelude.*` | 原语实现由 `builtin/protocols/staging.fl` 与绑定层配合 |

---

## 2. 现状与缺口全景总表

> 优先级定义：`P0` = 阻断核心库/自举与基础语义正确性的关键缺口；`P1` = 完成标准库人体工学与两相对称性的核心协议；`P2` = 完善泛型浮点、高阶借用与诊断打磨；`P3` = 演进观察项。

| ID | 符号 / 特性 | 包路径 | 阶段 | 现状 (Current Status) | 优先级 | 编译器依赖 |
|---|---|---|---|---|---|---|
| `WL-01-01` | `Add`, `Sub`, `Mul`, `Div`, `Rem`, `Neg`, `Not` | `builtin.ops` | Runtime | 已声明并绑定至 `language-items.conf`；`operators.rs` 仅部分原生接入，有符号整除/取余与浮点 `/`/`%` 报未实现 | `P0` | elaboration 原生有符号 `/`、`%` 与 Float `/`/`%` 舍入契约补齐 |
| `WL-01-02` | `ComptimeAdd`, `ComptimeSub`, `ComptimeMul`, `ComptimeDiv`, `ComptimeRem`, `ComptimeNeg`, `ComptimeNot` | `meta.ops` | Comptime | **完全缺失**；编译时用户自定义类型无法重载算术与 `not` | `P1` | `pure comptime trait` 查找优先级接线；`language-items.conf` 新增 `comptime-*-protocol`；comptime 解释器分派 |
| `WL-01-03` | `AddAssign`, `SubAssign`, `MulAssign`, `DivAssign`, `RemAssign`（`+=` `-=` `*=` `/=` `%=`） | `builtin.ops` | Runtime | **协议完全缺失**；`places.rs:293-538` 仅展开 local/deref Copy 读写与 BitVec 切片写回，明确拒绝 `vec[i] += 1` 且要求 `Copy` | `P0` | `places.rs` 左值单次定位（含 `IndexMut` / `DerefMut`）；协议查询优先/后备机制与非 `Copy` 原地更新 |
| `WL-01-04` | `ComptimeAddAssign..ComptimeRemAssign` 与编译时复合赋值 | `meta.ops` | Comptime | **完全缺失**；comptime 复合赋值依赖局部可变拷贝，不支持自定义容器原地值引用修改 | `P1` | comptime VM 左值单次定位与值引用 `self` 原地更新执行 |
| `WL-01-05` | `BitAnd`, `BitOr`, `BitXor`, `Shl`, `Shr` | `builtin.ops` | Runtime | 存在于 `ops.fl:51-79` 但无表面运算符、未绑定 `language-items.conf`，与 `docs/grammar/index.md` 冲突 | `P1` | 无需新词法；待裁决从 `ops.fl` 移除或仅作显式方法 trait 并补齐 `BitNot` |
| `WL-01-06` | `BitVec<N>` 上的急切 `and` / `or` / `not` 位代数 | `builtin` | Runtime & Comptime | 已部分接入 `BitVec<N>`，需固化“双边急切求值、无短路、无任意用户重载 `and`/`or`”边界 | `P0` | elaboration 确保 `BitVec` 与 `bool` 的求值及 occurrence typing 分流不混淆 |
| `WL-01-07` | `Concat` 与 `++` 运算符 | `builtin.ops`, `std` | Runtime | `operators.rs:551` 硬编码仅支持 `BitVec<M> ++ BitVec<N>`；`Concat` 缺失，`Array`/`Vec`/`String`/`Slice` 均不可用 `++` | `P0` | `concat-protocol` 注册与 `operators.rs` 通用 trait 分派；保留 `BitVec` 维度加法证明 |
| `WL-01-08` | `ComptimeConcat` 与编译时 `++` | `meta.ops` | Comptime | **完全缺失**；`str` 与 `meta.List<T>` 被迫使用 `.concat()` 方法 | `P1` | `comptime-concat-protocol` 注册；`Tuple`/`meta.List`/`str`/`BitVec` 编译时分发 |
| `WL-01-09` | `RangeFull`, `RangeFrom`, `RangeTo`, `RangeToInclusive`, `Range`, `RangeInclusive`, `Bound` | `builtin.range` | Runtime & Comptime | **完全缺失**；`ast_lowering` 产生 `H.Range`，仅特判于 `BitVec[lo..hi]`、`inline for` 与模式，独立表达式 `0..10` 直接报错 | `P0` | `H.Range` elaboration 构造对应 6 种结构体；保留 `BitVec`/`inline for` 特殊语义不退化 |
| `WL-01-10` | `RangeBounds<T>` 与 `ComptimeRangeBounds<T>` | `builtin.range`, `meta.range` | Runtime & Comptime | **完全缺失**；泛型切片与集合范围查询接口无法定义 | `P1` | 无需特殊语言项；依赖 `Eq`/`Ord` 引用接收者修正及 `assoc` 默认实现降码 |
| `WL-01-11` | 整数 Range 的 `Iterator` / `PureComptimeIterator` 实现 | `builtin.range`, `meta.range` | Runtime & Comptime | **完全缺失**；`for i in 0..n` 缺乏标准 `Iterator` / `IntoIterator` 驱动路径 | `P0` | 整数族 trait 或宏生成标准 impl；确保 `0u8..=255u8` 无溢出及零开销内联优化 |
| `WL-01-12` | `Eq`, `Ord`, `Order` 接收者与作用域修复 | `builtin.cmp` | Runtime | 已声明并绑定，但 `eq(self, other: Self)` 与 `cmp(self, other: Self)` **按值消费参数**，导致 `HashMap` 克隆键、`String` 消费 `self` | `P0` | `operators.rs` 降码 `==`/`!=`/`<`/`<=`/`>`/`>=` 为左侧单次求值+临时 `.ref` 传递（不消费 owner） |
| `WL-01-13` | `ComptimeEq`, `ComptimeOrd` 统一与补全 | `meta.cmp` | Comptime | 已存在（`meta/numeric/{equality,ordering}.fl`），但缺 `pure` 函数标示、缺 `Tuple`/`Range` 等复合类型实现 | `P1` | `comptime-eq-protocol` / `comptime-ord-protocol` 绑定与 comptime operator 查找优先级 |
| `WL-01-14` | IEEE 浮点比较边界与 `TotalOrd`（及 `PartialEq`/`PartialOrd` 裁决） | `builtin.cmp`, `meta.cmp` | Runtime & Comptime | 浮点 `f16`/`bf16`/`f32`/`f64` 未实现 `Eq`/`Ord`，走编译器 `Scalar*`，泛型函数 `T :- Eq/Ord` 拒绝浮点 | `P2` | 补齐 IEEE 754 `total_cmp` intrinsic 与 `TotalOrd`（或按裁决引入 `PartialEq`/`PartialOrd`） |
| `WL-01-15` | `Hash`, `Hasher`, `BuildHasher`（流式哈希与借用接收者） | `builtin.hash` | Runtime | `traits.fl:23-25` 仅有 `fn hash(self) -> u64`（消费 `self` 且硬编码返回 `u64`），无 `Hasher`/`BuildHasher` | `P0` | 移除对单态 `u64` 的假设；`std.collections.HashMap` 同步改用 `*self` 与 `BuildHasher` |
| `WL-01-16` | `DefaultHasher`（SipHash-1-3）与 `RandomState` | `std.hash` | Runtime | **完全缺失**；`HashMap` 使用内部固定 `mix_hash`，无 HashDoS 随机种子保护 | `P1` | 需 OS 随机种子原语或线程局部 entropy（无 OS 时确定性回退） |
| `WL-01-17` | `ComptimeHash`, `ComptimeHasher`, `ComptimeBuildHasher`, `DefaultComptimeHasher` | `meta.hash` | Comptime | `meta.ComptimeHash` 仅固定返回 `u64`；需与值引用 `self` 契约对齐流式哈希或明确确定性分工 | `P1` | 无指针约束下 comptime 状态传递支持 |
| `WL-01-18` | `Index`, `IndexMut` 修正与 `places.rs` 写回 | `builtin.ops` | Runtime | `ops.fl:82-90` 中 **`Index.index(self, idx)` 按值消费 `self` 却返回 `*Output`（悬垂隐患）**；`IndexMut` 未绑定；`places.rs` 不支持 `a[i] = v` | `P0` | `index-protocol` / `index-mut-protocol` 接线；`places.rs` 识别下标左值调用 `index_mut` 并完成写回/drop 旧值 |
| `WL-01-19` | `ComptimeIndex` 与编译时下标 | `meta.ops` | Comptime | **完全缺失**（`Index` 返回 `*Output`，在 comptime 中非法） | `P1` | `comptime-index-protocol` 绑定与 comptime 下标返回值/元素读取分发 |
| `WL-01-20` | `SpaceApply`, `VariadicCollector` 与多参下标解析 | `builtin.ops` | Runtime | **完全缺失**；`docs/grammar/syntax:161` 与 `parse/src/expressions.rs:1502` 仅解析单参数 `[expr]`，不支持 `xs[0..2, 0, ..]` | `P1` | 语法/parser 扩展 `[arg1, arg2, ...]`；elaboration 按 `Index` 优先再 `SpaceApply` + `VariadicCollector` 分发（禁止失败静默回退） |
| `WL-01-21` | 可调用闭包边界（明确无 `Fn`/`FnOnce`/`FnMut`） | `builtin`（语言原生） | Runtime & Comptime | 文档已明确由 `%c fn(...) -> R` + `Type.vars.Closure` + `Once`/环境契约负责，但编译器多态闭包特化与文档对齐需持续守护 | `P0` | `%c fn(...)` closure parameter polymorphism 完整落地；禁止新增 `Fn*` 协议 |
| `WL-01-22` | `Deref`, `DerefMut`, `Reference` 与接收者适配 | `builtin.ops` | Runtime | `docs/traits/protocols.md` 与 `docs/type/receiver-adaptation.md` 已确立规则，但 **`library/` 与编译器完全未定义该三协议**，`.*`/`.ref` 仅对 `*T` 硬编码 | `P1` | `deref-protocol` / `deref-mut-protocol` / `reference-protocol` 接线；三层接收者适配引擎（防循环/未知挂起/只执行选中路径） |
| `WL-01-23` | `LiteralInto<suffix, R>` 声明、配置绑定与四主体覆盖 | `builtin.convert` | Runtime (+ Comptime CTFE) | `literal_conversion.rs` 与 `SysrootBindings`（`type_constructors.rs:571`）已实现，但 **`library/language-items.conf` 未写该键且 `builtin` 无 `LiteralInto` 声明**；`str`/`char`/负字面量未全测 | `P0` | `language-items.conf` 补齐键；隔离试探上下文 `R` 推断；`StringConcat` vs 插值拒绝检查 |
| `WL-01-24` | `From`, `Into`, `TryFrom`, `TryInto`, `InfallibleErr` | `builtin.convert` | Runtime | `conversion.fl` 中 **`TryFrom`/`TryInto` 错误返回 `?Self`/`?Target`**，且缺 `InfallibleErr` 与通用互导 impl | `P0` | 依赖 `assoc Err: Type` 与 `!Err T` 原生错误行；coherence 检查泛型传递 impl |
| `WL-01-25` | `ComptimeFrom`, `ComptimeInto`, `ComptimeTryFrom`, `ComptimeTryInto` | `meta.convert` | Comptime | `meta/numeric/conversions.fl` 中 `ComptimeTryFrom`/`ComptimeTryInto` 同样误用 `?Self`/`?Target`；`ComptimeFrom`/`ComptimeInto` 缺失 | `P1` | comptime 对 `!Err T` 完整支持与 `pure comptime trait` 查找优先级 |
| `WL-01-26` | `AsRef`, `AsMut` | `builtin.convert` | Runtime | **完全缺失**；集合借用转换（`*String -> *str`、`*Vec<T> -> *Slice<T>`）各自散落固有方法 | `P2` | 明确与 `.as(T)` 及 `Reference` 的边界定位 |
| `WL-01-27` | `Default` 与 `ComptimeDefault` | `builtin.convert`, `meta.convert` | Runtime & Comptime | `Default.default_value()` 位于 `traits.fl:197`；`ComptimeDefault` 缺失 | `P1` | `derive Default` 与 `default_value` vs `default` 命名最终裁决 |
| `WL-01-28` | `Splice` 跨阶段拼接与宏/构造校验 | `builtin.staging` | Comptime -> Runtime | 已定义于 `staging.fl` 并绑定 `splice-protocol`；两世界边界、`Once`/`Drop` 资源与容器 `Splice` 派生尚不完整 | `P1` | `quoting-and-splicing.md` 跨阶段合规校验；派生策略 |
| `WL-01-29` | Pipeline `\|>`（`PipePrepend`） | 语言原生语法 | Runtime & Comptime | **严重缺陷**：词法/fmt 已有，但 `ast_lowering` 报不支持；**解析优先级错设为 `80`（高于 `+`/`*`）** | `P1` | 修正 `parse` 与 `fmt` 优先级至低优先级（待裁决 `15` 或 `35`）；`ast_lowering` 改写为 prepend 调用 |
| `WL-01-30` | 逻辑蕴含 `==>`（`BoolImplies`） | 语言原生语法 + PTC | Runtime, Comptime & PTC | **结合性缺陷**：`fmt` 按右结合而 **`parse/expressions.rs:194` 按左结合解析**；`ast_lowering` 未支持（HIR 仅有 `ExprKind.Implication` 存根） | `P1` | 修正 `parse` 为右结合；`ast_lowering` & `elaboration` 接入 `bool` 短路 `(not lhs) or rhs` 与 PTC 布尔原子 |
| `WL-01-31` | 布尔逻辑 `and` / `or` / `not` 短路与 occurrence typing 统一 | 语言原生 + `Not` | Runtime & Comptime | `bool` 短路已有基础；需固化不向用户开放 `and`/`or` 重载、统一 `not` 降级 `Not`/`ComptimeNot` 及 `BitVec` 急切规则 | `P0` | elaboration 确保 `and`/`or` 短路控制流正确生成 occurrence typing 事实 |
| `WL-01-32` | 两相分离策略对齐（`pure comptime trait` vs `^attrs.comptime_phase`） | `builtin`, `meta`, `attrs` | Runtime & Comptime | `docs/comptime/the-comptime-phases-of-traits.md` 已定稿；`library/builtin/protocols/README.md:3` 存过期冲突文案 | `P0` | elaboration 实现 comptime `pure comptime trait` 查找层、同签名阶段一致性与 Unknown 挂起 |
| `WL-01-33` | `library/language-items.conf` & `SysrootBindings` / `Protocols` 全量键表 | `library/`, `luna` | Compiler & Sysroot | 现仅 20 键；**`literal-into-protocol` 编译器已支持却漏配于 `library/language-items.conf`**；缺 25+ 新键与 `--*-protocol` CLI 参数 | `P0` | 扩展 `SysrootBindings`、`Protocols`、`cli.rs`、`package_cli.rs`、`help.rs` 与 `language-items.conf` |

---

## 3. 详细规格设计

> 下列全部 Flurry 代码示例严格遵循正式语言规范：点路径（无 `::`）、原生 `?T` 与 `!Err T`（无 `Option`/`Result`）、`and`/`or`/`not`（无 `&&`/`||`/`!`）、`--` 注释（无 `//`/`/* */`）、多行 `where`/`requires` 左花括号 `{` 另起一行对齐、中缀 `+ - * / < > <= >=` 两侧留白、省略 `}` 后分号、comptime 方法全部采用无指针 `self` 值引用语义、禁止在使用处写 `.symbol'builtin`。

### 3.1 二元/一元算术与复合赋值协议（`Add..Rem`, `Neg`, `Not`, `AddAssign..RemAssign`）

#### 3.1.1 Runtime 算术与一元运算符族（`builtin.ops`）

所有算术与一元协议以 `.Rhs: Type = Self` 支撑同构默认与异构扩展（如 `Duration + Duration`、`Instant + Duration`）：

```flurry
-- 归属：library/builtin/protocols/ops.fl（builtin.ops，由 builtin.prelude 导出）

pub trait Add
where
    .Rhs: Type = Self,
{
    assoc Output: Type;
    fn add(self, rhs: Rhs) -> Output;
}

pub trait Sub
where
    .Rhs: Type = Self,
{
    assoc Output: Type;
    fn sub(self, rhs: Rhs) -> Output;
}

pub trait Mul
where
    .Rhs: Type = Self,
{
    assoc Output: Type;
    fn mul(self, rhs: Rhs) -> Output;
}

pub trait Div
where
    .Rhs: Type = Self,
{
    assoc Output: Type;
    fn div(self, rhs: Rhs) -> Output;
}

pub trait Rem
where
    .Rhs: Type = Self,
{
    assoc Output: Type;
    fn rem(self, rhs: Rhs) -> Output;
}

pub trait Neg {
    assoc Output: Type;
    fn neg(self) -> Output;
}

pub trait Not {
    assoc Output: Type;
    fn invert(self) -> Output;
}
```

#### 3.1.2 Comptime 对偶族（`meta.ops`）

按 [docs/comptime/the-comptime-phases-of-traits.md](../../docs/comptime/the-comptime-phases-of-traits.md)，使用同名方法与独立 `pure comptime trait` 对偶（comptime `self` 为值引用，无指针）：

```flurry
-- 归属：library/meta/ops.fl（meta.ops，由 meta.prelude 导出）

pub pure comptime trait ComptimeAdd
where
    .Rhs: Type = Self,
{
    assoc Output: Type;
    pure comptime fn add(self, rhs: Rhs) -> Output;
}

pub pure comptime trait ComptimeSub
where
    .Rhs: Type = Self,
{
    assoc Output: Type;
    pure comptime fn sub(self, rhs: Rhs) -> Output;
}

pub pure comptime trait ComptimeMul
where
    .Rhs: Type = Self,
{
    assoc Output: Type;
    pure comptime fn mul(self, rhs: Rhs) -> Output;
}

pub pure comptime trait ComptimeDiv
where
    .Rhs: Type = Self,
{
    assoc Output: Type;
    pure comptime fn div(self, rhs: Rhs) -> Output;
}

pub pure comptime trait ComptimeRem
where
    .Rhs: Type = Self,
{
    assoc Output: Type;
    pure comptime fn rem(self, rhs: Rhs) -> Output;
}

pub pure comptime trait ComptimeNeg {
    assoc Output: Type;
    pure comptime fn neg(self) -> Output;
}

pub pure comptime trait ComptimeNot {
    assoc Output: Type;
    pure comptime fn invert(self) -> Output;
}
```

#### 3.1.3 复合赋值协议族（`AddAssign..RemAssign`）与求值顺序

**关键对账**：[docs/resource-management/bindings.md](../../docs/resource-management/bindings.md#L62-L65) 描述 `+=`、`-=`、`*=`、`/=`、`%=` 复用算术运算与普通写回（先定位目标一次，读旧值、求值右侧、执行运算、写回结果）；但编译器当前实现（`compiler/elaboration/src/program/places.rs:293-538`）仅支持 local/deref 处要求 `Copy` 的重写与 `BitVec` 切片写回，拒绝 `vec[i] += 1`，且**完全无法支持非 `Copy` 拥有资源的原地复合赋值**（如 `BigInt`、`Tensor`、`Vec` 包装）。

完整定义原地更新协议族（`builtin.ops` 与 `meta.ops`）：

```flurry
-- Runtime 协议：通过 *self 原地修改，不要求 Self :- Copy，不触发旧值整值搬出再 Drop
pub trait AddAssign
where
    .Rhs: Type = Self,
{
    fn add_assign(*self, rhs: Rhs) -> void;
}

pub trait SubAssign
where
    .Rhs: Type = Self,
{
    fn sub_assign(*self, rhs: Rhs) -> void;
}

pub trait MulAssign
where
    .Rhs: Type = Self,
{
    fn mul_assign(*self, rhs: Rhs) -> void;
}

pub trait DivAssign
where
    .Rhs: Type = Self,
{
    fn div_assign(*self, rhs: Rhs) -> void;
}

pub trait RemAssign
where
    .Rhs: Type = Self,
{
    fn rem_assign(*self, rhs: Rhs) -> void;
}

-- Comptime 对偶（meta.ops）：comptime 无 *self 指针，使用值引用 self 语义更新调用方 var 绑定
pub pure comptime trait ComptimeAddAssign
where
    .Rhs: Type = Self,
{
    comptime fn add_assign(self, rhs: Rhs) -> void;
}

pub pure comptime trait ComptimeSubAssign
where
    .Rhs: Type = Self,
{
    comptime fn sub_assign(self, rhs: Rhs) -> void;
}

pub pure comptime trait ComptimeMulAssign
where
    .Rhs: Type = Self,
{
    comptime fn mul_assign(self, rhs: Rhs) -> void;
}

pub pure comptime trait ComptimeDivAssign
where
    .Rhs: Type = Self,
{
    comptime fn div_assign(self, rhs: Rhs) -> void;
}

pub pure comptime trait ComptimeRemAssign
where
    .Rhs: Type = Self,
{
    comptime fn rem_assign(self, rhs: Rhs) -> void;
}
```

**求值顺序与 `*self` / `IndexMut` / `BitVec` 语义契约**：

1. **左值单次定位**：`place_expr += rhs_expr` 严格先求值 `place_expr` 得到目标存储指针 `ptr: *L`（若 `place_expr` 为 `container[idx]`，先按源序求值 `container` 与 `idx`，调用 `IndexMut.index_mut(container.ref, idx)` 取得 `*L`；若为 `p.*` 则调用 `DerefMut.deref_mut`；若为局部/字段绑定则检查可写性，例如 `let x = 1; x += 2;` 拒绝而 `var x = 1; x += 2;` 允许）。目标定位表达式内的副作用或函数调用**仅执行一次**。
2. **右侧求值先于修改**：随后求值 `rhs_expr` 一次得到临时值 `rhs_val: Rhs`。若 `rhs_expr` 提前返回、失败传播（`!`）或控制流转移（`break`/`continue`），则不执行原地更新或写回，已发生的副作用保持。
3. **分派优先与不静默回退**：
   - 原生数值类型与 `BitVec` 位切片（`word'bitvec[lo..hi] += delta`）走编译器原生求值写回路线；
   - 若 `L :- AddAssign<.Rhs = Rhs>` 唯一适用，直接调用 `L.as(AddAssign<.Rhs = Rhs>).add_assign(ptr, rhs_val)`；
   - 若存在 `Unknown`/候选歧义，则挂起或诊断错误，禁止静默回退；
   - **关于是否保留 `Copy + Add` 隐式展开回退**：若作者确认仅在显式无 `AddAssign` 实现且 `L :- Copy + Add<.Rhs = Rhs>` 且 `requires L.as(Add<.Rhs = Rhs>).Output == L` 时才合成 `ptr.* = L.as(Add<.Rhs = Rhs>).add(ptr.*, rhs_val)`（或彻底要求为原生与标准类型显式实现 `*Assign`，仅通过 `derive` 派生），须在 §5 裁决后统一接线（当前 `bindings.md:62-65` 支持合成路线，但长期对非 `Copy` 不足）。
4. **RHS 推断与编译器缺口**：二元算术 `a + b` 允许从上下文目标或 RHS 对等推断推导出 `Rhs`；无预期或仍多解时报错而非任意猜测。当前 `compiler/elaboration/src/program/operators.rs` 尚缺有符号整除/取余与浮点 `/`/`%` 舍入契约，需与 `04` 协作补齐。

#### 3.1.4 正反用例

```flurry
struct Vec2 {
    x: f64,
    y: f64,
}

impl Add for Vec2 {
    assoc Output: Type = Vec2;

    fn add(self, rhs: Vec2) -> Vec2 {
        Vec2 { x: self.x + rhs.x, y: self.y + rhs.y }
    }
}

impl AddAssign for Vec2 {
    fn add_assign(*self, rhs: Vec2) -> void {
        self.x.* = self.x.* + rhs.x;
        self.y.* = self.y.* + rhs.y;
    }
}

test {
    var acc = Vec2 { x: 1.0, y: 2.0 }
    acc += Vec2 { x: 3.0, y: 4.0 }
    let sum = acc + Vec2 { x: 0.5, y: 0.5 }
}
```

**反例（必须拒绝）**：
- `let x: i32 = 1; x += 2;` -> 拒绝：`let` 绑定禁止直接作整值覆盖/复合赋值目标（[docs/resource-management/bindings.md](../../docs/resource-management/bindings.md#L27)）。
- `let u: u32 = 1; let neg_u = -u;` -> 拒绝：无符号整数未实现 `Neg`。
- 非 `BitVec`/`bool` 且未实现 `Not` 的类型使用 `not value` -> 拒绝：缺少 `Not` 实现。
- `read_only_vec[idx()] += 1`（其中 `read_only_vec` 仅实现 `Index` 未实现 `IndexMut`）-> 拒绝：缺少 `IndexMut` 可写位置协议。

---

### 3.2 `BitAnd` / `BitOr` / `BitXor` / `Shl` / `Shr` 与文档对账

#### 3.2.1 现存代码与正式规范冲突

`library/builtin/protocols/ops.fl:51-79` 当前声明了以下五个 trait（均未写入 `library/language-items.conf`，也未在任何词法/语法文件出现对应表面运算符 `&`、`|`、`^`、`<<`、`>>`）：

```flurry
-- 现存于 library/builtin/protocols/ops.fl:51-79（无对应语言表面运算符）
pub trait BitAnd
where
    .Rhs: Type = Self,
{
    assoc Output: Type;
    fn bit_and(self, rhs: Rhs) -> Output;
}

pub trait BitOr
where
    .Rhs: Type = Self,
{
    assoc Output: Type;
    fn bit_or(self, rhs: Rhs) -> Output;
}

pub trait BitXor
where
    .Rhs: Type = Self,
{
    assoc Output: Type;
    fn bit_xor(self, rhs: Rhs) -> Output;
}

pub trait Shl
where
    .Rhs: Type = Self,
{
    assoc Output: Type;
    fn shl(self, rhs: Rhs) -> Output;
}

pub trait Shr
where
    .Rhs: Type = Self,
{
    assoc Output: Type;
    fn shr(self, rhs: Rhs) -> Output;
}
```

**规范冲突点**：
- [docs/grammar/index.md](../../docs/grammar/index.md) 明确规定：**Flurry 语言有意不提供传统的 C/Rust 位运算符**（`|` 保留为扩展集合/模式分隔，`~` 为 splice）；`and`/`or`/`not` 既用于 `bool` 逻辑，也用于 **`BitVec<N>` 的逐位布尔运算（且保证两侧均求值）**；标量机器整数的位运算由 `builtin.math.bit_and`/`bit_or`/`bit_xor`/`rotate_*`/`funnel_shift_*` 与原语固有方法提供。
- 此外，上述五个 trait 缺少 `BitNot`，且 `Shr` 无法区分算术右移（`ashr`）与逻辑右移（`lshr`），又被放进 `protocols/ops.fl`（给开发者造成“存在 `&`/`|`/`<<`/`>>` 运算符重载”的错觉）。

#### 3.2.2 调和目标设计

在获得 §5 正式裁决前，确立以下契约边界：
1. **绝不新增 `&` / `|` / `^` / `<<` / `>>` 表面运算符**（完全遵守 [docs/grammar/index.md](../../docs/grammar/index.md)）。
2. **`BitVec<N>` 位运算不走 `BitAnd`/`BitOr` trait**：`a and b`、`a or b`、`not a` 在 `BitVec<N>` 上由编译器按 `BitVec` 代数规则直接处理并保证双边急切求值；任意用户 struct 仍不得通过 trait 重载中缀 `and`/`or`。
3. **推荐清理（方案 A）**：将 `BitAnd`/`BitOr`/`BitXor`/`Shl`/`Shr` 从语法协议核心文件 `builtin/protocols/ops.fl` 移除（避免与表面语法失配的僵尸 trait），通用位操作统一由 `BitVec<N>` + `builtin.math.bit_*` + 固定整数固有方法（`bit_and`、`bit_or`、`bit_xor`、`shl`、`wrapping_shl`、`logical_shr`、`arithmetic_shr`）提供。若作者需保留纯泛型位代数约束（方案 B），则应迁入 `builtin.numeric.bits` 并补齐 `BitNot`/`ArithmeticShr`/`LogicalShr` 与 `*Assign` 变体，明确与 `language-items.conf` 脱钩。

---

### 3.3 拼接协议 `Concat` 与 `++` 运算符

#### 3.3.1 完整协议签名（Runtime & Comptime）

当前编译器仅在 `compiler/elaboration/src/program/operators.rs:551-581` 硬编码 `BitVec<M> ++ BitVec<N>`。目标设计将其推广为通用语法协议：

```flurry
-- Runtime 协议：library/builtin/protocols/ops.fl（builtin.ops）
pub trait Concat
where
    .Rhs: Type = Self,
{
    assoc Output: Type;
    fn concat(self, rhs: Rhs) -> Output;
}

-- Comptime 对偶：library/meta/ops.fl（meta.ops）
pub pure comptime trait ComptimeConcat
where
    .Rhs: Type = Self,
{
    assoc Output: Type;
    pure comptime fn concat(self, rhs: Rhs) -> Output;
}
```

#### 3.3.2 标准实现表、Allocator 约束与两相边界

| 主体 `Self` | `.Rhs` | `Output` | 阶段 | 核心契约与说明 |
|---|---|---|---|---|
| `BitVec<M>` | `BitVec<N>` | `BitVec<M + N>` | Runtime & Comptime | 保留编译器内建位连接降码与 CEGIS/SAT 位宽证明（`M` 在高位，`N` 在低位）；`meta` 侧可由 `ComptimeConcat` 同义支撑 |
| `Array<T, M>` | `Array<T, N>` | `Array<T, M + N>` | Runtime & Comptime | 逐元素移动（仅需 `T :- Once` 即可，不强求 `Copy`），保持源顺序；编译时同样适用 |
| `str` | `str` | `str` | **仅 Comptime**（`ComptimeConcat`） | **重要架构约束**：运行期两个未知 `str` 切片拼接必须分配堆内存，而 `builtin` 严禁暗中依赖全局分配器或 `std.string.String`；故 `str ++ str` **仅在 `comptime` 实现 `ComptimeConcat` 返回 `str`**，运行期文本拼接显式使用 `String` 与传入的 `.allocator` |
| `meta.List<T>` | `meta.List<T>` | `meta.List<T>` | **仅 Comptime**（`ComptimeConcat`） | 不可变持久化或值引用追加，代替目前仅有的 `.concat()` 方法 |
| `Tuple<...As>` | `Tuple<...Bs>` | `Tuple<...As, ...Bs>` | Runtime & Comptime | 通过通用可变参数协议实现（不硬编码特判 `Tuple` 名称），`As`/`Bs` 中非 `Copy` 成员按值移动 |
| `std.string.String<.A = A, .allocator = a>` | `str` 或 `String<...>` | `String<.A = A, .allocator = a>` | Runtime（`std.string`） | 消费 `self`（复用既有堆缓冲区追加并在容量不足时以持有的 `allocator` 扩容），传播 `!AllocErr` 吗？**注意**：若会分配失败，签名为何？详见下方契约分析！ |
| `std.collections.Vec<T, .A = A, .allocator = a>` | `Vec<T, .A = A, .allocator = a>` | `Vec<T, .A = A, .allocator = a>` | Runtime（`std.collections`） | 消费 `self` 与 `rhs` 转移元素（见下方失败传播边界） |

**资源、错误与纯度关键边界**：
1. **`Concat.concat` 消费 `self` 与失败表达**：`Concat` 自身签名返回 `Output`（非固定 `!E Output`），允许类型按需将 `Output` 具体化为 `!AllocErr String<...>`（若存在 OOM），或者在 `std.string` 定义中明确 `String ++ str -> !AllocErr String<...>` 与无分配版本（如 `BitVec<M + N>`、`Array<T, M + N>` 返回纯值 `Output`）。对于不支持隐式 OOM 的纯计算表达式 `a ++ b ++ c`，`Output = !AllocErr String` 会在 `++` 链式拼接时类型不匹配（迫使用户写成 `(a ++ b)! ++ c` 或 `builder`），因此 `++` 默认更适合无失败值拼接（`BitVec`、`Array`、`Tuple`、comptime `str`/`meta.List`）或明确携带 `!AllocErr` 输出的类型。
2. **解析优先级注记**：`compiler/fmt/src/exprs.rs:2616` 当前将 `AddAdd` 置于优先级 `70`（与 `*`/`/`/`%` 同级，高于 `+`/`-` 的 `60`）。应在 §5 确认是否调整到 `55`（低于 `+`/`-`，避免 `vec ++ [a + b]` 之外如 `[1] ++ [2]` 等无需括号但 `1 ++ 2 + 3` 反直觉）或保持现有 `70`。
3. **与 `ConcatPatternView` 交互**：模式中的 `prefix ++ rest` 路由到 `ConcatPatternView` / `ComptimeConcatPatternView`（见 `02-pattern-matching-and-views.md`），不调用构造用 `Concat.concat`。

#### 3.3.3 正反用例

```flurry
-- 合法：comptime 下 str ++ str 返回新 str；运行期 BitVec 位拼接保留宽度类型
comptime {
    let tag: str = "flurry_" ++ "v1";
}

fn pack_header(ver: BitVec<4>, flags: BitVec<4>) -> BitVec<8> {
    ver ++ flags
}
```

**反例（必须拒绝）**：
- `fn join_rt(a: str, b: str) -> str { a ++ b }` -> 拒绝：运行期动态 `str` 不实现 `builtin.ops.Concat`（无隐式堆分配器），应改为使用 `std.string.String` 与显式 `.allocator` 或迁移至 `comptime`。
- `let bad: BitVec<10> = (0b1111'bitvec) ++ (0b0000'bitvec);` -> 拒绝：PTC 证明 `4 + 4 != 10`。

---

### 3.4 全系列 Range 类型与 `RangeBounds`

当前 `docs/grammar/syntax` 已定义全部 6 种范围表达式语法（并降低为 `H.Range { start, end, inclusive }`），但标准库完全未定义任何 Range 类型与接口，导致独立 `0..10` 表达式在 `elaboration` 阶段报错。

#### 3.4.1 六种完整 Range 类型与 `Bound<T>`（`builtin.range`）

```flurry
-- 归属：library/builtin/range.fl（builtin.range，由 builtin.prelude 导出）

pub enum Bound
where
    T: Type,
{
    included(T),
    excluded(T),
    unbounded,
}

-- 1. `..`
pub struct RangeFull {}

-- 2. `start..`
pub struct RangeFrom
where
    Idx: Type,
{
    pub start: Idx,
}

-- 3. `..end`
pub struct RangeTo
where
    Idx: Type,
{
    pub end: Idx,
}

-- 4. `..=end`
pub struct RangeToInclusive
where
    Idx: Type,
{
    pub end: Idx,
}

-- 5. `start..end`（半开区间 [start, end)）
pub struct Range
where
    Idx: Type,
{
    pub start: Idx,
    pub end: Idx,
}

-- 6. `start..=end`（闭区间 [start, end]，内部维护 exhausted 以安全处理 0u8..=255u8 边界）
pub struct RangeInclusive
where
    Idx: Type,
{
    start: Idx,
    end: Idx,
    exhausted: bool,
}
```

`RangeInclusive` 配套固有构造与访问方法（`impl` 于 `RangeInclusive<Idx>`）：

```flurry
impl RangeInclusive<Idx>
where
    Idx: Type,
{
    pub fn new(start: Idx, end: Idx) -> RangeInclusive<Idx> {
        RangeInclusive { start: start, end: end, exhausted: false }
    }

    pub fn start(*self) -> *Idx {
        self.start
    }

    pub fn end(*self) -> *Idx {
        self.end
    }

    pub fn into_inner(self) -> Tuple<Idx, Idx> {
        (self.start, self.end)
    }

    pub fn is_empty(*self) -> bool
    where
        requires Idx :- Ord,
    {
        self.exhausted.* or Idx.as(Ord).gt(self.start, self.end)
    }

    pub fn contains(*self, item: *Idx) -> bool
    where
        requires Idx :- Ord,
    {
        if self.exhausted.* {
            return false;
        }
        Idx.as(Ord).le(self.start, item) and Idx.as(Ord).le(item, self.end)
    }
}
```

#### 3.4.2 语法映射与求值顺序

| 表面表达式 | AST / HIR 表示 | 目标类型 | 求值顺序 | 特殊上下文快速路径说明 |
|---|---|---|---|---|
| `..` | `H.Range(null, null, false)` | `RangeFull` | ZST 常量构造成本为零 | `xs[..]`（`Slice`/`Vec` 全切片）；`SpaceApply` 中作全轴占位符 |
| `start..` | `H.Range(start, null, false)` | `RangeFrom<T>` | 先求值 `start` | `for x in 0..` 需 `start` 实现步进/迭代 |
| `..end` | `H.Range(null, end, false)` | `RangeTo<T>` | 先求值 `end` | 切片语法 `xs[..n]` |
| `..=end` | `H.Range(null, end, true)` | `RangeToInclusive<T>` | 先求值 `end` | 切片语法 `xs[..=n]` |
| `start..end` | `H.Range(start, end, false)` | `Range<T>` | 从左到右先 `start` 后 `end` | `BitVec[lo..hi]` 与 `inline for i in 0..N` 保留静态定宽/展开特判；其余统一构造 `Range<T>` |
| `start..=end` | `H.Range(start, end, true)` | `RangeInclusive<T>` | 从左到右先 `start` 后 `end` | 同样保留 `inline for i in 0..=N`，运行期构造 `RangeInclusive.new(start, end)` |

**关键对齐**：`start` 与 `end` 必须统一为相同类型 `Idx`（若为未后缀整数/浮点字面量则按双向推断或默认类型 `i32`/`f64` 统一，如 `0..10` 默认可受目标上下文 `Range<usize>` 约束）；允许非 `Copy` 端点类型（例如 `String` 范围 `"a".to_string().."z".to_string()`）构造成 `Range<String>`，移动端点所有权而不强求 `Idx :- Copy`。

#### 3.4.3 `RangeBounds<T>` 与 `ComptimeRangeBounds<T>`

```flurry
-- Runtime 协议：builtin.range
pub trait RangeBounds
where
    T: Type,
{
    fn start_bound(*self) -> Bound<*T>;
    fn end_bound(*self) -> Bound<*T>;

    fn contains(*self, item: *T) -> bool
    where
        requires T :- Ord,
    {
        let start_ok = self.start_bound() match {
            .included(s) => T.as(Ord).le(s, item),
            .excluded(s) => T.as(Ord).lt(s, item),
            .unbounded => true,
        }
        if not start_ok {
            return false;
        }
        self.end_bound() match {
            .included(e) => T.as(Ord).le(item, e),
            .excluded(e) => T.as(Ord).lt(item, e),
            .unbounded => true,
        }
    }
}

-- Comptime 对偶：meta.range（遵守 comptime 无指针规则，Bound 直接承载值而非 *T）
pub pure comptime trait ComptimeRangeBounds
where
    T: Type,
{
    pure comptime fn start_bound(self) -> Bound<T>;
    pure comptime fn end_bound(self) -> Bound<T>;

    pure comptime fn contains(self, item: T) -> bool
    where
        requires T :- ComptimeOrd,
    {
        let start_ok = self.start_bound() match {
            .included(s) => T.as(ComptimeOrd).le(s, item),
            .excluded(s) => T.as(ComptimeOrd).lt(s, item),
            .unbounded => true,
        }
        if not start_ok {
            return false;
        }
        self.end_bound() match {
            .included(e) => T.as(ComptimeOrd).le(item, e),
            .excluded(e) => T.as(ComptimeOrd).lt(item, e),
            .unbounded => true,
        }
    }
}
```

*关键设计解释*：在 Runtime `contains` 中显式使用 `T.as(Ord).le(s, item)` 而非 `s <= item`，因为 `s` 与 `item` 的类型均为 `*T`；直接写 `s <= item` 会比较指针地址或造成指针与值类型歧义！而在 Comptime `ComptimeRangeBounds` 中，`Bound<T>` 直接携带值且方法接收 `self`（值引用），零 `*T` 指针出现，且方法名 `start_bound`、`end_bound`、`contains` 与 Runtime 版完全一致。

完整实现 `RangeBounds<T>` 对六种类型的全部映射：

- `RangeFull`：`start_bound` -> `.unbounded`，`end_bound` -> `.unbounded`
- `RangeFrom<T>`：`start_bound` -> `.included(self.start)`，`end_bound` -> `.unbounded`
- `RangeTo<T>`：`start_bound` -> `.unbounded`，`end_bound` -> `.excluded(self.end)`
- `RangeToInclusive<T>`：`start_bound` -> `.unbounded`，`end_bound` -> `.included(self.end)`
- `Range<T>`：`start_bound` -> `.included(self.start)`，`end_bound` -> `.excluded(self.end)`
- `RangeInclusive<T>`：若 `self.exhausted.*` 为真则返回已空闭区间语义（`start_bound` -> `.excluded(self.end)` 或专有判空、`end_bound` -> `.excluded(self.end)`，配合 `.contains()` 重写确保永返回 `false`）；未耗尽时 `start_bound` -> `.included(self.start)`，`end_bound` -> `.included(self.end)`。

#### 3.4.4 整数 Range 迭代接入（`Iterator` / `PureComptimeIterator`）

为 `Range<I>`、`RangeInclusive<I>`、`RangeFrom<I>` 对全部定宽整型（`u8..u128`、`usize`、`i8..i128`、`isize`）实现 `builtin.iterator.Iterator`（推式 `inline fn iterate` + `emit_item`，见 `04-builtin-types-math-iterators-fmt.md`），使得 `for i in 0..10 { ... }` 与 `(0..=255u8).iter()` 无需特殊语法循环特判即可零开销展开：

- `Range<I>`：当 `start < end` 时循环产出 `cur` 并置 `cur = cur + 1`（半开区间末次不发生溢出）；
- `RangeInclusive<I>`：若 `exhausted` 为真则 0 次；否则产出 `cur`，**先判断是否 `cur == end`**（若相等则标记 `exhausted = true` 后退出，**绝不计算 `end + 1`**，彻底消除 `0u8..=255u8` 上界溢出！）；
- `RangeFrom<I>`：无限递增序列，在触发上界溢出策略时按 Flurry 整数溢出契约处理（debug 检查 / panic / abort）；
- **反例（必须拒绝）**：
  - `for x in ..10 { ... }` -> 拒绝：`RangeTo` 没有起始点，不实现 `Iterator` / `IntoIterator`；
  - `for x in 0.0..1.0 { ... }` -> 拒绝：浮点步长存在精度与 NaN 问题，不默认实现 `Iterator`；
  - `let r: Range<u32> = 0u32..10i32;` -> 拒绝：`Range<Idx>` 两端点类型不统一。

---

### 3.5 比较与排序协议（`Eq`, `Ord`, `Order`, `TotalOrd`）

#### 3.5.1 现存实现缺陷审计

`library/builtin/numeric/comparison.fl`（及 `library/meta/numeric/{equality,ordering}.fl`）存在三项严重设计与可用性缺陷：

1. **按值消费导致所有权破坏**：`Eq.eq(self, other: Self) -> bool` 与 `Ord.cmp(self, other: Self) -> Order` 按值传参。实测证据：`library/std/string_impl/string.fl:834-858` 中 `String.eq` 与 `String.cmp` 被迫接管 `self`/`other` 并在比较结束时 Drop 两个字符串；`library/std/collections_impl/hash_map.fl:653-654` 被迫额外克隆待比较的键后再调用 `existing_copy.as(Eq).eq(probe_copy)`！
2. **默认 `==`/`<` 不支持泛型借用比较**：若 `v1`、`v2` 为非 `Copy` 的 `Vec<i32>`，直接写 `if v1 == v2 { ... }` 会把 `v1` 和 `v2` 全部移动消费掉，分支之后无法继续使用 `v1`！
3. **浮点完全游离于 trait 之外**：`f16`/`bf16`/`f32`/`f64` 均未实现 `Eq`/`Ord`/`ComptimeEq`/`ComptimeOrd`，导致任何 `where requires T :- Eq` 或 `requires T :- Ord` 的泛型算法（如 `Slice.contains`、`Slice.sort`、`std.cmp.min`/`max`）都无法用于浮点数。

#### 3.5.2 目标规范（指针借用接收者 + `TotalOrd`）

```flurry
-- 归属：library/builtin/numeric/comparison.fl（builtin.cmp，由 builtin.prelude 导出）

pub enum Order {
    less,
    equal,
    greater,
}

pub trait Eq
where
    .Rhs: Type = Self,
{
    fn eq(*self, other: *Rhs) -> bool;

    fn ne(*self, other: *Rhs) -> bool {
        not self.eq(other)
    }
}

pub trait Ord :- Eq {
    fn cmp(*self, other: *Self) -> Order;

    fn lt(*self, other: *Self) -> bool {
        self.cmp(other) match {
            .less => true,
            _ => false,
        }
    }

    fn le(*self, other: *Self) -> bool {
        self.cmp(other) match {
            .greater => false,
            _ => true,
        }
    }

    fn gt(*self, other: *Self) -> bool {
        self.cmp(other) match {
            .greater => true,
            _ => false,
        }
    }

    fn ge(*self, other: *Self) -> bool {
        self.cmp(other) match {
            .less => false,
            _ => true,
        }
    }
}

-- IEEE 754 totalOrder 等全序协议（专为可包含 NaN / -0.0 与 +0.0 的浮点与排序场景提供）
pub trait TotalOrd {
    fn total_cmp(*self, other: *Self) -> Order;
}
```

**Comptime 对偶族（`meta.cmp`）**：采用无指针值引用语义、统一方法名，并全部标记为 `pure comptime fn` 以便 `ComptimeRangeBounds.contains` 等纯编译时函数合法调用：

```flurry
-- 归属：library/meta/numeric/equality.fl & ordering.fl（meta.cmp，由 meta.prelude 导出）

pub pure comptime trait ComptimeEq
where
    .Rhs: Type = Self,
{
    pure comptime fn eq(self, other: Rhs) -> bool;

    pure comptime fn ne(self, other: Rhs) -> bool {
        not self.eq(other)
    }
}

pub pure comptime trait ComptimeOrd :- ComptimeEq {
    pure comptime fn cmp(self, other: Self) -> Order;

    pure comptime fn lt(self, other: Self) -> bool {
        self.cmp(other) match {
            .less => true,
            _ => false,
        }
    }

    pure comptime fn le(self, other: Self) -> bool {
        self.cmp(other) match {
            .greater => false,
            _ => true,
        }
    }

    pure comptime fn gt(self, other: Self) -> bool {
        self.cmp(other) match {
            .greater => true,
            _ => false,
        }
    }

    pure comptime fn ge(self, other: Self) -> bool {
        self.cmp(other) match {
            .less => false,
            _ => true,
        }
    }
}

pub pure comptime trait ComptimeTotalOrd {
    pure comptime fn total_cmp(self, other: Self) -> Order;
}
```

#### 3.5.3 `==` / `<` 运算符降码、等价关系与 IEEE 浮点取舍

1. **运算符求值不消耗 owner**：对于 `lhs == rhs`（及 `!=`、`<`、`<=`、`>`、`>=`），若未命中编译器原生标量快速路径：
   - 按源序先计算 `lhs` 定位（若为右值临时值则物化于匿名局部栈槽），再计算 `rhs`（临时值物化于匿名局部栈槽）；
   - 取两者地址调用 `Eq.eq(lhs_slot.ref, rhs_slot.ref)` 或 `Ord.lt(...)`；
   - 比较完成后仅清理本次创建的右值临时值，**绝不移动或销毁调用方的 `let`/`var` 变量**。从而彻底修复 `String == String`、`Vec == Vec` 静默消费对象的问题！
2. **`PartialEq` / `PartialOrd` vs `Eq` / `Ord` + `TotalOrd` 分析**：
   - Flurry 当前显式取消了 Rust 的 `PartialEq`/`PartialOrd` 双层拆分（`Eq` 直接代表等价关系，`Ord` 代表全序关系），在语义上最契合 `BTreeMap`/`sort`/`HashMap` 等容器契约，且大幅减少库作者重复实现两套 trait 的样板。
   - **IEEE 754 浮点边界**：原生浮点运算 `a == b`、`a < b`（对 `NaN` 恒为 `false`、`NaN != NaN` 为 `true`）必须保持编译器原生标量语义以符合 IEEE 754；正因 `NaN != NaN` 违反 `Eq` 自反律，**不应将 IEEE 语义直接硬塞进 `Eq`/`Ord` 伪称全序**。
   - 推荐方案（需 §5 确认）：保持 `Eq`/`Ord` 为纯全序/等价协议，为 `f16`/`bf16`/`f32`/`f64` 提供固有方法 `.total_cmp` 与 `TotalOrd` 实现（遵照 IEEE 754 `totalOrder`：`-NaN < -Inf < ... < -0.0 < +0.0 < ... < +Inf < +NaN`），并提供显式包装新类型（如 `TotalFloat<F>` 或 `NonNanFloat<F>`，其中 `new` 校验非 NaN 后允许实现 `Eq + Ord + Hash`）供排序与哈希表键使用；同时保留是否新增 `PartialEq`/`PartialOrd` 作备选供作者裁决。

**反例（必须拒绝）**：
- `var xs = Vec<f64>.new(); xs.push(1.0); xs.sort();`（假设 `sort` 要求 `requires T :- Ord`）-> 拒绝：`f64` 因 NaN 偏序不实现 `Ord`，应调用 `xs.sort_by(...)` 或使用 `TotalFloat<f64>`。

---

### 3.6 哈希协议族（`Hash`, `Hasher`, `BuildHasher`, `DefaultHasher`）

#### 3.6.1 现存实现缺陷

`library/builtin/protocols/traits.fl:23-25` 当前仅有无状态签名的 `pub trait Hash { fn hash(self) -> u64; }`，并消费 `self`；`library/meta/data/hashing.fl` 为 `pub pure comptime trait ComptimeHash { comptime fn hash(self) -> u64; }`。该设计无法切换哈希算法（如 SipHash-1-3 防 HashDoS、FxHash 极速小整数哈希）、无法避免元组/字符串前缀碰撞（`("ab", "c")` vs `("a", "bc")`），也迫使非 `Copy` 键克隆或消耗。

#### 3.6.2 Runtime 流式哈希族（`builtin.hash` 与 `std.hash`）

```flurry
-- 归属：library/builtin/hash.fl（builtin.hash，由 builtin.prelude 导出）

pub trait Hasher {
    fn finish(*self) -> u64;
    fn write(*self, bytes: Slice<u8>) -> void;

    fn write_u8(*self, value: u8) -> void {
        let buf: Array<u8, 1> = [value];
        self.write(buf.as_slice());
    }

    fn write_u16(*self, value: u16) -> void {
        self.write(value.to_ne_bytes().as_slice());
    }

    fn write_u32(*self, value: u32) -> void {
        self.write(value.to_ne_bytes().as_slice());
    }

    fn write_u64(*self, value: u64) -> void {
        self.write(value.to_ne_bytes().as_slice());
    }

    fn write_u128(*self, value: u128) -> void {
        self.write(value.to_ne_bytes().as_slice());
    }

    fn write_usize(*self, value: usize) -> void {
        self.write(value.to_ne_bytes().as_slice());
    }

    fn write_i8(*self, value: i8) -> void {
        self.write_u8(value.as(u8));
    }

    fn write_i16(*self, value: i16) -> void {
        self.write_u16(value.as(u16));
    }

    fn write_i32(*self, value: i32) -> void {
        self.write_u32(value.as(u32));
    }

    fn write_i64(*self, value: i64) -> void {
        self.write_u64(value.as(u64));
    }

    fn write_i128(*self, value: i128) -> void {
        self.write_u128(value.as(u128));
    }

    fn write_isize(*self, value: isize) -> void {
        self.write_usize(value.as(usize));
    }

    fn write_length_prefix(*self, len: usize) -> void {
        self.write_usize(len);
    }

    fn write_str(*self, text: str) -> void {
        let n = text.byte_length();
        self.write_length_prefix(n.as(usize));
        -- 注：待 04-builtin-types-math-iterators-fmt.md 落地 str.as_bytes() -> Slice<u8> 后可直接调用 self.write(text.as_bytes()) 批量写入
        var i: u64 = 0;
        while i < n {
            let b = text.byte(i) match {
                v? => v,
                null => break,
            }
            self.write_u8(b);
            i = i + 1;
        }
    }
}

pub trait Hash {
    fn hash(*self, state: *H) -> void
    where
        H: Type,
        requires H :- Hasher;

    fn hash_slice(data: Slice<Self>, state: *H) -> void
    where
        H: Type,
        requires H :- Hasher,
    {
        state.write_length_prefix(data.len());
        var i: usize = 0;
        let n = data.len();
        while i < n {
            data[i].hash(state);
            i = i + 1;
        }
    }
}

pub trait BuildHasher {
    assoc HasherTy: Type
    where
        requires HasherTy :- Hasher;

    fn build_hasher(*self) -> HasherTy;

    fn hash_one(*self, value: *T) -> u64
    where
        T: Type,
        requires T :- Hash,
    {
        var hasher = self.build_hasher();
        value.hash(hasher.ref);
        hasher.finish()
    }
}
```

并在 `std.hash` 中提供防 HashDoS 的默认 `SipHash-1-3` 实现（`DefaultHasher` 仅作标准散列表基线实现，不越权进入通用加密算法如 SHA-256/BLAKE3，后者归属 Tier 2 官方协议包 `crypto`/`digest`）：

```flurry
-- 归属：library/std/hash_impl/sip.fl（std.hash，按 std 规则显式导入或通过 std.prelude 导出）

pub struct DefaultHasher {
    k0: u64,
    k1: u64,
    v0: u64,
    v1: u64,
    v2: u64,
    v3: u64,
    length: usize,
    tail: u64,
    ntail: usize,
}

impl DefaultHasher {
    pub fn new() -> DefaultHasher {
        DefaultHasher.new_with_keys(0, 0)
    }

    pub fn new_with_keys(key0: u64, key1: u64) -> DefaultHasher {
        DefaultHasher {
            k0: key0,
            k1: key1,
            v0: builtin.math.bit_xor(key0, 0x736f6d6570736575u64),
            v1: builtin.math.bit_xor(key1, 0x646f72616e646f6du64),
            v2: builtin.math.bit_xor(key0, 0x6c7967656e657261u64),
            v3: builtin.math.bit_xor(key1, 0x7465646279746573u64),
            length: 0,
            tail: 0,
            ntail: 0,
        }
    }
}

impl Default for DefaultHasher {
    fn default_value() -> DefaultHasher {
        DefaultHasher.new()
    }
}

impl Hasher for DefaultHasher {
    fn write(*self, bytes: Slice<u8>) -> void {
        self.length.* = self.length.* + bytes.len();
        -- 按 8 字节分块执行 SipHash-1-3 压缩（内部位混合通过 builtin.math.bit_* 与 rotate_left 实现）
    }

    fn finish(*self) -> u64 {
        -- 写入末尾长度字节并执行 3 轮 SipRound 终结混合后输出 64 位摘要
        builtin.math.bit_xor(builtin.math.bit_xor(self.v0.*, self.v1.*), builtin.math.bit_xor(self.v2.*, self.v3.*))
    }
}

pub struct RandomState {
    k0: u64,
    k1: u64,
}

impl RandomState {
    pub fn from_keys(key0: u64, key1: u64) -> RandomState {
        RandomState { k0: key0, k1: key1 }
    }

    pub fn new() -> RandomState {
        -- 实际运行期种子由 07-std-mem-collections-io-os.md 定义的 OS 随机熵源填充；无 OS 宿主时采用确定性种子
        RandomState.from_keys(0x0706050403020100u64, 0x0f0e0d0c0b0a0908u64)
    }
}

impl Default for RandomState {
    fn default_value() -> RandomState {
        RandomState.new()
    }
}

impl BuildHasher for RandomState {
    assoc HasherTy: Type = DefaultHasher;

    fn build_hasher(*self) -> DefaultHasher {
        DefaultHasher.new_with_keys(self.k0.*, self.k1.*)
    }
}
```

#### 3.6.3 Comptime 哈希对偶（`meta.hash`）——无指针制约下的完整分析

因为 comptime **没有指针**（无法传 `state: *H` 给 `fn hash(self, state: *H)`），如果 `ComptimeHash.hash` 接收值参数 `state: H`，在函数体内修改 `state` 只会修改局部变量副本，无法更新调用者的 `hasher`！因此 comptime 存在两种合法建模：

- **方案 A（推荐：值穿线 `ComptimeHasher` + 保留同方法名 `write`/`hash`/`hash_slice`）**：利用 `self` 值引用驱动 `ComptimeHasher.write(self, bytes: meta.List<u8>)` 原地累计状态（与 Runtime `Hasher` 保持完全一致的方法集与同名规则：`finish`、`write`、`write_u8..write_isize`、`write_length_prefix`、`write_str`，不增设 `Hasher` 中不存在的额外方法名），同时让 `ComptimeHash.hash(self, state: H) -> H` 与 `hash_slice(data: meta.List<Self>, state: H) -> H` 返回更新后的 `H`（函数式状态传递）：

```flurry
-- 归属：library/meta/data/hashing.fl（meta.hash，由 meta.prelude 导出）

pub pure comptime trait ComptimeHasher {
    pure comptime fn finish(self) -> u64;
    comptime fn write(self, bytes: meta.List<u8>) -> void;

    comptime fn write_u8(self, value: u8) -> void {
        self.write([value]);
    }

    comptime fn write_u16(self, value: u16) -> void {
        var i: usize = 0;
        var rem = value;
        while i < 2 {
            self.write_u8((rem % 256).as(u8));
            rem = rem / 256;
            i = i + 1;
        }
    }

    comptime fn write_u32(self, value: u32) -> void {
        var i: usize = 0;
        var rem = value;
        while i < 4 {
            self.write_u8((rem % 256).as(u8));
            rem = rem / 256;
            i = i + 1;
        }
    }

    comptime fn write_u64(self, value: u64) -> void {
        var i: usize = 0;
        var rem = value;
        while i < 8 {
            self.write_u8((rem % 256).as(u8));
            rem = rem / 256;
            i = i + 1;
        }
    }

    comptime fn write_u128(self, value: u128) -> void {
        var i: usize = 0;
        var rem = value;
        while i < 16 {
            self.write_u8((rem % 256).as(u8));
            rem = rem / 256;
            i = i + 1;
        }
    }

    comptime fn write_usize(self, value: usize) -> void {
        self.write_u64(value.as(u64));
    }

    comptime fn write_i8(self, value: i8) -> void {
        self.write_u8(value.as(u8));
    }

    comptime fn write_i16(self, value: i16) -> void {
        self.write_u16(value.as(u16));
    }

    comptime fn write_i32(self, value: i32) -> void {
        self.write_u32(value.as(u32));
    }

    comptime fn write_i64(self, value: i64) -> void {
        self.write_u64(value.as(u64));
    }

    comptime fn write_i128(self, value: i128) -> void {
        self.write_u128(value.as(u128));
    }

    comptime fn write_isize(self, value: isize) -> void {
        self.write_usize(value.as(usize));
    }

    comptime fn write_length_prefix(self, len: usize) -> void {
        self.write_usize(len);
    }

    comptime fn write_str(self, text: str) -> void {
        let n = text.byte_length();
        self.write_length_prefix(n.as(usize));
        var i: u64 = 0;
        while i < n {
            let b = text.byte(i) match {
                v? => v,
                null => break,
            }
            self.write_u8(b);
            i = i + 1;
        }
    }
}

pub pure comptime trait ComptimeHash {
    pure comptime fn hash(self, state: H) -> H
    where
        H: Type,
        requires H :- ComptimeHasher;

    pure comptime fn hash_slice(data: meta.List<Self>, state: H) -> H
    where
        H: Type,
        requires H :- ComptimeHasher,
    {
        var acc = state;
        let n = data.length();
        acc.write_length_prefix(meta.to_usize(n));
        var i: Integer = 0;
        while i < n {
            acc = data[i].hash(acc);
            i = i + 1;
        }
        acc
    }
}

pub pure comptime trait ComptimeBuildHasher {
    assoc HasherTy: Type
    where
        requires HasherTy :- ComptimeHasher;

    pure comptime fn build_hasher(self) -> HasherTy;

    pure comptime fn hash_one(self, value: T) -> u64
    where
        T: Type,
        requires T :- ComptimeHash,
    {
        let h0 = self.build_hasher();
        let h1 = value.hash(h0);
        h1.finish()
    }
}
```

- **方案 B（轻量级，兼容现有 `library/meta/data/hashing.fl`）**：由于 comptime 求值具备确定性要求且无需抵御外部运行时 HashDoS，保留 `ComptimeHash { pure comptime fn hash(self) -> u64; }`，或提供带 `ComptimeHasher` 的两层，在 `meta.Map` 中使用确定性 `DefaultComptimeHasher`（SipHash 固定常量种子或 FNV-1a/WyHash）。

**`Eq` / `Hash` 一致性契约（不可违反）**：对任意可同时参与比较与哈希的类型 `T`，若 `T.as(Eq).eq(a, b)`（或 `ComptimeEq.eq(a, b)`）为 `true`，则对任意相同初始状态的 `H`，`a` 与 `b` 馈入 `H` 的序列与终值 `finish()` 必须完全一致。

**反例（必须拒绝）**：
- `let bad = comptime { var h = DefaultComptimeHasher.new(); h.ref.write_u64(1); h.finish() }` -> 拒绝：comptime 禁止 `.ref`、`*H` 或 `.*`。
- 为同时满足 `Eq` 的结构体跳过关键判等字段（或自定义 `Eq` 忽略大小写但默认 `Hash` 区分大小写）-> 逻辑契约破坏（应在文档及验证测试用例中明确禁止）。

---

### 3.7 索引（`Index`, `IndexMut`）、`SpaceApply` 与可调用边界

#### 3.7.1 `Index` 与 `IndexMut` 的生命周期悬垂修复及值视图分歧

**现存致命 bug**（`library/builtin/protocols/ops.fl:82-90`）：

```flurry
-- 现存 BUG 代码（严禁沿用）：按值获取 self 却返回内部元素指针 *Output！
pub trait Index where Idx: Type {
    assoc Output: Type;
    fn index(self, idx: Idx) -> *Output;
}
```

若 `v: Vec<String>` 按值传入 `index(self, 0)`，`Vec` 在 `index` 函数体退出时被 Drop，返回的 `*String` 立即悬垂（use-after-free）！

**目标设计（修正 `*self` 借用，并调和“位置指针下标” vs “计算视图下标”）**：

在 Flurry 中存在两类下标场景：
1. **容器存储槽位下标**（`Vec<T>[i]`、`Array<T, N>[i]`、`Slice<T>[i]`、`HashMap<K, V>[k]`）：返回指针 `*Output` 供字段投影（`vec[i].field`）、`.*` 读取或通过 `IndexMut` 写入（`vec[i] = val`、`vec[i] += 1`）。
2. **掩码 / 切片 / 投影视图下标**（如 [proposal_trace/2026-9-7-tensor-algebra.fl](../../proposal_trace/2026-9-7-tensor-algebra.fl#L31-L39) 中的 `xs[mask]` 返回新 `Tensor<T>` 值，或 `xs[lo..hi]` 返回值语义的 `Slice<T>` 视图，而非指向既有内存的 `*Slice<T>`）：返回**按值构造的视图 `Output`**（`Slice<T>` 或 `Tensor<T>`），若要写回整片视图不能靠 `*Slice<T>`。

针对这一结构选择，定义如下标准签名（`builtin.ops`）：

```flurry
pub trait Index
where
    Idx: Type,
{
    assoc Output: Type;
    fn index(*self, idx: Idx) -> *Output;
}

pub trait IndexMut :- Index<Idx>
where
    Idx: Type,
{
    fn index_mut(*self, idx: Idx) -> *Self.as(Index<Idx>).Output;
}
```

对照 `proposal_trace/2026-9-7-tensor-algebra.fl`：如果采纳 **规格 B**（`Index` 改为 `fn index(*self, idx: Idx) -> Output;`，`IndexMut` 额外给出 `assoc MutOutput: Type; fn index_mut(*self, idx: Idx) -> *MutOutput;` 或保持 `IndexMut` 关联类型可写入），则 `xs[0..2]` 返回 `Slice<T>`、`xs[mask]` 返回 `Tensor<T>`、`vec[i]` 若希望可直接读取需解决 `vec[i]` 结果类型是 `Output = T` 还是 `*T`（若 `Output = T` 则非 `Copy` 元素无法直接从 `*Vec<T>` 中移出！）。**因此，规格 A（`Index.index(*self, idx: Idx) -> *Output` 专司地址槽位访问，值视图切片与掩码走 `SpaceApply.space_apply(*self, ...) -> Target`）是唯一与 Flurry 非 `Copy` 仿射资源系统无矛盾的自洽模型！** 对必须直接 `slice_expr = arr[lo..hi]` 的容器（`Vec<T>`、`Array<T, N>`、`Slice<T>`），单参数 `Range` 类型未实现返回 `*Slice<T>` 的 `Index`（因为 `Slice<T>` 本身是个临时构造出的 `{ ptr, len }` 描述符，内存中不存在既有 `*Slice<T>` 地址！），正好自然落入紧随其后的 `SpaceApply`！

**Comptime 对偶（`meta.ops.ComptimeIndex`）**：comptime 无指针，因此 `ComptimeIndex` 按值/视图返回 `Output`（`T :- Copy` 或编译时值引用结构）：

```flurry
pub pure comptime trait ComptimeIndex
where
    Idx: Type,
{
    assoc Output: Type;
    comptime fn index(self, idx: Idx) -> Output;
}
```

（注意：comptime 中没有 `*Output` 与 `IndexMut`；若需修改 comptime 容器内部元素，使用 `meta.List<T>` 方法或在 §3.1.3 编译时复合赋值/元素赋值原语处由 VM 左值定位更新。）

#### 3.7.2 `SpaceApply` 与 `VariadicCollector`（多参数 `[...args]` 与范围切片）

依据 [docs/traits/protocols.md](../../docs/traits/protocols.md#L51) 与 `proposal_trace/2026-9-7-tensor-algebra.fl:43-82`：

```flurry
-- 归属：library/builtin/ops.fl（builtin.ops）

pub trait VariadicCollector {
    assoc Tmp: Type;
    fn variadic_collect(comptime S: Type, args: S) -> Tuple<Tmp, Self>;
}

pub trait SpaceApply {
    assoc CollectTo: Type
    where
        requires CollectTo :- VariadicCollector;

    assoc Target: Type;

    fn space_apply(*self, index: CollectTo) -> Target;
}
```

**核心求值与生命周期机制**：
1. **语法缺口**：当前 [docs/grammar/syntax](../../docs/grammar/syntax#L161) 为 `index_application -> expr [expr]`，且 `compiler/parse/src/expressions.rs:1497-1527` 只允许单个 `expr`。必须升级语法与 parser 支持方括号内多实参列表 `[argument*]`（逗号分隔，支持展开参数与范围 `xs[0..2, 0, ..]`）。
2. **解析分派顺序与“无静默回退”铁律**：
   - 当方括号内有**单个位置参数** `base[arg]`：先查询 `Index<arg'type> for Base`（按正常三层接收者适配规则）；
     - 若唯一匹配 `Index`（且上下文为写入目标时匹配 `IndexMut`），选择 `Index` / `IndexMut`；
     - 若高优先 `Index` 候选处于 `Unknown` 或发生候选内部歧义/trait 要求违背，**立即挂起或诊断失败，严禁静默回退到 `SpaceApply`**（[docs/traits/protocols.md](../../docs/traits/protocols.md)）；
     - **仅当明确证明 `Base` 无任何适用 `Index` 实现**（例如 `Vec<T>` 对 `Range<usize>` 无可指向的 `*Slice<T>` 槽位），才查询 `SpaceApply for Base`。
   - 当方括号内包含**多个实参或命名/展开实参** `base[a0, a1, ...]`：直接查询 `SpaceApply for Base`（构造实参元组 `args: Tuple<T0, T1, ...>`）。
3. **栈临时缓冲区保活**：`let collected = CollectTo.as(VariadicCollector).variadic_collect(S, args);`，元组 `Tuple<Tmp, CollectTo>` 在调用方栈帧保活底层缓冲 `Tmp`（例如 `SmallVec<RangeOrIndex, 5>`），将第二个成员 `CollectTo`（例如借用切片 `Slice<RangeOrIndex>`）传入 `base.space_apply(collected.1)`，在 `space_apply` 返回之后才析构 `Tmp`！

*完整示例*：

```flurry
-- 1. Tier 1 内置容器切片（Vec<T> 与 Range<usize>）：
--    单个索引 xs[i] 命中 Index<usize> 返回 *T；范围 xs[1..4] 无 Index<Range<usize>> 实现，按顺序转入 SpaceApply
impl SpaceApply for Vec<T, .A = A, .allocator = allocator>
where
    T: Type,
    A: Type,
    requires A :- std.mem.Allocator,
    quote allocator: lift *A,
{
    assoc CollectTo: Type = Slice<Range<usize>>;
    assoc Target: Type = Slice<T>;

    fn space_apply(*self, index: Slice<Range<usize>>) -> Slice<T> {
        let r = index[0];
        self.as_slice().subslice(r.start.*, r.end.*)
    }
}

-- 2. Tier 2/3 科学计算/ML 用户包中的多维 Tensor 切片示例（非六个内置包成员）：
enum RangeOrIndex {
    range(Range<usize>),
    full,
    index(usize),
}

struct AxisList {
    items: Array<RangeOrIndex, 4>,
    len: usize,
}

impl VariadicCollector for AxisList {
    assoc Tmp: Type = void;

    fn variadic_collect(comptime S: Type, args: S) -> Tuple<void, AxisList> {
        -- 利用 inline for 遍历 args'tuple_enumerate，将 Range / RangeFull / usize 依次写入 AxisList
        ((), AxisList { items: [RangeOrIndex.full, RangeOrIndex.full, RangeOrIndex.full, RangeOrIndex.full], len: 0 })
    }
}

-- 使用形态：xs[0..2, 0, ..] -> 收集为 AxisList 后调用 Tensor.space_apply
```

**反例（必须拒绝）**：
- `let x = xs[ambiguous_key];` 其中 `Index` 候选存在两条同精度冲突（或 `Index` 条件处于 `Unknown` 未决）但类型同时实现了 `SpaceApply` -> **必须拒绝或挂起，严禁静默回退到 `SpaceApply`**（[docs/traits/protocols.md](../../docs/traits/protocols.md)）。

#### 3.7.3 可调用边界（无 `Fn` / `FnOnce` / `FnMut`）

按 [docs/functions/higher-order-functions.md](../../docs/functions/higher-order-functions.md)、[docs/functions/closure-environments.md](../../docs/functions/closure-environments.md#L224) 与 `docs_chat_history/056-functions-and-closures-01.md:229, 245, 476` 权威裁定：

- **Flurry 明确不设立 `Fn`、`FnMut`、`FnOnce` 或 `Call` 运算符重载协议**（[docs/traits/protocols.md](../../docs/traits/protocols.md) 协议表故意留空 `()` 调用重载）。
- 高阶函数多态通过原生闭包限定函数类型 `%c fn(Arg0, Arg1, ...) -> R where c: Type.vars.Closure` 表达：
  - 空闭包环境自动消除为 `%void fn(...)`（零开销函数指针兼容）；
  - 单次可调用（Rust `FnOnce` 对应场景）、多次可调用（`Fn`/`FnMut`）不靠 3 套分裂 trait 编码，而是直接由捕获环境字段的 `Once`/`Copy`/指针可变性与函数主体资源契约决定。
- 函数值及闭包类型 `%c fn(...) -> R` **不能作为用户 `impl` 主体**（不可为其添加任何 trait impl）。

**反例（必须拒绝）**：
- `impl FnOnce for MyCallable { ... }` -> 拒绝：不存在也不引入 `FnOnce`/`FnMut`/`Fn` 语言协议。
- `impl Add for %c fn(i32) -> i32 where c: Type.vars.Closure { ... }` -> 拒绝：函数/闭包类型不得作为用户 `impl` 主体（[docs/functions/higher-order-functions.md](../../docs/functions/higher-order-functions.md)）。

---

### 3.8 `Deref`、`DerefMut` 与 `Reference`

#### 3.8.1 规范演进与权威来源对齐

说明规范历史以免混用：早期 `docs_chat_history/037-type-system-09.md:428` 曾提出仅显式 `.*`/`.ref`；但在 `docs_chat_history/047-trait-implementation-03.md:122-200` 与 `048-trait-implementation-04.md:29, 36` 中作者明确扩充接收者适配策略，该最终结论已写入现行正式正文 [docs/type/receiver-adaptation.md](../../docs/type/receiver-adaptation.md#L34-L36) 与 [docs/traits/protocols.md](../../docs/traits/protocols.md#L50)。然而当前 `library/` 与 `compiler/` 中这组协议尚完全空白。

#### 3.8.2 完整协议签名（Runtime 专属，`builtin.ops`）

```flurry
-- 归属：library/builtin/protocols/ops.fl（builtin.ops，由 builtin.prelude 导出）

pub trait Deref {
    assoc Target: Type;
    fn deref(*self) -> *Target;
}

pub trait DerefMut :- Deref {
    fn deref_mut(*self) -> *Self.as(Deref).Target;
}

pub trait Reference {
    assoc Target: Type;
    fn reference(*self) -> *Target;
}
```

#### 3.8.3 `.*`、`.ref` 与三层接收者适配规则

1. **显式运算符语义**：
   - `expr.*`：若 `expr` 为原生指针 `*T`，执行内建解引用得到可定位或读取的 `T`（`Copy` 引导读取或指针写回目标）；若 `expr` 为非原生指针但 `S :- Deref`，调用 `S.as(Deref).deref(expr.ref) -> *Target` 后执行内建指针解引用得到 `Target` 位置（赋值为写目标 `expr.* = v` 时改为调用 `DerefMut.deref_mut` 获取可写指针）。
   - `expr.ref`：若 `expr` 为可寻址原生位置且期望为原生 `*T`，直接取地址；若需满足特定目标指针或 `S :- Reference` 重载（例如显式句柄向内部指针转换），调用 `S.as(Reference).reference(expr.ref) -> *Target`。
2. **三层接收者适配顺序**（严格遵照 [docs/type/receiver-adaptation.md](../../docs/type/receiver-adaptation.md)）：
   - **第一层（视角层）**：显式 `.as(View)` 从后往前依次检索；
   - **第二层（解引用深度）**：对当前视角先原形态、再逐步执行内建指针 `*` 解引用及**唯一确定**的 `Deref` 路径（通过 `deref(*self) -> *Target` 直接拿到 `*Target` 供下一方法使用，**绝不复制整个 `Target` 对象后再取临时地址**）；
   - **第三层（实现精度与末端取址）**：同一形态内按 normalized IR 结构精度选择成员来源；所选方法要求 `*self` 时执行一次内建 `.ref` 或唯一 `Reference` 适配。
3. **关键安全边界**：
   - **零试探执行副作用**：检索路径时不得执行任何 `Deref`/`Reference` 体代码，仅最终唯一选定路径运行一次；
   - **循环与未决保护**：跟踪已访问的归一化类型头防止 `Deref` 自环死循环并设深度预算；若 `Deref.Target` 为 `Unknown`，挂起对应义务，绝不跳过并偷偷回退到外层候选；
   - **Comptime 禁用**：comptime 无指针，**不设 `ComptimeDeref`/`ComptimeReference`**；在 comptime 使用 `.*` 或 `.ref` 必须明确诊断拒绝。

**反例（必须拒绝）**：
- 下例构造了自身解引用回自身的 `Deref` 循环（在方法适配链展开时必须报错终止）：
```flurry
struct SelfRefPtr {
    tag: u32,
}

impl Deref for SelfRefPtr {
    assoc Target: Type = SelfRefPtr;

    fn deref(*self) -> *SelfRefPtr {
        self
    }
}
```
- 仅实现 `Deref`（未实现 `DerefMut`）的只读智能指针 `ro_ptr.* = 42` -> 拒绝：缺少 `DerefMut` 写入权限。
- 在 `comptime` 块内写 `x.ref` 或 `p.*` -> 拒绝：comptime 禁止指针与 `Deref`/`Reference` 协议。

---

### 3.9 字面量后缀协议（`LiteralInto<suffix, R>`）

#### 3.9.1 完整协议声明与缺失绑定

依据 [docs/traits/protocols.md](../../docs/traits/protocols.md#L33,L141) 与 [docs/basics/literals.md](../../docs/basics/literals.md#L9,L77-L80)：

```flurry
-- 归属：library/builtin/protocols/conversion.fl（builtin.convert，由 builtin.prelude 导出）

pub trait LiteralInto
where
    suffix: Symbol,
    R: Type,
{
    fn literal_into(comptime value: Self) -> R;
}
```

**现状对比**：编译器侧 `compiler/elaboration/src/program/literal_conversion.rs` 与 `SysrootBindings`（`compiler/luna/src/protocols/type_constructors.rs:571`）均已实现 `"literal-into-protocol"` 接线与 `--literal-into-protocol` CLI 覆写；但 **`library/builtin/` 尚未声明 `LiteralInto` 协议，且 `library/language-items.conf` 漏配 `literal-into-protocol = ...`！**

#### 3.9.2 四类主体、推断隔离、字符串插值与负字面量边界

1. **允许的四大字面量主体 `S`**：`Integer`、`Float`、`char`、`str`（对应 `compiler/parse/src/fragment.rs:365-370` `Int | Real | Str | StringConcat | Char`）。
   - 相邻静态字符串合并产物（`StringConcat`）可接入 `LiteralInto<suffix, R> for str`；
   - **含插值的字符串字面量**（如 `"id=${x}"` 或 `"id=$x"`，降码为 `std.fmt.comptime_format`）产生运行期依赖或 `FormatPlan`，**不能作为静态 `comptime value: str` 传给 `LiteralInto`**，必须语法/lowering 期拒绝后缀（注：Flurry 中仅 `$name` / `${expr}` 引入插值，单独的 `{x}` 是普通 `str` 字符）；
   - **禁止双后缀**：`12pxrem` 已由 `compiler/parse/src/expressions.rs:1967` 直接拒绝（`"A literal accepts only one suffix identifier"`）。
2. **结果类型 `R` 推断与隔离试探**：
   - 对表达式 `32px`，编译器构造查询 `exists ?R: Type. LiteralInto<.px, ?R> for Integer`；
   - 若上下文期望类型已知（如 `let x: Pixel = 32px;` 或函数形参约束），`?R` 先受上下文约束；否则保持自由推断元变量，**每个候选的试探替换必须在独立事务/沙盒中进行并按结构精度排序**，互不污染外部替换表；若多候选同精度（例如同一类型后缀同时支持 `.m` 转 `Meters` 与 `Minutes` 且无上下文类型指引）则报歧义；若无适用候选（如范围溢出、无效语法后缀）在 `comptime` 检查期通过 `meta.compilation.err` 报错。
3. **两相与 `const` 求值注意**：`LiteralInto.literal_into` 是带有 `comptime` 形参的普通 runtime phase 函数（[docs/comptime/the-comptime-phases-of-traits.md](../../docs/comptime/the-comptime-phases-of-traits.md#L32)）。当字面量后缀用在 `const K: Pixel = 32px;` 等编译时常量位置时，允许在 CTFE 对无运行期副作用的 `literal_into` 体执行常量求值；若某个后缀的 `literal_into` 执行了运行期分配（如动态字符串构造）则合法运行于 runtime 并在 `const` 中因阶段检查被拒绝！
4. **负字面量与模式差异（关键边界陷阱）**：
   - **在表达式中**（[docs/basics/literals.md](../../docs/basics/literals.md)）：`-` 是一元前缀运算符 `Neg`，**不是整数字面量词法单元的一部分**！因此 `-32px` 解析为 `Neg(LiteralExtension(32, px))`，即先用 `Integer(32)` 查 `LiteralInto<.px, ?R>` 得到 `Pixel`，再对结果 `R` 调用 `Neg.neg`！由此引出一个经典边界：**`-128i8` 或 `-9223372036854775808i64` 若先求 `128i8` 会在 `LiteralInto<.i8, i8>` 中溢出！** 编译器原生数字字面量折叠必须识别紧邻一元负号与有符号原生整型最小值的组合（`-128i8`），而自定义 `LiteralInto`（如 `-32px`）是否允许直通负数或走 `Neg` 必须在规范中显式说明（见 §5）。
   - **在模式中**（[docs/grammar/syntax](../../docs/grammar/syntax)）：`literal_pattern -> -? (integer | float) id?` 允许负号直接附着于字面量前缀构成负字面量模式匹配。

#### 3.9.3 正反用例

```flurry
pub struct Pixel {
    value: u32,
}

impl LiteralInto<.px, Pixel> for Integer {
    fn literal_into(comptime value: Integer) -> Pixel {
        inline if value < 0 or value > 4294967295 {
            meta.compilation.err("pixel literal out of u32 range");
        }
        Pixel { value: value.as(u32) }
    }
}

-- 同一后缀依据上下文区分结果类型（Miles vs Meters）
test {
    let w: Pixel = 32px;
}
```

**反例（必须拒绝）**：
- `let x = 32pxrem;` -> 拒绝：单个字面量只接受一个后缀标识符。
- `let s = "hi ${name}"custom;`（或 `"hi $name"custom;`）-> 拒绝：插值字符串非编译期纯 `str` 常量，不可附带 `LiteralInto` 后缀。
- 若类型未实现 `Neg` 却在表达式写 `-32px`（当 `Pixel` 未实现 `Neg` 时）-> 拒绝：表达式中 `-` 作用于 `LiteralInto` 结果 `Pixel`。

---

### 3.10 类型转换、默认值与 `Splice`（`From`, `Into`, `TryFrom`, `TryInto`, `AsRef`, `AsMut`, `Default`, `Splice`）

#### 3.10.1 `From` / `Into` / `TryFrom` / `TryInto`（彻底废除 `?T` 与严禁 `Result`）

`library/builtin/protocols/conversion.fl` 当前将 `TryInto` 与 `TryFrom` 定义为返回可选类型 `?Target` / `?Self`，完全丢失转换失败的错误原因！且 `library/meta/numeric/conversions.fl` 同样沿用了 `?Self`。依据 `AGENTS.md` 与 [docs/algebraic-effect-and-delimited-continuation/error-handling.md](../../docs/algebraic-effect-and-delimited-continuation/error-handling.md)，标准转换协议全部升级为原生错误结果 `!Err T`（错误类型 `PascalCaseErr`、变体 `PascalCase`）：

```flurry
-- 归属：library/builtin/protocols/conversion.fl（builtin.convert，由 builtin.prelude 导出）

pub enum InfallibleErr {}

pub enum ConversionErr {
    Overflow,
    Underflow,
    OutOfRange,
    InvalidFormat,
}

pub trait From
where
    Source: Type,
{
    fn from(source: Source) -> Self;
}

pub trait Into
where
    Target: Type,
{
    fn into(self) -> Target;
}

pub trait TryFrom
where
    Source: Type,
{
    assoc Err: Type;
    fn try_from(source: Source) -> !Err Self;
}

pub trait TryInto
where
    Target: Type,
{
    assoc Err: Type;
    fn try_into(self) -> !Err Target;
}

-- 通用互导实现（需配合 Flurry coherence 与特化精度校验）
impl From<T> for T
where
    T: Type,
{
    fn from(source: T) -> T {
        source
    }
}

impl Into<Target> for Source
where
    Source: Type,
    Target: Type,
    requires Target :- From<Source>,
{
    fn into(self) -> Target {
        Target.as(From<Source>).from(self)
    }
}

impl TryFrom<Source> for Target
where
    Source: Type,
    Target: Type,
    requires Source :- Into<Target>,
{
    assoc Err: Type = InfallibleErr;

    fn try_from(source: Source) -> !InfallibleErr Target {
        source.into()
    }
}

impl TryInto<Target> for Source
where
    Source: Type,
    Target: Type,
    requires Target :- TryFrom<Source>,
{
    assoc Err: Type = Target.as(TryFrom<Source>).Err;

    fn try_into(self) -> !Target.as(TryFrom<Source>).Err Target {
        Target.as(TryFrom<Source>).try_from(self)
    }
}
```

**Comptime 对偶族（`meta.convert`）**（修复现有 `meta/numeric/conversions.fl` 误用 `?Self` 的问题）：

```flurry
-- 归属：library/meta/numeric/conversions.fl 等（meta.convert，由 meta.prelude 导出）

pub pure comptime trait ComptimeFrom
where
    Source: Type,
{
    pure comptime fn from(source: Source) -> Self;
}

pub pure comptime trait ComptimeInto
where
    Target: Type,
{
    pure comptime fn into(self) -> Target;
}

pub pure comptime trait ComptimeTryFrom
where
    Source: Type,
{
    assoc Err: Type;
    pure comptime fn try_from(source: Source) -> !Err Self;
}

pub pure comptime trait ComptimeTryInto
where
    Target: Type,
{
    assoc Err: Type;
    pure comptime fn try_into(self) -> !Err Target;
}
```

> **Coherence 边界提示**：若用户尝试同时为具体类型手写 `impl TryFrom<A> for B` 而又存在泛型 `impl TryFrom<Source> for Target where ... requires Source :- Into<Target>`，则按 [docs/traits/coherence.md](../../docs/traits/coherence.md) 校验两者不会产生重叠冲突（因为只要 `A` 没有实现 `Into<B>`，该泛化 impl 即互斥；但如同时手写 `From<A> for B` 与特定 `TryFrom<A> for B` 则被禁止）。另可检查 `!Target.as(TryFrom<Source>).Err Target` 中关联类型投影在错误行处的语法接受度。

#### 3.10.2 `AsRef`、`AsMut` 与 `Default`

```flurry
-- Runtime 借用转换（builtin.convert）：
pub trait AsRef
where
    Target: Type,
{
    fn as_ref(*self) -> *Target;
}

pub trait AsMut
where
    Target: Type,
{
    fn as_mut(*self) -> *Target;
}

-- 默认值（Runtime 与 Comptime 对偶）：
-- Runtime：builtin.convert
pub trait Default {
    fn default_value() -> Self;
}

-- Comptime：meta.convert
pub pure comptime trait ComptimeDefault {
    pure comptime fn default_value() -> Self;
}
```

- **`AsRef`/`AsMut` vs `.as(T)`**：`.as(Target)` 在 Flurry 中承担标量截断/拓宽、newtype 双向表示变换与 trait 视角选择（无额外运行期查找）；`AsRef<Target>` 用于指针级视图转化（如 `*String -> *str` 或 `*Vec<T> -> *Slice<T>` 的通用抽象，配合 `fn open_file(path: *P) where P: Type, requires P :- AsRef<str>`）。由于 Flurry 使用统一指针 `*T`，`AsRef` 与 `AsMut` 的区别在于语义可写契约（仅实现 `AsRef` 的类型不开放经由此视图的写入）。
- **`Default.default_value` vs `default` 命名**：`library/builtin/protocols/traits.fl:197-200` 现使用 `default_value()`（因早期保留/上下文关键字 `default`），将在 §5 列明是否重命名为 `default()` 或保持 `default_value()`。

#### 3.10.3 `Splice` 跨阶段拼接（`builtin.staging`）

沿用 [docs/comptime/quoting-and-splicing.md](../../docs/comptime/quoting-and-splicing.md#L16-L18) 与 `library/builtin/protocols/staging.fl:3-6`：

```flurry
-- 归属：library/builtin/protocols/staging.fl（builtin.staging，由 builtin.prelude 导出）

pub trait Splice {
    assoc RuntimeTarget: Type;
    fn splice(comptime data: Self) -> RuntimeTarget;
}
```

- **两阶段与双宇宙边界**：
  - `~expr` 在运行期表达式（或带引号 `'(...)` 构造体）中以 `comptime` 值 `data: S` 查询 `Splice for S`，将 `comptime` 世界中的标量、`str`、`Symbol`、`meta.List<T>`（当元素可 `Splice` 且目标合适）、定长数组、结构体等转化为等价运行期 `RuntimeTarget` 表达式，遵守 [docs/basics/primitives.md](../../docs/basics/primitives.md) 中的两宇宙拼接规则；
  - `ComptimeOnly` 类型（如 `Type` 本身、编译时 AST handle、包含编译器内部上下文的对象）**严禁实现 `Splice`**（除非其 `RuntimeTarget` 为显式允许降码的数据快照如 `TypeId`）；
  - 含有运行期资源/分配器依赖的类型在 `Splice` 后进入运行期位置时，必须在运行期重新建立或接管合法初始化与 Drop 责任。

**反例（必须拒绝）**：
- `let x: i32 = u8.try_from(300u32);` -> 拒绝：`try_from` 返回 `!ConversionErr u8`，不能未处理/捕获错误直接赋给 `i32`。
- 在运行期对 `let x: Type = i32; let y = ~x;` 执行 `~x` -> 拒绝：`Type` 属于纯 comptime 类型，不实现 `Splice`。

---

### 3.11 管道 `|>`、蕴含 `==>` 与布尔逻辑 `and` / `or` / `not`

#### 3.11.1 Pipeline 运算符 `|>`（`NodeKind.PipePrepend`）

**现存编译器重大 Bug 实证**：
1. 词法 `TokenKind.PipeGt`（`|>`, `compiler/lex/src/token.rs:45`）与格式化（`compiler/fmt/src/exprs.rs:1437, 2617`）存在；
2. `compiler/ast_lowering/src/expr.rs` **完全未实现 `NodeKind.PipePrepend`**，直接报错 unsupported；
3. **优先级错误**：`compiler/fmt/src/exprs.rs:2617`（与 parser 操作符表一致）竟将 `|>` 优先级设在 **`80`**，高于 `* / % ++`（`70`）和 `+ -`（`60`）与比较（`40`）！这会导致 `x + 1 |> double` 错误解析为 `x + (1 |> double)`！

**目标设计契约**：
1. **优先级与结合性修正**：将 `|>` 调整为**低优先级左结合**——推荐优先级 **`35`**（高于 `and`/`or`/`==>`，低于 `==`/`!=`/`<`/`>`/`+`/`*`，从而 `x + 1 |> f |> g == 10` 自然解析为 `((x + 1) |> f) |> g == 10`；备选优先级 **`15`** 低于 `or` 与 `and`，待 §5 裁决）。左结合：`a |> f |> g` 等价于 `(a |> f) |> g`。
2. **语法级前置注入改写（非 trait 重载）**：
   依据 AST 节点命名 `PipePrepend`，`|>` 在 AST lowering / elaboration 期展开为纯语法级调用改写，不经过 trait 分派（从而天然兼容 `%c fn(...)` 闭包、泛型实参 `<T>`、命名可选默认参数与 `comptime` 求值）：
   - **形式 1（函数/闭包值）**：`lhs |> callee` -> `callee(lhs)`（严格先求值 `lhs` 一次，再求值 `callee`，再执行调用）；
   - **形式 2（带参数调用前置注入，Elixir `PipePrepend` 风格）**：`lhs |> callee(arg1, arg2, .opt = v)` -> `callee(lhs, arg1, arg2, .opt = v)`，保留方法链 `lhs |> obj.method(arg1)` -> `obj.method(lhs, arg1)`；求值次序仍保证源序：`lhs` -> `obj`/`callee` -> `arg1` -> `arg2` -> `.opt`；
   - **形式 3（命名与泛型参数组合）**：`lhs |> parse<u32>(.radix = 16)` -> `parse<u32>(lhs, .radix = 16)`；
   - **禁止隐式与 `Concat` 混用**，右操作数必须是可调用形式（函数/方法引用或带 `(...)` 的调用表达式）。

**反例（必须拒绝）**：
- `10 |> 20`（右侧非可调用实体）-> 拒绝：`PipePrepend` 右操作数必须为可调用函数/闭包或带参调用式。
- 在当前未修优先级下依赖 `x + 1 |> f` 误解析 -> 必须修正解析器优先表至 `35`（或 `15`）后回归测试覆盖。

#### 3.11.2 逻辑蕴含 `==>`（`NodeKind.BoolImplies`）

**现存编译器 Bug 实证**：
1. `compiler/fmt/src/exprs.rs:2601, 2626` 将 `BoolImplies` 标为**优先级 `10` 且右结合**；
2. **`compiler/parse/src/expressions.rs:194-198` 仅给 `TokenKind.Arrow`（`->`）写了右结合**，导致 `a ==> b ==> c` 被 parser 错误解析为左结合 `(a ==> b) ==> c`，与 `fmt` 自身矛盾！
3. `compiler/ast_lowering/src/expr.rs` **未实现 `BoolImplies`**（`ExprKind.Implication` 仅作 HIR 存根存在）。

**目标设计契约**：
1. **结合性与优先级**：必须同步修复 parser 为**优先级 `10`、右结合**（`a ==> b ==> c` 解析为 `a ==> (b ==> c)`，符合数学与逻辑惯例）。
2. **双上下文精确语义（不作为普通用户 trait 重载）**：
   - **在运行期 / 编译期普通 `bool` 表达式中**：`lhs ==> rhs` 等价于**短路实质蕴含** `(not lhs) or rhs`：先求值 `lhs: bool` 一次；仅当 `lhs == true` 时求值 `rhs: bool` 并返回其值；当 `lhs == false` 时跳过 `rhs` 直接返回 `true`（因此 `(x != null) ==> (x? > 0)` 具备与 `and` 同等的 occurrence typing 前提守卫安全，不触发空解包 panic！）。
   - **在 PTC / `requires` / `ensures` / `assumes` 规范上下文中**：直接降码为 PTC 命题蕴含连接词 `lhs -> rhs`（例如 [docs/type/prompt.fl](../../docs/type/prompt.fl#L369) 中 `Range.disjoint(r1, r2) ==> ...`；非权威参考来源，语义以 [docs/type/propositional-constraints/index.md](../../docs/type/propositional-constraints/index.md) 为准）。
   - **严禁将 `==>` 对普通用户 trait 开放或用于 `BitVec<N>`**（若需位级蕴含应使用 `(not a) or b`）。

#### 3.11.3 布尔逻辑 `and` / `or` / `not` 规则总览

| 操作符 | `bool` 语义 | `BitVec<N>` 语义 | 用户类型扩展能力 | Occurrence Typing |
|---|---|---|---|---|
| `and`（prec 30，左结合） | **短路与**：左为 `false` 不求值右 | **位与（急切）**：双边必求值，禁止短路 | **不可重载**（防止破坏短路与静态流分析） | 左分支 `x != null` / `x is Pat` 事实在右操作数内立即成立 |
| `or`（prec 20，左结合） | **短路或**：左为 `true` 不求值右 | **位或（急切）**：双边必求值，禁止短路 | **不可重载** | 反向/否证事实（`not (x == null)`）沿分支合取传播 |
| `not`（前缀一元） | 布尔逻辑非 | 逐位按位取反 | **可重载**，经由 `builtin.ops.Not.invert` / `meta.ops.ComptimeNot.invert` | 反转 `bool` 守卫条件谓词（用户 `Not` 不参与内置 `bool` 谓词收窄） |

---

### 3.12 两相策略矩阵与 `language-items.conf` 完整注册表

#### 3.12.1 `pure comptime trait` 对偶策略 vs `^attrs.comptime_phase`

依据 [docs/comptime/the-comptime-phases-of-traits.md](../../docs/comptime/the-comptime-phases-of-traits.md) 与 [docs/type/member-projection.md](../../docs/type/member-projection.md#L29-L46)：

1. **何时采用 `pure comptime trait Comptime*` 独立对偶**（本组协议主力方案）：
   - 凡涉及 **Runtime `*self` 借用指针 / `*Output` 返回指针 / `!AllocErr` 分配策略 / 随机 `BuildHasher`** 的协议（`Add..Rem`、`Neg`、`Not`、`AddAssign..RemAssign`、`Concat`、`Eq`、`Ord`、`TotalOrd`、`Hash`、`Hasher`、`BuildHasher`、`Index`、`From`、`Into`、`TryFrom`、`TryInto`、`Default`、`RangeBounds`），由于 Runtime 版签名带 `*T` 或运行期分配模型而 Comptime 版**禁止指针、使用值引用 `self`**，两者函数签名与 ABI 天然不同，**必须拆分为 Runtime `trait` 与 `meta.*` 中的 `pure comptime trait Comptime*`**，并**强制保持完全一致的方法名**（绝不加 `comptime_` 前缀或 `_comptime` 后缀）。
   - 同时存在于两阶段的类型（如 `i32`、`BitVec<N>`、`str`、`Range<usize>`），在 comptime 未限定查找中由编译器优先选择 `pure comptime trait` 层。
2. **何时保留单一 Runtime `trait`（可辅以 `comptime` 参数或 `^attrs.comptime_phase`）**：
   - 仅当两阶段签名完全相同或协议自身就是**跨阶段桥接**（如 `LiteralInto.literal_into(comptime value: Self) -> R`、`Splice.splice(comptime data: Self) -> RuntimeTarget`）；此时普通 trait 不进入 `pure comptime trait` 优先层，但无副作用纯函数可在 `const`/CTFE 中按规范求值。
   - **纯运行期硬件指针协议**（`Deref`、`DerefMut`、`Reference`、`IndexMut`、`AsRef`、`AsMut`）：仅存在 Runtime 形态，不设 comptime 对偶。
3. **过期文档修复声明**：`library/builtin/protocols/README.md:3` 所写的 *"协议跨 comptime/runtime 的适用范围由声明确定，不按调用阶段复制 trait"* 已过时，与正式文档 [docs/comptime/the-comptime-phases-of-traits.md](../../docs/comptime/the-comptime-phases-of-traits.md) 及已落地的 `meta.ComptimeEq`/`ComptimeHash`/`ComptimeOrd` 冲突，必须在后续库文档提交中同步更新。

#### 3.12.2 完整 `library/language-items.conf` & `SysrootBindings` 注册表

当前 `library/language-items.conf`（`version = 1`）与 `compiler/luna/src/protocols/type_constructors.rs`（`SysrootBindings`）及 `compiler/elaboration/src/program.rs`（`Protocols`）的完整对齐清单（**显式区分当前 sysroot 实配路径与目标规范/重导出路径**；除本文档外另有资源/模式/迭代语言项保留在列以保持全表完整）：

| 配置键（`language-items.conf`） | 对应 CLI 覆写参数 | 当前实配路径（`library/language-items.conf`） | 目标规范路径（含新增/统一重导出） | 现状 |
|---|---|---|---|---|
| `add-protocol` .. `not-protocol`（7 键） | `--add-protocol` .. `--not-protocol` | `builtin.protocols.ops.{Add,Sub,Mul,Div,Rem,Neg,Not}` | `builtin.protocols.ops.*`（`builtin.prelude` / `builtin.ops` 重导出） | **已存在并接线** |
| `eq-protocol`, `ord-protocol` | `--eq-protocol`, `--ord-protocol` | `builtin.numeric.comparison.{Eq,Ord}` | `builtin.numeric.comparison.{Eq,Ord}`（`builtin.prelude` / `builtin.cmp` 重导出） | **已存在并接线** |
| `index-protocol` | `--index-protocol` | `builtin.protocols.ops.Index` | `builtin.protocols.ops.Index` | **已存在并接线** |
| `splice-protocol` | `--splice-protocol` | `builtin.protocols.staging.Splice` | `builtin.protocols.staging.Splice` | **已存在并接线** |
| `literal-into-protocol` | `--literal-into-protocol` | **未配置（缺失）** | `builtin.protocols.conversion.LiteralInto` | **编译器已支持，但 `library/language-items.conf` 漏配且 `builtin` 缺声明！** |
| `add-assign-protocol` .. `rem-assign-protocol`（5 键） | `--add-assign-protocol` .. `--rem-assign-protocol` | **未配置** | `builtin.protocols.ops.{AddAssign,SubAssign,MulAssign,DivAssign,RemAssign}` | **待新增** |
| `concat-protocol` | `--concat-protocol` | **未配置** | `builtin.protocols.ops.Concat` | **待新增** |
| `index-mut-protocol` | `--index-mut-protocol` | **未配置** | `builtin.protocols.ops.IndexMut` | **待新增** |
| `space-apply-protocol` | `--space-apply-protocol` | **未配置** | `builtin.protocols.ops.SpaceApply` | **待新增** |
| `variadic-collector-protocol` | `--variadic-collector-protocol` | **未配置** | `builtin.protocols.ops.VariadicCollector` | **待新增** |
| `deref-protocol`, `deref-mut-protocol`, `reference-protocol` | `--deref-protocol`, `--deref-mut-protocol`, `--reference-protocol` | **未配置** | `builtin.protocols.ops.{Deref,DerefMut,Reference}` | **待新增** |
| `range-full-type`, `range-from-type`, `range-to-type`, `range-to-inclusive-type`, `range-type`, `range-inclusive-type` | `--range-*-type` | **未配置** | `builtin.range.{RangeFull,RangeFrom,RangeTo,RangeToInclusive,Range,RangeInclusive}` | **待新增** |
| `comptime-add-protocol` .. `comptime-not-protocol`（7 键） | `--comptime-add-protocol` .. `--comptime-not-protocol` | **未配置** | `meta.ops.{ComptimeAdd,ComptimeSub,ComptimeMul,ComptimeDiv,ComptimeRem,ComptimeNeg,ComptimeNot}` | **待新增** |
| `comptime-add-assign-protocol` .. `comptime-rem-assign-protocol`（5 键） | `--comptime-add-assign-protocol` .. | **未配置** | `meta.ops.{ComptimeAddAssign..ComptimeRemAssign}` | **待新增** |
| `comptime-concat-protocol` | `--comptime-concat-protocol` | **未配置** | `meta.ops.ComptimeConcat` | **待新增** |
| `comptime-eq-protocol`, `comptime-ord-protocol` | `--comptime-eq-protocol`, `--comptime-ord-protocol` | **未配置** | `meta.numeric.{equality.ComptimeEq,ordering.ComptimeOrd}`（`meta.cmp` 重导出） | **待新增** |
| `comptime-index-protocol` | `--comptime-index-protocol` | **未配置** | `meta.ops.ComptimeIndex` | **待新增** |
| （非本专章既有键，保留兼容） | `--copy-protocol`, `--drop-protocol`, `--iterator-protocol`, `--into-iterator-protocol`, `--*-list-pattern-protocol`, `--meta-list-type`, `--tuple-type` | `builtin.protocols.{resources,iterator,patterns}.*`, `meta.List`, `builtin.types.tuple.Tuple`, `meta.data.patterns.ComptimeListPatternView` | 同左 | **已存在并接线** |

---

## 4. 排期优先级与分阶段交付路线

### Phase 0（`P0` — 修复语义破坏、补全核心基础，解除标准库阻塞）

- **前置依赖**：无外部阻塞，直接依赖现有 `luna` 前端、`elaboration` 与 `library/` sysroot。
- **核心交付项**：
  1. **补齐 `LiteralInto` 声明与 `language-items.conf`**：添加 `builtin.convert.LiteralInto` 并在 `library/language-items.conf` 补入 `literal-into-protocol`，修复 `str`（含拒绝插值）与 `char` 测试。
  2. **修正 `Eq` / `Ord` / `Hash` 借用接收者与升级 `TryFrom`/`TryInto` 为 `!Err T`**：去除 `Eq.eq`、`Ord.cmp`、`Hash.hash` 的按值消费 bug，同步改造 `library/std/string_impl/string.fl` 与 `library/std/collections_impl/hash_map.fl`（去除无谓键克隆）；将 `TryFrom`/`TryInto` 从 `?T` 迁移至 `assoc Err: Type` + `!Err T`。
  3. **修正 `Index` `*self` 接收者、接入 `IndexMut` 与 `AddAssign..RemAssign`**：修复 `ops.fl:84` 的 `fn index(self, ...)` 悬垂签名；在 `places.rs` 支持 `vec[i] = v`、`vec[i] += 1` 左值单次定位与原地写回（不再强制 `Copy`）。
  4. **落地六种 `Range` 类型与整数迭代 + `Concat` (`++`) + 算术原语缺口**：实现 `RangeFull..RangeInclusive` 并接通 `H.Range` elaboration 与整数 `Iterator`（特别覆盖 `0u8..=255u8`）；接通 `Concat` 于 `BitVec`/`Array`/`Vec`/`String`；补齐有符号整数 `/`/`%` 与浮点 `/`/`%` 原生处理；在 `compiler/elaboration` 落地两相 `pure comptime trait` 查找层并修正 `builtin/protocols/README.md`。
- **P0 验收测试大纲**：
  - `literal_into_sysroot.rs`：验证 `32px` 上下文 `R` 推导、隔离候选错误、禁用双后缀、禁用插值串后缀、负字面量 `-32px` 对 `Neg` 调用；
  - `comparison_hashing_borrow.rs`：非 `Copy` 的 `String`/`Vec<i32>` 连续执行 `s1 == s2`、`s1 < s2`、`HashMap` 存取不移动也不报错；
  - `index_mut_compound_assign.rs`：带副作用计数器的 `slot().vec[idx()] += delta` 验证 `slot()` 与 `idx()` 各只求值一次；非 `Copy` 大对象执行 `AddAssign`；
  - `range_iteration_and_concat.rs`：`0u8..=255u8` 遍历刚好 256 次且不溢出；`BitVec<4> ++ BitVec<4>` 与 `[1, 2] ++ [3]` 类型正确；动态 runtime `str ++ str` 被拒绝；
  - `try_from_error_row.rs`：`u8.try_from(300u32)` 返回 `!ConversionErr u8`，配合 `catch` 精确匹配 `.Overflow`。

### Phase 1（`P1` — 人体工学、Comptime 对偶完备与解析修正）

- **前置依赖**：Phase 0 完成后执行；`SpaceApply` 需 `VariadicCollector` 与可变参数收集落地。
- **核心交付项**：
  1. **完善流式 `Hasher` / `BuildHasher` / `std.hash.{DefaultHasher, RandomState}`**：接通 `std.collections.HashMap` 默认使用 `RandomState`（SipHash-1-3），支持自定义快速 `BuildHasher`（如 FxHash 策略）。
  2. **实现 `Deref` / `DerefMut` / `Reference` 与三层接收者适配引擎**：使智能指针 `Box<T>`、`Rc<T>` 等通过 `.method()`、`.ref` 与 `.*` 自然衔接；落实防自环、Unknown 挂起、无副作用空跑。
  3. **扩展 `[...args]` 语法并实现 `SpaceApply` + `VariadicCollector` + `RangeBounds`**：升级 `docs/grammar/syntax` 与 `parse` 接受多参下标 `xs[0..2, 0, ..]`，接通 `Index` 优先再 `SpaceApply`（无静默回退）。
  4. **补齐全部 `meta.*` `Comptime*` 对偶**：`ComptimeAdd..ComptimeNot`、`ComptimeAddAssign..ComptimeRemAssign`、`ComptimeConcat`、`ComptimeIndex`、`ComptimeHash`/`ComptimeHasher`/`ComptimeBuildHasher`、`ComptimeRangeBounds`、`ComptimeFrom..ComptimeTryInto`、`ComptimeDefault`，配合 `language-items.conf` 全量键表。
  5. **修复 `|>` 与 `==>` 解析 bug 并完成 `ast_lowering` 降码**：将 `|>` 优先级从 `80` 降至 `35`（或裁决值 `15`），将 `==>` 在 `parse` 修复为右结合，并在 `ast_lowering` / `elaboration` 完整接入。
  6. **裁决并处理 `BitAnd..Shr` 与 `Default.default_value` 命名，补充 `Splice` 容器派生校验**。
- **P1 验收测试大纲**：
  - `pipeline_and_implies_precedence.rs`：`a + 1 |> mul(2) |> add(3) == 11` 与 `p ==> q ==> r` 右结合解析+短路验证（`false ==> panic_expr()` 不触发 panic）；
  - `deref_adaptation_order.rs`：`Box<Widget>` 自动调用 `Widget` `*self` 方法且只调用一次 `deref`；自环 `Deref` 报循环错误；comptime 中 `x.ref`/`p.*` 报错；
  - `space_apply_and_range_bounds.rs`：`vec[1..3]` 产出 `Slice<T>`、多维 `Tensor` 示例 `xs[0..2, 0, ..]` 正常编译、`Index` 歧义不静默回退至 `SpaceApply`；
  - `comptime_operator_twins.rs`：`comptime` 块内自定义定点数算术 `+`/`+=`、`meta.List ++ meta.List`、`ComptimeRangeBounds`（零 `*self`/`.ref`）运行通过。

### Phase 2（`P2` — 高级泛型浮点与细化借用接口）

- **前置依赖**：Phase 1 完成后执行。
- **核心交付项**：
  1. **`TotalOrd` 与 IEEE 754 浮点生态（`TotalFloat<F>` / 固有 `total_cmp`）**：保证 `f16`/`bf16`/`f32`/`f64` 在排序与集合键中存在安全、明确的全序包装。
  2. **`AsRef` / `AsMut` 与通用借用抽象**：协调与 `.as(T)` 及 `Reference` 的边界后在 `std` 文本/路径/集合模块部署。
- **P2 验收测试大纲**：
  - `float_total_ord.rs`：`-0.0` vs `+0.0`、`-NaN` vs `+NaN` 满足严格全序自反与传递性。

### Phase 3（`P3` — 生态观察与静态可判定边界演进）

- **核心交付项**：结合 `05` 完善 `Eq`/`Hash`/`Ord`/`Default` 派生对泛型、依赖参数环境（如 allocator）的自动透传；评估 `|>|`/`|>` 占位符扩展需求与 `Range` 在验证（`verification`）层上的区间抽象融合。

---

## 5. 待作者裁决的语义空缺与冲突（Open Questions）

### OQ-01-01：`BitAnd` / `BitOr` / `BitXor` / `Shl` / `Shr` 的去留与位置
- **来源引用**：[library/builtin/protocols/ops.fl](../../library/builtin/protocols/ops.fl#L51-L79) vs [docs/grammar/index.md](../../docs/grammar/index.md) & [docs/program-synthesis/bitvec-algebra.md](../../docs/program-synthesis/bitvec-algebra.md)。
- **冲突描述**：`ops.fl` 定义了五个传统位运算 trait，但 Flurry 正式语法文档明确宣布**不提供 `&`/`|`/`^`/`<<`/`>>`/`~` 位运算符**，`language-items.conf` 也未绑定这五个 trait，且缺少 `BitNot` 与算术/逻辑右移区分。
- **推荐方案（方案 A）**：从 `builtin.protocols.ops` 中删除 `BitAnd`/`BitOr`/`BitXor`/`Shl`/`Shr`，位操作统一走 `BitVec<N>` 的急切 `and`/`or`/`not`、`builtin.math.bit_*` 与整数固有方法。
- **备选方案（方案 B）**：将它们移出 `ops.fl` 置入 `builtin.numeric.bits`，补全 `BitNot`/`Ashr`/`Lshr`，仅为整数和自定义大整数类型提供统一的泛型具名方法接口（无运算符语法绑定）。

### OQ-01-02：`+=` / `-=` / `*=` / `/=` / `%=` 采取 `AddAssign..RemAssign` trait 还是纯 `Add + writeback` 语法降码
- **来源引用**：[docs/resource-management/bindings.md](../../docs/resource-management/bindings.md#L62-L65) & `compiler/elaboration/src/program/places.rs:293-538`。
- **冲突描述**：`bindings.md` 将复合赋值写成“定位左值、读旧值、复用算术、写回结果”，这一规则要求 `Self :- Copy`（否则读旧值后写回或右侧失败将破坏资源线性性），无法支持 `BigInt`、`Vec`、`Tensor` 等拥有内存对象的原地高效 `+=`。
- **推荐方案（方案 A）**：正式引入 `AddAssign..RemAssign`（`*self` 原地更新）作为首要解析协议；若目标无对应 `*Assign` 实现但 `Self :- Copy + Add` 且目标可写，由编译器安全降码为“单次定位读-算-写”回退（但遇 `*Assign` 候选 `Unknown` 或歧义时绝不静默回退）。
- **备选方案（方案 B）**：彻底要求所有支持 `op=` 的类型显式实现（或 `derive`）`*Assign`，`builtin` 为全部原生整数/浮点提供原生 `*Assign` 行为，移除依赖 `Copy` 的隐式读-写展开。

### OQ-01-03：`Index.index` 返回 `*Output`（槽位指针）vs `Output`（可为值视图）
- **来源引用**：[library/builtin/protocols/ops.fl](../../library/builtin/protocols/ops.fl#L82-L90) vs `proposal_trace/2026-9-7-tensor-algebra.fl:31-39`（非权威近期草案）。
- **冲突描述**：`ops.fl` 定义 `Index.index(self, idx) -> *Output`（其中按值 `self` 是明确 bug 须改 `*self`），强制下标只能返回内存指针，导致无法用 `Index` 返回计算出的视图值（如 `Tensor[Tensor<bool>] -> Tensor<T>` 或 `BitVec[i] -> BitVec<1>`）；而若改 `Index.index(*self, idx) -> Output` 返回值，容器元素存取 `vec[i]` 又需区分为 `Output = *T`（调用方多一层 `*` 或依赖 target-guided read）或另行处理非 `Copy` 元素读取。
- **推荐方案（方案 A）**：`Index` 固定为 `fn index(*self, idx: Idx) -> *Output` 专责“可寻址内部槽位”，所有计算视图/掩码/切片（如 `Vec[lo..hi]`、`Tensor[mask]`、`Tensor[0..2, ..]`）统一归入 `SpaceApply.space_apply(*self, ...) -> Target`。
- **备选方案（方案 B）**：允许 `Index.index(*self, idx: Idx) -> Output` 直接返回任意 `Output`（指针或视图），`IndexMut` 保持 `assoc MutOutput: Type; fn index_mut(*self, idx: Idx) -> *MutOutput`。

### OQ-01-04：`PartialEq` / `PartialOrd` 是否需要从 `Eq` / `Ord` 中拆分以容纳 IEEE 浮点数
- **来源引用**：[library/builtin/numeric/comparison.fl](../../library/builtin/numeric/comparison.fl)、`library/meta/numeric/{equality,ordering}.fl` & [docs/control-flow/pattern-matching/index.md](../../docs/control-flow/pattern-matching/index.md)。
- **冲突描述**：Flurry 当前仅有 `Eq` 与 `Ord`（无 `PartialEq`/`PartialOrd`），并将 `f16`/`bf16`/`f32`/`f64` 排除在外（原生比较走内置分支）。这导致任何泛型算法 `where T :- Eq` / `Ord` 均拒绝 `f32`/`f64`。
- **推荐方案（方案 A）**：保持 `Eq`/`Ord` 单层简洁模型代表等价与全序（原生浮点继续遵守 IEEE `NaN != NaN` 标量语义），新增 `TotalOrd`（`total_cmp`）与 `TotalFloat<F>` 显式包装新类型接入有序集合与排序，避免拆分两倍 trait。
- **备选方案（方案 B）**：效仿 Rust 拆分为 `PartialEq` + `Eq` 与 `PartialOrd` + `Ord`，`==`/`<` 绑定 `Partial*` 使原生浮点可以直接参与普通比较泛型约束。

### OQ-01-05：Pipeline `|>` 的最终优先级与注入形式
- **来源引用**：[docs/grammar/syntax](../../docs/grammar/syntax#L123)、`compiler/fmt/src/exprs.rs:2617`、`compiler/ast_lowering/src/expr.rs`。
- **冲突描述**：当前词法/格式化将 `|>` 错设为优先级 **`80`**（高于 `+`/`-`/`*`/`/`/`++`），完全违背数据流管道书写直觉（`a + 1 |> f` 会解析成 `a + (1 |> f)`）；且 `ast_lowering` 尚未规定仅支持 `x |> f(a, b)` 前置首参（`PipePrepend`），还是亦需占位符或末参注入。
- **推荐方案（方案 A）**：将 `|>` 优先级修正为 **`35`**（低于比较 `40` 与所有算术 `60/70`，高于 `and`/`or`/`==>`，左结合）——若希望 `cond_a or cond_b |> to_flag` 成立也可定为 **`15`**（介于 `==>` `10` 与 `or` `20` 之间，推荐 `35` 或 `15` 二选一，以 `35` 更少意外跨越逻辑连接词，或 `15` 与 Elixir/F# 最一致）；采纳命名所示 Elixir 式首参注入 `x |> f(a, b)` -> `f(x, a, b)` 与 `x |> f` -> `f(x)`。
- **备选方案（方案 B）**：引入显式占位符 `_`（如 `x |> f(a, _, b)`），未写占位符则仅限一元 `f(x)`；或采用末参注入。

### OQ-01-06：`==>` 在 `bool` 与 `parse` 中的精确边界
- **来源引用**：[docs/grammar/syntax](../../docs/grammar/syntax#L121)、`compiler/parse/src/expressions.rs:194-198`、`compiler/fmt/src/exprs.rs:2601,2626`、`docs/type/prompt.fl`。
- **冲突描述**：`parse` 把 `a ==> b ==> c` 解析为左结合而 `fmt` 视为右结合；同时需作者最终拍板 `==>` 在运行期是否严格为短路 `(not lhs) or rhs` 而不开放给用户类型重载。
- **推荐方案（方案 A）**：立即修复 `parse` 为右结合（与 `fmt` 一致，优先级 `10`）；仅作用于 PTC 命题与原生 `bool`（按短路 `(not lhs) or rhs` 求值并支持 occurrence typing），不开放用户重载与 `BitVec` 重载。
- **备选方案（方案 B）**：仅限 PTC 规范子句使用 `==>`，禁止在普通运行期值表达式出现（但需同步移除或报错于现存已接收表达语法的 `bool` 解析场景）。

### OQ-01-07：`Default` 方法名 `default_value` vs `default` 与 `ComptimeHash` 状态传递形式
- **来源引用**：[library/builtin/protocols/traits.fl](../../library/builtin/protocols/traits.fl#L197-L200)、[library/meta/data/hashing.fl](../../library/meta/data/hashing.fl) 与 `AGENTS.md`（comptime 无指针规则）。
- **冲突描述**：
  1. `traits.fl` 使用 `fn default_value() -> Self`，但部分讨论习惯写 `.default()`（需确认是否与关键字/内置名如 `DefaultAllocator.default` 冲突）；
  2. 由于 comptime 严禁 `*H` 指针，流式 `ComptimeHasher` 在 `ComptimeHash.hash` 中需要返回更新后的 `H`（`fn hash(self, state: H) -> H`）还是保持无状态 `fn hash(self) -> u64`。
- **推荐方案**：
  1. 维持 `default_value()` 名称（避免与 `default` 默认策略/分配器字段冲突，且已写入现行库）；
  2. `ComptimeHasher` + `ComptimeHash.hash(self, state: H) -> H` 采用函数式线程返回状态以对齐 Runtime 流式防碰撞契约（同时允许 `DefaultComptimeHasher` 采用确定性常数种子）。

### OQ-01-08：表达式负字面量 `-32px` 与有符号最小值 `-128i8` 的交互
- **来源引用**：[docs/basics/literals.md](../../docs/basics/literals.md)、[docs/traits/protocols.md](../../docs/traits/protocols.md#L141) & `compiler/parse/src/expressions.rs`。
- **冲突描述**：[docs/basics/literals.md](../../docs/basics/literals.md) 明确规定 `-` 为一元前缀运算符而非字面量一部分。因此 `-128i8` 若严格先执行 `LiteralInto<.i8, i8>(128)` 会因 `128 > i8.MAX (127)` 溢出；同理 `-32px` 要求 `Pixel` 额外实现 `Neg` 才能取负。
- **推荐方案（方案 A）**：对无后缀或自带内建有符号整型后缀的负字面量（如 `-128i8`），编译器原子级合并前置 `-` 与整数字面量作范围校验；自定义 `LiteralInto` 保持 `-` 走 `Neg` 协议（使 `u32` 等无符号后缀 `-10u32` 天然报缺失 `Neg` 错误），或显式文档约定需负数构造的自定义后缀实现 `Neg`。
- **备选方案（方案 B）**：保留表达式 `-` 仅走 `Neg` 原则不例外，仅在自定义/原生类型中提供负值构造函数（对 `-128i8` 强制要求写 `-127i8 - 1` 或 `(-128).as(i8)`——体验较差，不推荐）。

### OQ-01-09：`Concat` (`++`) 的失败与分配模型（`String ++ str` vs `BitVec`）
- **来源引用**：`compiler/elaboration/src/program/operators.rs:551-581`、`library/std/string_impl/string.fl` & [docs/program-synthesis/bitvec-algebra.md](../../docs/program-synthesis/bitvec-algebra.md)。
- **冲突描述**：`String` 扩容需要运行时依赖 `.allocator` 并可能产生 `!AllocErr`；若 `String ++ str` 将 `Output` 设为 `!AllocErr String`，链式 `a ++ b ++ c` 无法直接继续作为 `Concat` 左操作数（除非写成 `(a ++ b)! ++ c`）。
- **推荐方案**：Runtime `Concat` 主要面向无分配/确定性拼接（`BitVec`、`Array`、`Tuple`、预留/借用视图或返回 `!AllocErr String` 由调用方显式 `!` 解包，不暗自吞掉 OOM 也不依赖全局默认 panic）；运行期 `str ++ str` 禁止在 `builtin` 提供，强制引导至 `comptime` 或显式分配器 `String`。
