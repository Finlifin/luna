# Compiler Wishlist 05: `meta` 包、CMTT 分阶段代码、宏系统与类型反射 / PTC

> **撰写角色**：`@meta-author`  
> **目标交付物**：`/home/fin/luna/tmp/wishlist/05-meta-cmtt-macros-reflection.md`  
> **状态**：Wave 1 完整需求规格与缺口清单（覆盖编译器前置能力、`Type.*` 查询族与 `library/meta` 六内置包接口契约）

---

## 1. 范围、权威来源与归属包

### 1.1 覆盖范围

本 Wishlist 覆盖 Flurry 编译期元编程、类型约束系统与源码扩展的完整垂直栈，对应以下 8 个子系统域：

1. **`Type` 布局、身份与 ABI 查询**：`size_of`、`align_of`、`stride_of`、`bit_width_of`、`field_offset_of`、`variant_tag_offset_of`、`repr_of`、`type_id`（128 位 Merkle ABI 身份与 `__flurry_stable_since`）、`type_name`、`has_niche`、`is_zst`、`needs_drop`、`has_padding`。
2. **`Type.predicates.*` 14 组性质、`Type.vars.*` 与 PTC 原子 A01–A19**：涵盖标量、结构、类型描述、参数化、限定、字段与变体、类型关系、函数、表示、形状、行、位模式、阶段交界、行为能力全部 14 个公开性质组，`Type.vars.{Closure, EffectRow, ErrorRow, Reachability, Any}` 推断开槽，以及 PTC 与程序验证的严格边界。
3. **`meta.reflection` 类型与程序项反射**：名义 `struct`（`StructInfo`、`FieldInfo`）、**匿名记录 `Ps`（`RecordInfo`，Silver P0-2 阻塞项）**、`enum`（全 6 种变体形态 + 符号/固定位编码）、`union`、`newtype`、`tuple`、`function`、`trait`、`impl`、`module`、`attribute` 与 `clause` 反射；复用并扩展 `library/meta/reflection/structure.fl` 既有接口。
4. **事务型 `meta.definitions` 构造器**：`StructBuilder`、`EnumBuilder`、`UnionBuilder`、`NewtypeBuilder`、`TypeAliasBuilder`、`TraitBuilder`、`ImplBuilder`、`FunctionBuilder`、`ModuleBuilder` 的事务发布、`BuildErr`、可见性、属性、子句与方法挂载。
5. **`meta.data` 编译期容器与基础值类型**：`List<T>`、`Map<K, V>`、`Set<T>`、`Object`、`Integer`、`Float`、`Symbol`、`ComptimeTextBytes`（`str`），全部可变方法严格遵循 **`self` 值引用语义（零 comptime 指针）**，双阶段同名 trait 不加 `comptime_` 前缀。
6. **`meta.code`（CMTT 分阶段代码组装与证据）**：两阶段 rank-1 单洞上下文 `< Ψ |- T >` / `< T -| Ψ >`、`Context`、`Substitution`、`CodeBinding<T>`、`ExprCode`、`PatternCode`、`ArmCode`、`code_match`、`code_sequence`、`code_let`、`code_if`、`code_loop`、`Any` 证据恢复、`Type.FromCode`、上下文扩展/弱化、`quote fn` 与 `template`。
7. **`meta.ast` 与 `meta.macros`**：`SourceSpan`、`Token`、`TokenKind`、`TokenSpacing`、`TokenBuffer`、`ToTokens`、`Parse`、完整结构化 Flurry AST 节点族、4 种构造宏（`^attrs.macros.{ast, peg, macro3, macro4}`）、`define` 模板宏卫生与反射、全部内置宏（`@builtin.str.raw` 等 9 项）。
8. **`meta.compilation`、`meta.diagnostics` 与 `meta.runtime`**：编译期错误/警告/提示、环境变量/目标 `cfg`、fuel 与记忆化、IIR 管线句柄、`DiagnosticBuilder` 多跨度诊断、**`bounded_stack_bytes`（Silver P0-3 阻塞项）** 及宿主桥接沙盒 `meta.runtime.invoke`。

### 1.2 权威依据与非权威来源边界

严格按照 [AGENTS.md](file:///home/fin/luna/AGENTS.md) 确立语义权威层级：

1. **第一顺位（最高权威）**：作者明确确认与本轮协作规则（`AGENTS.md` 第 5 节“不得破坏的语义边界”：无 `Option`/`Result` 枚举、严格点分隔路径 `a.b.c`、无 comptime 指针、错误类型 `PascalCaseErr`/错误变体 `PascalCase`/普通变体 `snake_case`、双阶段同名 trait、禁止使用处直写 `.symbol'builtin`、Type 返回函数 `PascalCase`、仅允许 `^attrs.*` 与 `__flurry_*` 官方键）。
2. **第二顺位（现行正式规范）**：
   - PTC 与类型系统：[atoms.md](file:///home/fin/luna/docs/type/propositional-constraints/atoms.md)、[index.md](file:///home/fin/luna/docs/type/propositional-constraints/index.md)、[occurrence-typing.md](file:///home/fin/luna/docs/type/propositional-constraints/occurrence-typing.md)、[theory-solvers.md](file:///home/fin/luna/docs/type/propositional-constraints/theory-solvers.md)、[solver-protocol.md](file:///home/fin/luna/docs/type/propositional-constraints/solver-protocol.md)、[reflection-and-layout.md](file:///home/fin/luna/docs/type/reflection-and-layout.md)、[type_id.md](file:///home/fin/luna/docs/type/type_id.md)、[type-identity.md](file:///home/fin/luna/docs/type/type-identity.md)、[rows.md](file:///home/fin/luna/docs/type/rows.md)、[structs.md](file:///home/fin/luna/docs/type/user-defined-types/structs.md)、[enums.md](file:///home/fin/luna/docs/type/user-defined-types/enums.md)、[unions.md](file:///home/fin/luna/docs/type/user-defined-types/unions.md)、[newtype-and-typealias.md](file:///home/fin/luna/docs/type/user-defined-types/newtype-and-typealias.md)、[docs/type/qualified-types/*](file:///home/fin/luna/docs/type/qualified-types/closure-type.md)。
   - CMTT 与宏系统：[docs/cmtt/index.md](file:///home/fin/luna/docs/cmtt/index.md)、[contexts-and-substitution.md](file:///home/fin/luna/docs/cmtt/contexts-and-substitution.md)、[patterns-and-arms.md](file:///home/fin/luna/docs/cmtt/patterns-and-arms.md)、[any-and-code-evidence.md](file:///home/fin/luna/docs/cmtt/any-and-code-evidence.md)、[program-construction.md](file:///home/fin/luna/docs/cmtt/program-construction.md)、[quote-functions-and-templates.md](file:///home/fin/luna/docs/cmtt/quote-functions-and-templates.md)、[construction-macro.md](file:///home/fin/luna/docs/macro/construction-macro.md)、[template-macro.md](file:///home/fin/luna/docs/macro/template-macro.md)。
   - Comptime、属性、管线与组织：[docs/comptime/index.md](file:///home/fin/luna/docs/comptime/index.md)、[official-attribute-keys.md](file:///home/fin/luna/docs/comptime/official-attribute-keys.md)、[the-comptime-phases-of-traits.md](file:///home/fin/luna/docs/comptime/the-comptime-phases-of-traits.md)、[docs/code-organization/builtin-packages.md](file:///home/fin/luna/docs/code-organization/builtin-packages.md)、[docs/grammar/index.md](file:///home/fin/luna/docs/grammar/index.md)、[backends-and-artifacts.md](file:///home/fin/luna/docs/pipeline-projection/backends-and-artifacts.md)、[effect-abi.md](file:///home/fin/luna/docs/algebraic-effect-and-delimited-continuation/effect-abi.md)、[error-handling.md](file:///home/fin/luna/docs/algebraic-effect-and-delimited-continuation/error-handling.md)。
3. **非权威参考（仅用于理解历史意图或实现现状，绝不作为语义裁决依据）**：`proposal_trace/*.fl`、`prompt.fl`、`OLD-2024/`、`library/` 当前不完整代码、`flurry_tutorial/`。

### 1.3 六内置包职责边界与归属

| 内置包 | 与本 Wishlist 相关的归属职责 | 明确不归属的内容 |
|---|---|---|
| `builtin` | 原始类型与内建头（`Type`、`Integer`、`Symbol`、`str`、`bool`、固定位宽数值、`void`、`never`、`Any`、`any`、`Tuple`）、`Once` 资源协议、`builtin.math` LLVM 泛型包装、`OptionalOps`/`ErrorResultOps`（原生 `?T`/`!E T` 的 runtime 操作，非枚举）。 | 不承担通用 comptime 容器、反射数据结构或定义构造器；`Type.*` 编译器查询可由 `builtin` 提供底层原语再由 `meta` 导出结构化结果。 |
| `attrs` | 官方属性包装层，集中封装 `__flurry_*` 内部键：`attrs.macros.{ast, peg, macro3, macro4}`、`attrs.packed.{byte, bit}`、`attrs.abi`、`attrs.stable_since`、`attrs.comptime_phase`、`attrs.quote_fn_template.{for_match_arms, allow_non_exhaustive_instance}`，以及待注册的 `^attrs.layout.align`（`__flurry_align`）与 `^attrs.compilation.pipeline`（`__flurry_pipeline`）。 | 不直接执行 AST 解析或代码生成逻辑；宏用户面类型与辅助函数归 `meta.macros` / `meta.ast`。 |
| `meta` | **本 Wishlist 主归属包**。集中绑定 `.symbol'builtin` 于 [library/meta/bindings.fl](file:///home/fin/luna/library/meta/bindings.fl) 及 [library/meta/lib.fl](file:///home/fin/luna/library/meta/lib.fl) 绑定层，向上提供 `meta.data`、`meta.reflection`、`meta.definitions`、`meta.code`、`meta.ast`、`meta.macros`、`meta.compilation`、`meta.diagnostics`、`meta.runtime` 与 `meta.math`（re-export）。 | 不定义 `String`（归 `std`）；不承载程序状态/堆内存验证谓词（归 `verification`）；不接管构建脚本工程图调度（归 `build`）。 |
| `build` | 目标包/配置/构建任务驱动接口（`TargetInfo`、`BuildMode`、`PackageInfo`）。 | 不负责类型反射或 AST 扩展；`meta.compilation.cfg` 只读消费目标配置并受构建失效追踪约束。 |
| `std` | 运行时数据结构（`String`、`Vec`、`Box` 等，采用运行时依赖 allocator 范式）与 runtime IO/并发。 | 不得混用 `meta` 的 `Object` / `List<T>` 作为 runtime 容器；不直接写 `.symbol'builtin`。 |
| `verification` | 程序状态、堆内存、指针别名、资源归还数量与效应前置/后置程序契约。 | 严禁污染 PTC 与 `Type.predicates.*`；类型命题归编译器 PTC 求解器。 |

---

## 2. 现状与缺口全景总表

基于对 `compiler/elaboration/src/program/projections.rs`、`compiler/elaboration/src/program/type_data.rs`、`compiler/nir/src/types.rs`、`compiler/macro_expander/src/registry.rs`、[library/meta](file:///home/fin/luna/library/meta/lib.fl)、[ptc_atoms_status_report_2026-11-21.md](file:///home/fin/luna/ai_dev_logs/ptc_atoms_status_report_2026-11-21.md)、[issues/0003](file:///home/fin/luna/issues/0003-builtin-packages-compiler-blockers.md)、[issues/0004](file:///home/fin/luna/issues/0004-macro-system-carbon.md) 与 [04-compiler-wishlist.md](file:///home/fin/luna/tmp/silver-design/04-compiler-wishlist.md) 的核查，当前实现高度集中在基础标量/名义结构、部分 `meta.data` 值引用容器及简单表层 Token 流；`meta.code`、`meta.compilation`、`meta.diagnostics`、`meta.runtime` 目前仅为 3 行占位 README，匿名记录与绝大多数 PTC 原子完全缺失。

| ID | 领域 | 需求项 / 符号 | 现状（已核实） | 核心缺口 | 优先级 | 目标归属（编译器 / 库） | 关联追踪 |
|---|---|---|---|---|---|---|---|
| `WL-05-01` | `Type` 布局 | `size_of`、`align_of`、`stride_of`、`bit_width_of` | `projections.rs:244-247` 仅实现 `size_of` 与 `align_of`（映射 `nir::TypeLayoutQuery::{SizeOf, AlignOf}`）。 | 缺 `stride_of`（数组步长对齐）、`bit_width_of`（含 bit-packed 宽度）、`IllFormed`/`Unknown` 分级与目标绑定记录。 | `P0` | `elaboration/projections.rs`, `nir/layout.rs`, `meta` | [04-compiler-wishlist.md](file:///home/fin/luna/tmp/silver-design/04-compiler-wishlist.md) |
| `WL-05-02` | `Type` 布局 | `field_offset_of`、`variant_tag_offset_of`、`repr_of`（`TypeRepr`） | 编译器未实现；`meta.reflection` 仅有无偏移的 `struct_field_names/types`。 | 缺字段字节/位偏移查询、enum discriminant 偏移查询、结构化 `TypeRepr` 枚举/记录。 | `P1` | `nir/layout.rs`, `elaboration/projections.rs`, `library/meta/reflection` | [reflection-and-layout.md](file:///home/fin/luna/docs/type/reflection-and-layout.md) |
| `WL-05-03` | `Type` ABI 属性 | `has_niche`、`is_zst`、`needs_drop`、`has_padding` | 编译器尚未暴露为 `Type.*` 公共查询。 | 缺 NIR niche/padding/ZST 查询和资源析构需求推导接口；必须随目标 `cfg` 与资源实现状态联动。 | `P1` | `nir/layout.rs`, `elaboration/projections.rs`, `library/meta/reflection` | `Silver` 容器与枚举优化 |
| `WL-05-04` | `Type` 身份 | `type_id`（128 位 Merkle ABI + `__flurry_stable_since`）、`type_name` | `type_id` 与 `type_name` 未接入；`__flurry_stable_since` 缺编译器消费者。 | 缺 3 分量（`FullPath+version`/`__flurry_stable_since`、Layout、递归 Symbol 依赖+回边）128-bit Merkle 哈希计算与 NF 属性传播；`!E T` 中 `Ok` 保留 ID `0`。 | `P1` | `nir/type_id.rs`, `elaboration/attributes.rs`, `library/attrs` | [type_id.md](file:///home/fin/luna/docs/type/type_id.md), [issues/0003](file:///home/fin/luna/issues/0003-builtin-packages-compiler-blockers.md) |
| `WL-05-05` | `Type.predicates` | 标量与结构分类（组 1–2）与命名对齐 | `projections.rs:156-172` 仅接受 12 个长命名（如 `is_boolean`, `is_integer`, `is_function`）；`nir::TypeKind` 缺 `Union`, `Record`, `Module`, `Char`。 | 缺 `is_char`, `is_anonymous_record`, `is_union`, `is_module`, `bit_width`；长/短命名冲突（`is_bool` vs `is_boolean` 等）未裁决。 | `P0` | `elaboration/projections.rs`, `nir/types.rs`, `ptc` | [atoms.md](file:///home/fin/luna/docs/type/propositional-constraints/atoms.md), [ptc_atoms_status_report](file:///home/fin/luna/ai_dev_logs/ptc_atoms_status_report_2026-11-21.md) |
| `WL-05-06` | `Type.predicates` | 类型描述、参数化、限定（组 3–5）| `is_type`, `is_any`, `is_void`, `is_never`, `has_head`, `HeadOf`, `is_optional`, `is_error_qualified`, `is_effect_qualified`, `is_closure`, `is_reachability` 均未实现。 | 缺原子 `A03 InUniverse`, `A04 Qualified`, `A06 HasHeadShape` 与规范头/载荷项投影；缺 `Tuple`/用户正常型函数头统一查询。 | `P0` | `ptc`, `elaboration/projections.rs`, `nir/types.rs` | [atoms.md](file:///home/fin/luna/docs/type/propositional-constraints/atoms.md), `AGENTS.md` NF 原则 |
| `WL-05-07` | `Type.predicates` | 字段与变体、类型关系、函数属性（组 6–8）| 仅 `A01 TypeEqual` 与 `A10 Implements` 已部分接入；`has_field`, `FieldType`, `compatible`, `subtype`, `is_pure` 等未接通。 | 缺 `A05 HasSlot`, `A08 Compatible`, `A09 Subtype`, `inline if has_field` occurrence typing 绑定增强，以及函数签名阶段/效应查询。 | `P0` | `ptc`, `elaboration/occurrence_typing`, `candidate-solving` | [occurrence-typing.md](file:///home/fin/luna/docs/type/propositional-constraints/occurrence-typing.md), `Silver P0-2` |
| `WL-05-08` | `Type.predicates` | 表示与大小、形状与序列、行理论（组 9–11）| 仅有内部 `IntLe(A12)` 未暴露；`is_not_sized(A11)`, `shape_*`, `row_*(A16/A17/A18)` 全缺失。 | 缺 `A11 NotSized`（依赖 target 与值信息判定）、`SEQ` 理论（形状长度/拼接/广播）及 `ROW` 理论（有序效应行 vs 无序唯一错误行）。 | `P1` | `ptc/solvers/{layout,seq,row}.rs`, `elaboration/projections.rs` | [theory-solvers.md](file:///home/fin/luna/docs/type/propositional-constraints/theory-solvers.md), [rows.md](file:///home/fin/luna/docs/type/rows.md) |
| `WL-05-09` | `Type.predicates` | 位模式、阶段交界、行为能力（组 12–14）| `A13`, `A14`, `A15`, `A03`, `is_copy/clone/drop/once/deref/reference` 未完整曝光。 | 缺 `BV` 位向量宽度/掩码/覆盖求解、`proxy_of`/`has_splice` 阶段桥接判断、以及通过 `A10 Implements` 路由能力查询。 | `P1` | `ptc/solvers/{bv,trait}.rs`, `elaboration/projections.rs` | [atoms.md](file:///home/fin/luna/docs/type/propositional-constraints/atoms.md) |
| `WL-05-10` | PTC 边界与求解器 | PTC 19 原子全覆盖与与程序验证隔离 | 报告 `ai_dev_logs/ptc_atoms_status_report_2026-11-21.md` 确认 19 个原子仅实现 4 个，且无公共边界测试矩阵。 | 补齐 A02–A19 中其余 15 个原子；显式禁止堆状态、指针别名、消费计数进入 PTC；修复 `UnvalidatedModel`/`BudgetExhausted` 语义退化。 | `P0` | `compiler/ptc`, `elaboration/constraints` | [issues/0003](file:///home/fin/luna/issues/0003-builtin-packages-compiler-blockers.md), [ptc_atoms_status_report](file:///home/fin/luna/ai_dev_logs/ptc_atoms_status_report_2026-11-21.md) |
| `WL-05-11` | `Type.vars.*` | 推断开槽全集 | `type_data.rs:155-163` 仅实现 `Type.vars.Closure`。 | 缺 `Type.vars.EffectRow`、`Type.vars.ErrorRow`、`Type.vars.Reachability`（待与 `reachability.md:132` 的 `ReachSet` 对齐）与 `Type.vars.Any`。 | `P0` | `elaboration/type_data.rs`, `nir/types.rs` | [reachability.md](file:///home/fin/luna/docs/resource-management/reachability.md), [closure-type.md](file:///home/fin/luna/docs/type/qualified-types/closure-type.md) |
| `WL-05-12` | `meta.reflection` | 名义 `struct` 与结构化 `*Info` 描述 | `structure.fl` 仅提供扁平列表（`struct_field_names`、`struct_field_types`）和 `Object`。 | 缺 `StructInfo`、`FieldInfo`（含可见性、默认值标志、属性、偏移/索引）、`TupleInfo`、`NewtypeInfo`（含公开可见性与属性）。 | `P1` | `library/meta/reflection/`, `elaboration/reflection` | [reflection-and-layout.md](file:///home/fin/luna/docs/type/reflection-and-layout.md) |
| `WL-05-13` | `meta.reflection` | **匿名记录 `Ps` 反射（Silver P0-2）** | `Term::Record` 在 `nir/types.rs` 对所有谓词均返回 `Some(false)`，无任何字段反射。 | 缺 `RecordInfo`、`record_fields`、以及 `struct_field_names`/`struct_field_types` 与 `has_field`/`FieldType` 对匿名记录 `Ps` 的一等支持。 | `P0` | `nir/types.rs`, `elaboration/projections.rs`, `library/meta/reflection` | [04-compiler-wishlist.md](file:///home/fin/luna/tmp/silver-design/04-compiler-wishlist.md) `P0-2` |
| `WL-05-14` | `meta.reflection` | `enum` 全 6 种变体形态与 `union` 反射 | `structure.fl` 仅支持 `unit`/`tuple`/`named` 形状和 `symbol`/`bits` 编码；无 `union`。 | 缺对 `pattern`、`expr`、`sub_enum` 另外 3 种变体形态的反射支持；缺 `UnionInfo`/`UnionFieldInfo`；缺 `EnumInfo`/`VariantInfo`。 | `P1` | `elaboration/reflection`, `nir/types.rs`, `library/meta/reflection` | [enums.md](file:///home/fin/luna/docs/type/user-defined-types/enums.md), [unions.md](file:///home/fin/luna/docs/type/user-defined-types/unions.md) |
| `WL-05-15` | `meta.reflection` | `function`、`trait`、`impl`、`module` 反射 | 完全缺失。 | 缺 `FunctionInfo`（参数、子句、阶段、纯度、ABI、返回类型、契约）、`TraitInfo`（关联项、超 trait、阶段层）、`ImplInfo`、`ModuleInfo`。 | `P1` | `elaboration/reflection`, `library/meta/reflection` | 派生物宏、依赖注入与文档生成 |
| `WL-05-16` | `meta.reflection` | `attribute` 与 `clause` 反射及旧键清理 | 仅有 `self'attribute.get` 原语；`structure.fl:133` 保留了裸 `.packed` 回退键；缺公开全量属性反射；`__flurry_align` 等键未在 `official-attribute-keys.md` 登记。 | 缺 `AttributeInfo`、`attributes_of`、`get_attribute`、`ClauseInfo`；须清除 `.packed` 裸键回退，接通 `__flurry_*` 消费者，并在 `official-attribute-keys.md`/`library/attrs` 注册 `__flurry_align`（`^attrs.layout.align`）。 | `P0` | `library/meta/reflection`, `elaboration/attributes.rs`, `library/attrs` | [official-attribute-keys.md](file:///home/fin/luna/docs/comptime/official-attribute-keys.md), [issues/0003](file:///home/fin/luna/issues/0003-builtin-packages-compiler-blockers.md) |
| `WL-05-17` | `meta.definitions` | `StructBuilder` / `EnumBuilder` / `NewtypeBuilder` 事务增强 | 现有 `structs.fl`、`enums.fl`、`newtypes.fl` 仅构建无方法无属性的基本类型，直接返回 `Type` 而无结构化错误与可见性/子句。 | 补齐事务失败返回 `!BuildErr Type`、可见性、属性（`^attrs.*`）、`where` 子句、文档与默认值元数据；确保失败不污染编译器状态。 | `P1` | `library/meta/definitions/`, `elaboration/type_builders` | [reflection-and-layout.md](file:///home/fin/luna/docs/type/reflection-and-layout.md) |
| `WL-05-18` | `meta.definitions` | `UnionBuilder`、`TypeAliasBuilder` | 完全缺失。 | 补齐 `UnionBuilder`（安全标签校验与 C ABI 布局属性）与 `TypeAliasBuilder`（含透明参数与受限导出）。 | `P2` | `library/meta/definitions/`, `elaboration/type_builders` | [unions.md](file:///home/fin/luna/docs/type/user-defined-types/unions.md) |
| `WL-05-19` | `meta.definitions` | `TraitBuilder`、`ImplBuilder`、`FunctionBuilder`、`ModuleBuilder` | 完全缺失（仅有类型定义生成，无法生成 trait、impl、函数或模块）。 | 新增程序项与实现定义事务构造器，统一通过 `Build` / `Publish` 提交，保持阶段分离与孤儿/一致性检查。 | `P1` | `library/meta/definitions/`, `elaboration/item_builders` | 宏生成 `impl`/Tier 2 协议派生 |
| `WL-05-20` | `meta.data` | 容器 `self` 值引用语义全面核对与文档纠偏 | `library/meta/data/{list,map,set,object}.fl` 源码已使用 `self` 值引用；但 `library/meta/README.md:5,216` 与 `docs/comptime/index.md` 仍残存 `*self`/`*List<T>` 过时表述；`structs.fl:42` `let draft` 与 `object.fl` `var` 更新语义文档待对齐。 | 在规范中彻底禁用 `*self`/`.ref`/`.*` 于 comptime；补全 `List<T>`/`Map<K,V>`/`Set<T>`/`Object`/`Integer` 完整契约并同步清理历史文档偏差。 | `P0` | `library/meta/README.md`, `docs/comptime/index.md`, `elaboration/comptime_eval` | `AGENTS.md` 第 5 节 |
| `WL-05-21` | `meta.data` | `ComptimeTextBytes` (`str`) 与 `Float` 缺口 | `text.fl` 使用 `u64` 作为长度/索引（与其余容器 `Integer` 不一致）；`meta.data.float` (`Float`) 缺失。 | 统一 `ComptimeTextBytes` 为 `Integer` 精确索引并提供 UTF-8 字符/切片/编码辅助；补全任意/编译期精确浮点 `Float` 接口或明确语义映射。 | `P1` | `library/meta/data/text.fl`, `library/meta/data/float.fl` | `meta.data` 一致性 |
| `WL-05-22` | `meta.data` | 双阶段 trait 与 `OptionalOps` / `ErrorResultOps` 完整性 | `library/meta/data/{optional,errors}.fl` 已实现双阶段同名 trait；但仅含 `unwrap_or` 等，缺部分常用组合子。 | 完善 `ComptimeOptionalOps`/`ComptimeErrorResultOps` 与 `Eq`/`Hash`/`Ord` 严格同名映射，并明确禁止 `comptime_` 前缀和 `Option`/`Result` 枚举。 | `P1` | `library/meta/data/{optional,errors}.fl`, `library/builtin/types/` | `AGENTS.md` 第 3/5 节 |
| `WL-05-23` | `meta.code` (CMTT) | 基础上下文、代码类型与构造拼接原语 | `library/meta/code/README.md` 为空占位；编译器仅接通基础 `'<...>` 与无自由引用/部分模式生成函数（`patterns-and-arms.md:9-13`）。 | 实现 `Context`、`ContextEntry`、`Substitution`、`CodeBinding<T>`、`ExprCode`、`PatternCode`、`ArmCode`、`code_match`/`code_sequence`/`code_let`/`code_if`/`code_loop`。 | `P0` | `library/meta/code/`, `elaboration/cmtt`, `nir/cmtt` | [docs/cmtt/*](file:///home/fin/luna/docs/cmtt/index.md) |
| `WL-05-24` | `meta.code` (CMTT) | `Any` / `any` 证据见证、上下文扩展/弱化与 `Type.FromCode` | `Any` 保留具体代码身份与依赖的跨边界接口尚未在 `meta.code` 公开；`Type.FromCode` 未接入。 | 定义 `Type.FromCode`、运行时证据解包、上下文 `extend`/`weaken`/`rebind` 组合子与“未见证拒绝”错误路径。 | `P1` | `library/meta/code/`, `elaboration/cmtt` | [any-and-code-evidence.md](file:///home/fin/luna/docs/cmtt/any-and-code-evidence.md) |
| `WL-05-25` | `meta.code` (CMTT) | `quote fn`、`template` 与闭包的 staged 常量折叠 | `issues/0003` 指出 `iir/src/staging.rs:3173` staged comptime closure 规范常量路径未打通；`attrs.quote_fn_template.{for_match_arms, allow_non_exhaustive_instance}` 未在 `official-attribute-keys.md` 登记且缺消费者。 | 打通 staged closure 规范化常量、在 `official-attribute-keys.md` 与 `library/attrs` 注册 `quote_fn_template` 相关键、接通开放模式 `use` 默认对应与两阶段不变量检查。 | `P0` | `iir/src/staging.rs`, `elaboration/cmtt`, `library/attrs` | [quote-functions-and-templates.md](file:///home/fin/luna/docs/cmtt/quote-functions-and-templates.md), [issues/0003](file:///home/fin/luna/issues/0003-builtin-packages-compiler-blockers.md) |
| `WL-05-26` | `meta.ast` | `SourceSpan`、`Token`/`TokenBuffer`、`ToTokens`、`Parse` 与结构化 AST 节点 | `ast_impl/nodes.fl` 的 10 个 `Flurry*` 包装仅保存 `{ tree: SyntaxTree, node: Integer }` 索引句柄，无法结构化模式匹配表达式/模式/定义内部组件；无 `Parse` trait。 | 在保留 `SourceSpan`/`TokenBuffer` 精确来源基础上，补全结构化 Flurry AST 节点模型与统一 `Parse` / `ToTokens` 双向 trait。 | `P1` | `library/meta/ast_impl/`, `compiler/parser`, `compiler/macro_expander` | [construction-macro.md](file:///home/fin/luna/docs/macro/construction-macro.md) |
| `WL-05-27` | `meta.macros` | 4 种构造宏属性统一接入与 PEG 文法对象 | `library/attrs/macros_impl/handlers.fl` 定义了 `ast`, `peg`, `macro3`, `macro4`，但 `library/meta/macros_impl/attributes.fl` 漏导出 `ast` 与 `peg`；`issues/0004` 报告顶层 `__flurry_macro_*` 未被 `attributes.rs` 放行；`macro_expander/registry.rs` 仅注册 `define_macro1/2`。 | 统一 `^attrs.macros.{ast, peg, macro3, macro4}` 的编译器属性入口并接通 `macro3`/`macro4` 真正展开路径；补全 PEG `PegGrammar`/`PegRule`/`PegExpr` 构造与类型绑定。 | `P0` | `compiler/macro_expander`, `elaboration/attributes.rs`, `library/meta/macros_impl/` | [issues/0004](file:///home/fin/luna/issues/0004-macro-system-carbon.md), [issues/0003](file:///home/fin/luna/issues/0003-builtin-packages-compiler-blockers.md) |
| `WL-05-28` | `meta.macros` | `define` 模板宏卫生反射与全部内置宏 | `registry.rs` 仅含 `BUILTIN_RAW_STR`、`BUILTIN_RAW_STR_ALIGNED`、`BUILTIN_DEFINE_MACRO1`、`BUILTIN_DEFINE_MACRO2`。 | 补全 `@builtin.{include_str, include_bytes, env, line, file, stringify, concat_idents}`；补全 `define` 模板宏（`N`, `N ...`, `(ids)`）卫生检查、循环展开限制与宏调试反射。 | `P1` | `compiler/macro_expander/src/registry.rs`, `library/meta/macros_impl/` | [template-macro.md](file:///home/fin/luna/docs/macro/template-macro.md) |
| `WL-05-29` | `meta.compilation` | `err`/`warn`/`note`、`env_var`、`cfg`、fuel/记忆化、`^attrs.compilation.pipeline` 与 IIR 投影 | `library/meta/compilation/README.md` 为空占位；`docs/pipeline-projection/` 引用的 `meta.compilation.{err, emit_diagnostic, IIR.Func, IIR.access}` 与 `^attrs.compilation.pipeline`（`__flurry_pipeline`）尚无库文件或官方属性登记。 | 实现带 `SourceSpan` 的编译期诊断退出、沙盒 `env_var`/`cfg`（记录构建依赖）、fuel/memo 查询、冻结后 `IIR` 只读投影句柄，并登记 `__flurry_pipeline`。 | `P0` | `library/meta/compilation/`, `library/attrs`, `elaboration/comptime_eval`, `iir` | [backends-and-artifacts.md](file:///home/fin/luna/docs/pipeline-projection/backends-and-artifacts.md) |
| `WL-05-30` | `meta.diagnostics` | 多跨度 `DiagnosticBuilder`、标签、建议与 fix-it | `library/meta/diagnostics/README.md` 为空占位。 | 设计纯 `self` 值引用构建器（`DiagnosticBuilder`、`SpanLabel`、`Suggestion`、`FixIt`）并贯通宏观/CMTT 展开来源链。 | `P1` | `library/meta/diagnostics/`, `compiler/diagnostics` | 宏与构造器报错体验 |
| `WL-05-31` | `meta.runtime` | **`bounded_stack_bytes`（Silver P0-3）** | `library/meta/runtime/README.md` 为空占位；编译器无静态栈边界分析接口。 | 为给定函数计算跨越至最近 delimited effect 边界的静态最大栈字节数（`?usize`，无限递归或未标注间接调用返回 `null`），配合 `BoundedStackPool`。 | `P0` | `compiler/mir`/`nir`/`llvm`, `library/meta/runtime/` | [04-compiler-wishlist.md](file:///home/fin/luna/tmp/silver-design/04-compiler-wishlist.md) `P0-3`, [effect-abi.md](file:///home/fin/luna/docs/algebraic-effect-and-delimited-continuation/effect-abi.md) |
| `WL-05-32` | `meta.runtime` | 编译期管线/宿主调用 `meta.runtime.invoke` 与能力沙盒 | 仅见于 `docs/pipeline-projection/` 草图（`invoke(fn, meta.compilation.IIR.access(ir))`），库实现与宿主沙盒完全空白。 | 兼容 `IIR.AccessToken` 与显式 `HostCapability` 授权、带 `Diagnostic` 载荷的 `HostInvokeErr`、确定性依赖声明与沙盒控制。 | `P2` | `library/meta/runtime/`, `compiler/comptime_host` | [backends-and-artifacts.md](file:///home/fin/luna/docs/pipeline-projection/backends-and-artifacts.md) |

---

## 3. 详细规格设计

### 3.1 `Type` 布局、身份与 ABI 方法

编译器在 `Type` 命名空间及其中心化绑定层提供静态布局、ABI 与身份查询。所有查询均为 `pure comptime fn`：不产生副作用、不伪造未知值，且每条结果必须在 elaboration 与 PTC 依赖图中绑定当前**目标配置（target `cfg`）**与主体依赖身份，禁止跨目标错误复用（依据 [reflection-and-layout.md](file:///home/fin/luna/docs/type/reflection-and-layout.md)、[type_id.md](file:///home/fin/luna/docs/type/type_id.md)）。

#### 3.1.1 完整签名契约

```flurry
use bindings;
use builtin.{Symbol, Type, bool, str, u128, usize}

pub enum TypeRepr {
    standard,
    c_abi,
    byte_packed,
    bit_packed(usize),
    transparent(Type),
    tagless_enum,
    tagged_enum {
        tag_type: Type,
        tag_offset: usize,
    },
    niche_enum {
        niche_carrier: Type,
    },
    custom_aligned(usize),
}

pub enum LayoutErr {
    NotRuntimeRepresentable,
    UnsizedType,
    UnresolvedType,
    MissingField,
    BitAlignedFieldNotByteAddressable,
    NotAnEnum,
    NoBitFieldLayout,
}

-- Type 布局、ABI 与身份核心查询（由 compiler 提供求值与依赖登记）
impl Type {
    -- 字节大小、最小对齐与数组步长（仅对合法定长 runtime 表示成立）
    pub pure comptime fn size_of(subject: Type) -> usize
    where
        requires not Type.predicates.is_comptime_only(subject),
        requires not Type.predicates.is_not_sized(subject)
    {
        bindings.compiler_size_of(subject)
    }

    pub pure comptime fn align_of(subject: Type) -> usize
    where
        requires not Type.predicates.is_comptime_only(subject),
        requires not Type.predicates.is_not_sized(subject)
    {
        bindings.compiler_align_of(subject)
    }

    -- stride_of(T) = size_of(T) 向上舍入至 align_of(T) 的倍数（元素数组步长）
    pub pure comptime fn stride_of(subject: Type) -> usize
    where
        requires not Type.predicates.is_comptime_only(subject),
        requires not Type.predicates.is_not_sized(subject)
    {
        bindings.compiler_stride_of(subject)
    }

    -- 精确值位宽：考虑 ^attrs.packed.bit、bool(1) 与窄位枚举，不圆整为字节
    pub pure comptime fn bit_width_of(subject: Type) -> !LayoutErr usize {
        bindings.compiler_bit_width_of(subject)
    }

    -- 字段字节偏移：适用于 struct、匿名记录 Ps 与 union（union 字段恒为 0）
    -- 若目标类型为 ^attrs.packed.bit 且字段未字节对齐，返回 error BitAlignedFieldNotByteAddressable
    pub pure comptime fn field_offset_of(subject: Type, field: Symbol) -> !LayoutErr usize {
        bindings.compiler_field_offset_of(subject, field)
    }

    -- 位打包或字节打包字段的精确位偏移（从结构最低有效位/目标端序起始计）
    pub pure comptime fn bit_field_offset_of(subject: Type, field: Symbol) -> !LayoutErr usize {
        bindings.compiler_bit_field_offset_of(subject, field)
    }

    -- 枚举判别标签字节偏移；对于空壳、单变体或全 niche 消去枚举返回 null
    pub pure comptime fn variant_tag_offset_of(subject: Type) -> !LayoutErr ?usize {
        bindings.compiler_variant_tag_offset_of(subject)
    }

    -- 查询目标布局分类描述
    pub pure comptime fn repr_of(subject: Type) -> !LayoutErr TypeRepr {
        bindings.compiler_repr_of(subject)
    }

    -- 128 位 Merkle ABI 哈希身份：
    --   1) 规范 FullPath + 版本或 ^attrs.stable_since
    --   2) 目标绑定 Layout（含尺寸、对齐、偏移、packed、ABI）
    --   3) 递归依赖 Symbol 与循环回边编码（含 NF 头属性传播）
    -- 约定：!E T 中 Ok 分支保留 ID 0 以配合错误路径零开销编码
    pub pure comptime fn type_id(subject: Type) -> !LayoutErr u128 {
        bindings.compiler_type_id(subject)
    }

    -- 供人类阅读与诊断的规范显示名；禁止作为持久化 ABI 键或身份等价判断依据
    pub pure comptime fn type_name(subject: Type) -> str {
        bindings.compiler_type_name(subject)
    }

    -- ABI 与资源性质查询
    pub pure comptime fn has_niche(subject: Type) -> !LayoutErr bool {
        bindings.compiler_has_niche(subject)
    }

    pub pure comptime fn is_zst(subject: Type) -> !LayoutErr bool {
        bindings.compiler_is_zst(subject)
    }

    pub pure comptime fn needs_drop(subject: Type) -> !LayoutErr bool {
        bindings.compiler_needs_drop(subject)
    }

    pub pure comptime fn has_padding(subject: Type) -> !LayoutErr bool {
        bindings.compiler_has_padding(subject)
    }
}
```

#### 3.1.2 前提与语义边界

1. **`Unknown` vs `IllFormed` 分级**：
   - 未实例化的泛型形参 `T` 调用 `Type.size_of(T)` 时，求解器返回 `Unknown`（残余等待具体单态化或约束证据），除非在泛型体内被 `requires not Type.predicates.is_not_sized(T)` 推迟到实例化。
   - **comptime-only 类型**（如 `Type`、`Integer`、`Object`、`meta.ast.TokenBuffer`）传入 `size_of` / `align_of` / `stride_of` 时**直接触发 `IllFormed`**，绝不能返回 `0` 或把 `is_not_sized` 判为 `true`/`false`。
2. **`type_id` 稳定锚点与 `^attrs.stable_since`**：
   - 默认状态下，`type_id(T)` 随编译器主版本号与目标配置变化而变化（不稳定，见 [type_id.md](file:///home/fin/luna/docs/type/type_id.md)）。
   - 仅当定义携带 `^attrs.stable_since("0.1.0")`（内部生成 `__flurry_stable_since`；必须传 `str` 版本串）且未变更签名、布局及传递依赖 `type_id` 时，才能在兼容检查下维持跨版本稳定指纹。
   - 对参数化 NF 头应用参数时（如 `Pair<A, B>`），头上的布局或稳定性属性沿实例化节点传播并纳入子节点递归 Merkle 哈希；自引用指针类型（如 `*Node`）通过统一回边编号避免无限哈希递归。
3. **`needs_drop` 与 `Once` 的正交性**：
   - `Once` 通过 `impl Once for T where T {}` 赋予默认仿射语义；`Type.needs_drop(T)` 为 `false` 表示无需运行 `Drop` 析构代码（仍受线性/仿射移动检查约束），仅当 `T` 或其活跃载荷/字段实现了 `Drop` 时 `needs_drop` 才为 `true`。

#### 3.1.3 拒绝例

```flurry
use attrs.packed;

-- [拒绝例 3.1-A] comptime-only 类型不允许询问目标布局
comptime fn bad_comptime_layout() -> usize {
    Type.size_of(Integer)  -- 错误：IllFormed（Integer 无 runtime 表示）
}

-- [拒绝例 3.1-B] bit_packed 非字节对齐字段禁止读取字节偏移
^attrs.packed.bit
struct Flags {
    valid: bool,   -- bit 0
    mode: u8,      -- bit 1..9（跨字节边界，非字节对齐）
}

comptime {
    let _ = Type.field_offset_of(Flags, .mode)!;  -- 失败：BitAlignedFieldNotByteAddressable
}

-- [拒绝例 3.1-C] 严禁在用户代码中跨模块或通过 .symbol'builtin 绕过绑定
-- comptime fn forbidden_direct_builtin(t: Type) -> usize {
--     .size_of'builtin(t)  -- 错误：禁止在使用处直写 .symbol'builtin
-- }
```

---

### 3.2 `Type.predicates.*` 14 性质组、`Type.vars.*` 与 PTC 原子 A01–A19 完整映射

依据 [atoms.md](file:///home/fin/luna/docs/type/propositional-constraints/atoms.md)、[occurrence-typing.md](file:///home/fin/luna/docs/type/propositional-constraints/occurrence-typing.md) 与 [theory-solvers.md](file:///home/fin/luna/docs/type/propositional-constraints/theory-solvers.md)，PTC 将布尔判定拆分为独立原子（`A01`–`A19`），观察值（如字段类型、位宽、列表长度、限定载荷）作为**观察项**与 `Eq`/`IntLe` 配合工作，否定严格表示逻辑否定而非“求解失败”。

> **命名对齐提示**：当前编译器 `compiler/elaboration/src/program/projections.rs:156-172` 仅实现了 12 个长命名谓词（`is_boolean`, `is_integer`, `is_signed_integer`, `is_unsigned_integer`, `is_floating_point`, `is_struct`, `is_enum`, `is_newtype`, `is_tuple`, `is_pointer`, `is_function`, `is_symbol`）。下表按目标 API 规范列出完整 14 组公开入口，并将返回值直接为 `Type` 的观察项函数按语法规范统一采用 `PascalCase`（其与现有实现及历史文档中的小驼峰/下划线拼写差异见第 5 节 Open Question 5.1 与 5.2）。

#### 3.2.1 14 个性质组完整 API 契约

```flurry
use builtin.{Any, Symbol, Type, bool, usize}
use meta.data.{Integer, List, Object}

pub enum TypeUniverse {
    runtime_value,
    comptime_value,
    type_level,
    code_level,
}

-- 组 1：标量（Scalars）
pub pure comptime fn is_bool(t: Type) -> bool;
pub pure comptime fn is_int(t: Type) -> bool;
pub pure comptime fn is_signed_int(t: Type) -> bool;
pub pure comptime fn is_unsigned_int(t: Type) -> bool;
pub pure comptime fn is_float(t: Type) -> bool;
pub pure comptime fn is_char(t: Type) -> bool;
pub pure comptime fn is_symbol(t: Type) -> bool;
pub pure comptime fn is_comptime_int(t: Type) -> bool;
pub pure comptime fn is_comptime_float(t: Type) -> bool;
pub pure comptime fn is_pointer_sized_int(t: Type) -> bool;
pub pure comptime fn bit_width(t: Type) -> ?usize;

-- 组 2：结构（Structure）
pub pure comptime fn is_tuple(t: Type) -> bool;
pub pure comptime fn is_struct(t: Type) -> bool;
pub pure comptime fn is_anonymous_record(t: Type) -> bool;
pub pure comptime fn is_enum(t: Type) -> bool;
pub pure comptime fn is_union(t: Type) -> bool;
pub pure comptime fn is_newtype(t: Type) -> bool;
pub pure comptime fn is_fn(t: Type) -> bool;
pub pure comptime fn is_pointer(t: Type) -> bool;
pub pure comptime fn is_module(t: Type) -> bool;

-- 组 3：类型描述（Type Descriptions）
pub pure comptime fn is_type(t: Type) -> bool;
pub pure comptime fn is_any(t: Type) -> bool;
pub pure comptime fn is_void(t: Type) -> bool;
pub pure comptime fn is_never(t: Type) -> bool;
pub pure comptime fn is_defined_description(v: Any) -> bool;

-- 组 4：参数化（Parameterization）-- 严格遵守通用 NF 头与任意 comptime 参数规则
pub pure comptime fn has_head(t: Type, head: Any) -> bool;
pub pure comptime fn HeadOf(t: Type) -> ?Type;
pub pure comptime fn type_args_of(t: Type) -> Object;
pub pure comptime fn type_arg_at(t: Type, index: usize) -> ?Any;
pub pure comptime fn arity_of(t: Type) -> ?usize;

-- 组 5：限定（Qualifiers）
pub pure comptime fn is_optional(t: Type) -> bool;
pub pure comptime fn OptionalPayload(t: Type) -> ?Type;
pub pure comptime fn is_error_qualified(t: Type) -> bool;
pub pure comptime fn ErrorRow(t: Type) -> ?Type;
pub pure comptime fn ErrorOkType(t: Type) -> ?Type;
pub pure comptime fn is_effect_qualified(t: Type) -> bool;
pub pure comptime fn EffectRow(t: Type) -> ?Type;
pub pure comptime fn EffectOkType(t: Type) -> ?Type;
pub pure comptime fn is_closure(t: Type) -> bool;
pub pure comptime fn ClosureEnv(t: Type) -> ?Type;
pub pure comptime fn is_reachability(t: Type) -> bool;
pub pure comptime fn ReachabilitySet(t: Type) -> ?Type;
pub pure comptime fn Pointee(t: Type) -> ?Type;

-- 组 6：字段与变体（Fields & Variants）
pub pure comptime fn has_field(t: Type, name: Symbol) -> bool;
pub pure comptime fn FieldType(t: Type, name: Symbol) -> ?Type;
pub pure comptime fn field_count(t: Type) -> ?usize;
pub pure comptime fn has_variant(t: Type, variant_path: List<Symbol>) -> bool;
pub pure comptime fn variant_count(t: Type) -> ?usize;
pub pure comptime fn tuple_len(t: Type) -> ?usize;
pub pure comptime fn TupleElem(t: Type, index: usize) -> ?Type;

-- 组 7：类型关系（Type Relations）
pub pure comptime fn eq(a: Type, b: Type) -> bool;
pub pure comptime fn compatible(a: Any, b: Any) -> bool;
pub pure comptime fn subtype(sub: Type, sup: Type) -> bool;
pub pure comptime fn implements(subject: Type, trait_type: Type) -> bool;
pub pure comptime fn assignable_to(source: Type, target: Type) -> bool;
pub pure comptime fn castable_to(source: Type, target: Type) -> bool;

-- 组 8：函数（Functions）
pub pure comptime fn param_count(fn_type: Type) -> ?usize;
pub pure comptime fn ParamType(fn_type: Type, index: usize) -> ?Type;
pub pure comptime fn ReturnType(fn_type: Type) -> ?Type;
pub pure comptime fn is_pure(fn_type: Type) -> bool;
pub pure comptime fn is_comptime(fn_type: Type) -> bool;
pub pure comptime fn is_inline(fn_type: Type) -> bool;
pub pure comptime fn is_unsafe(fn_type: Type) -> bool;
pub pure comptime fn is_verified(fn_type: Type) -> bool;
pub pure comptime fn is_atomic(fn_type: Type) -> bool;

-- 组 9：表示（Representation）
pub pure comptime fn is_not_sized(t: Type) -> bool;
pub pure comptime fn is_byte_packed(t: Type) -> bool;
pub pure comptime fn is_bit_packed(t: Type) -> bool;
pub pure comptime fn is_volatile(t: Type) -> bool;
pub pure comptime fn is_c_repr(t: Type) -> bool;
pub pure comptime fn is_transparent(t: Type) -> bool;

-- 组 10：形状（Shapes / Sequences）
pub pure comptime fn shape_len(shape: Any) -> Integer;
pub pure comptime fn shape_head(shape: Any) -> ?Any;
pub pure comptime fn shape_tail(shape: Any) -> ?Any;
pub pure comptime fn shape_concat(left: Any, right: Any) -> Any;
pub pure comptime fn broadcast_compatible(left: Any, right: Any) -> bool;

-- 组 11：行（Rows）-- 严格区分无序唯一错误行与有序作用域效应行
pub pure comptime fn row_contains(row: Type, label: Any) -> bool;
pub pure comptime fn row_subset(sub_err_row: Type, sup_err_row: Type) -> bool;
pub pure comptime fn row_disjoint(left_err_row: Type, right_err_row: Type) -> bool;
pub pure comptime fn RowUnion(left_row: Type, right_row: Type) -> Type;
pub pure comptime fn RowDiff(left_row: Type, right_row: Type) -> Type;

-- 组 12：位模式（Bit Patterns / BitVec）
pub pure comptime fn bv_width(pattern_or_type: Any) -> ?usize;
pub pure comptime fn bv_mask(pattern: Any) -> Integer;
pub pure comptime fn bv_overlap(left: Any, right: Any) -> bool;
pub pure comptime fn bv_covers(patterns: List<Any>, domain: Any) -> bool;

-- 组 13：阶段交界（Stage Boundaries）
pub pure comptime fn universe_of(t: Type) -> TypeUniverse;
pub pure comptime fn is_comptime_only(t: Type) -> bool;
pub pure comptime fn has_proxy(t: Type) -> bool;
pub pure comptime fn ProxyOf(t: Type) -> ?Type;
pub pure comptime fn has_splice(code_val: Any) -> bool;

-- 组 14：行为能力（Capabilities）-- 全走 A10 Implements，不绕过资源与 trait 契约
pub pure comptime fn is_copy(t: Type) -> bool;
pub pure comptime fn is_clone(t: Type) -> bool;
pub pure comptime fn is_drop(t: Type) -> bool;
pub pure comptime fn is_once(t: Type) -> bool;
pub pure comptime fn is_deref(t: Type) -> bool;
pub pure comptime fn is_reference(t: Type) -> bool;
```

#### 3.2.2 19 个 PTC 核心原子（A01–A19）完整映射表

下表将 [atoms.md](file:///home/fin/luna/docs/type/propositional-constraints/atoms.md) 定义的全部 19 个核心原子（`A01`–`A19`）一一映射至理论求解器、公开查询与当前实现状态（基于 [ptc_atoms_status_report_2026-11-21.md](file:///home/fin/luna/ai_dev_logs/ptc_atoms_status_report_2026-11-21.md)）：

| 原子 | 模式 | 归属理论 | 公开 API / 语法入口 | 当前状态 | 否定与残余规则 |
|---|---|---|---|---|---|
| `A01` | `Eq(sort, a, b)` | `EUF` + 专有理论 | `T1 == T2`, `Type.predicates.eq`, `N1 == N2` | 已实现（部分） | 结构不同不等于值不同；opaque NF 头不展开。 |
| `A02` | `Kind(T, k)` | `TYPE` | 组 1 标量、组 2 结构、组 3 类型描述 | 已部分实现（12 项） | 仅互斥类别可互推否定；`newtype` 不继承底层分类。 |
| `A03` | `InUniverse(T, u)` | `TYPE` | `is_type`, `universe_of`, `is_comptime_only` | **缺失** | 不默认两宇宙互为否定；开放变量残余为 `Unknown`。 |
| `A04` | `Qualified(T, q)` | `TYPE` | `is_optional`, `is_error_qualified`, `is_effect_qualified`, `is_closure`, `is_reachability` | **缺失** | 载荷由专有项提取；`?T` 不折叠（`??T != ?T`）。 |
| `A05` | `HasSlot(T, s)` | `TYPE` / `SEQ` | `has_field`, `has_variant`, `tuple_len`, `FieldType`, `TupleElem` | **缺失** | 必须区分字段名、路径与元组下标；开放记录/开放变体不可判缺失。 |
| `A06` | `HasHeadShape(n, h)` | `MATCH` | `has_head`, `HeadOf`, `type_args_of`, `arity_of` | **缺失** | 不展开函数体逆推工厂来源；不沿普通等式盲传。 |
| `A07` | `Matches(d, x, p)` | `MATCH` / `SEQ` / `BV` | `x matches P` 类型模式与值模式 | **缺失** | 成功绑定仅进入成功分支作用域；否定不引入绑定。 |
| `A08` | `Compatible(d, a, b)` | `REL` | `Type.predicates.compatible`、`Any`/`any` 匹配 | **缺失** | 不取传递闭包（`compat(A, Any)` 且 `compat(Any, B)` 不推 `compat(A, B)`）。 |
| `A09` | `Subtype(A, B)` | `REL` | `Type.predicates.subtype` | **缺失** | 不由 `castable_to` 推导，不自动生成泛型协变。 |
| `A10` | `Implements(x, t, world)` | `TRAIT` | `T :- Trait`, `implements`, `is_copy`/`is_clone`/`is_drop`/`is_once`/`is_deref`/`is_reference` | 已实现（部分） | 否定须有完整闭合与孤儿规则排除依据；不可只因暂未找到实例即判假。 |
| `A11` | `NotSized(T, target)` | `LAYOUT` | `is_not_sized` | **缺失** | 依赖目标配置；未知类型参数不判为真；comptime-only 类型先报 `IllFormed`。 |
| `A12` | `IntLe(a, b)` | `INT` | 数学 `Integer` 比较 `a <= b`, `0 <= idx and idx < len` | 仅内部存在 | 与定宽机器整数分离；不回退成有回绕溢出的边界。 |
| `A13` | `IntCongruent(a, r, m)` | `INT` | 对齐与步长同余约束（`stride % align == 0` 等） | **缺失** | `m <= 0` 或未知 `m` 报 `IllFormed`/`Unknown`。 |
| `A14` | `BVUnsignedLe(a, b, w)` | `BV` | `bv_width`, `bv_mask`, `bv_overlap`, `bv_covers`（无符号） | **缺失** | 需已知正宽度与一致位宽；否则残余为 `Unknown`。 |
| `A15` | `BVSignedLe(a, b, w)` | `BV` | 固定位宽有符号位向量范围与覆盖 | **缺失** | 严禁与无符号位向量语义混同。 |
| `A16` | `RowContains(k, R, l)` | `ROW` | `row_contains` | **缺失** | `k` 区分 `error` 与 `effect`；开放行尾未知时不判缺失。 |
| `A17` | `RowSubset(error, R, S)` | `ROW` | `row_subset` | **缺失** | **仅限唯一无序错误行**；禁止直接应用于有序 scoped `effect` 行。 |
| `A18` | `RowDisjoint(error, R, S)` | `ROW` | `row_disjoint` | **缺失** | 仅用于唯一错误行并/差不冲突判定；不路由效应 handler。 |
| `A19` | `PredicateCall(def, args, deps)` | `PRED` | `comptime predicate` 调用、`broadcast_compatible` | **缺失** | 展开稳定纯定义；无计算规则时残余为 `Unknown`，不判 `false`。 |

#### 3.2.3 `Type.vars.*` 推断开槽

`Type.vars.*` 仅用于泛型与签名 `where` 子句声明待推断的类型/行/闭包含义占位，**不是普通数据构造器**（`projections.rs`/`type_data.rs:155-163` 当前仅接通 `Type.vars.Closure`）：

| 槽位 | 所在文档依据 | 语义与用途 | 当前实现状态 |
|---|---|---|---|
| `Type.vars.Closure` | [closure-type.md](file:///home/fin/luna/docs/type/qualified-types/closure-type.md) | 函数/回调捕获环境开槽：`%c fn(T)`（`where c: Type.vars.Closure`） | **已实现** |
| `Type.vars.EffectRow` | [effect-type.md](file:///home/fin/luna/docs/type/qualified-types/effect-type.md) | 有序作用域效应行开槽：`#E T`（`where E: Type.vars.EffectRow`） | **缺失** |
| `Type.vars.ErrorRow` | [error-type.md](file:///home/fin/luna/docs/type/qualified-types/error-type.md) | 唯一无序错误行开槽：`!E T`（`where E: Type.vars.ErrorRow`） | **缺失** |
| `Type.vars.Reachability` | [reachability.md](file:///home/fin/luna/docs/resource-management/reachability.md)（注：现行文档使用 `ReachSet`） | 引用/视图可达集开槽（`where r: Type.vars.Reachability`；命名冲突见第 5 节 OQ-5.3） | **缺失** |
| `Type.vars.Any` | [any-and-code-evidence.md](file:///home/fin/luna/docs/cmtt/any-and-code-evidence.md) | 受控任意描述通配位（显式区分未定描述与类型统一变量） | **缺失** |

#### 3.2.4 PTC 与程序验证的严格隔离边界（及拒绝例）

依据 [AGENTS.md](file:///home/fin/luna/AGENTS.md) 第 5 节与 [atoms.md](file:///home/fin/luna/docs/type/propositional-constraints/atoms.md) 第 51 行：
- **PTC 仅处理类型命题**（结构、宇宙、行、trait 证据、定型值约束、comptime `predicate`）。
- **程序状态、堆内存有效性、指针别名（noalias/disjoint）、线性资源剩余计数、`read(x)`/`write(x)` 读写权限以及运行期状态 `requires`/`ensures` 全部归属程序验证层（`verification`）**，严禁作为 `Type.predicates.*` 或 PTC 核心原子引入。

```flurry
-- [拒绝例 3.2-A] 严禁在 Type.predicates 中查询堆/指针别名/资源状态
comptime fn illegal_ptc_state_query(p: Type) -> bool {
    Type.predicates.is_pointer(p) and Type.predicates.is_valid_heap_ptr(p)  -- 错误：无此 PTC 谓词，属程序验证职责
}

-- [拒绝例 3.2-B] 禁止对 scoped effect 行调用错误行子集规则 A17
pure comptime fn illegal_effect_subset(e1: Type, e2: Type) -> bool {
    Type.predicates.row_subset(e1, e2)  -- 若 e1/e2 为 #EffectRow，必须报 IllFormed（效应路由不可擦除重复层序）
}

-- [拒绝例 3.2-C] 禁止对 comptime-only 类型使用 is_not_sized 充当普通布尔拒绝
comptime {
    assumes not Type.predicates.is_not_sized(meta.Object);  -- 错误：IllFormed（Object 不在 runtime 表示域内）
}
```

---

### 3.3 `meta.reflection` 完整反射结构与查询接口

`meta.reflection` 复用并拓展 [library/meta/reflection/structure.fl](file:///home/fin/luna/library/meta/reflection/structure.fl) 的现有扁平列表导出（`struct_field_names`、`struct_field_types`、`enum_*`、`NewtypeRepresentation`），向上交付**结构化、只读且保留声明序与属性元数据**的 `*Info` 描述结构体（所有类型均以 `comptime` 值返回，查询无副作用，不打开不透明头私有内部，也不直接授予成员访问或布局绕过权）。

> **兼容性清理要求**：`library/meta/reflection/structure.fl:133` 现存的裸属性键回退 `self'attribute.get<bool>(.packed)` 必须随 `__flurry_packed_mode` elaboration 消费者打通而**删除**（见 [issues/0003](file:///home/fin/luna/issues/0003-builtin-packages-compiler-blockers.md) 与 [official-attribute-keys.md](file:///home/fin/luna/docs/comptime/official-attribute-keys.md)）。

```flurry
use builtin.{Any, Integer, Symbol, Type, bool, str, usize}
use meta.ast.SourceSpan
use meta.data.{List, Map, Object}

pub enum ReflectErr {
    NotAStruct,
    NotARecord,
    NotAnEnum,
    NotAUnion,
    NotANewtype,
    NotATuple,
    NotAFunction,
    NotATrait,
    NotAnImpl,
    NotAModule,
    OpaqueTypeNotInspectable,
    UnresolvedGenericTarget,
    AttributeNotFound,
}

-- 严格对应 docs/code-organization/visibility.md 三档可见性范围（无 module_only 或受限路径列表）
pub enum Visibility {
    public,   -- pub（全局导出，仍受中间路径可达约束）
    package,  -- 无修饰（声明所属包内默认可见）
    private,  -- private（声明所属模块及其子模块）
}

pub enum AttributeArg {
    bool_lit(bool),
    int_lit(Integer),
    str_lit(str),
    symbol_lit(Symbol),
    type_val(Type),
    list_val(List<AttributeArg>),
    object_val(Object),
}

pub struct AttributeInfo {
    path: List<Symbol>,
    official_key: ?Symbol,
    positional_args: List<AttributeArg>,
    named_args: Map<Symbol, AttributeArg>,
    span: SourceSpan,
}

pub enum ClauseKind {
    type_param,
    value_param,
    trait_bound,
    requires_contract,
    ensures_contract,
    ptc_predicate,
    quote_lift,
}

pub struct ClauseInfo {
    kind: ClauseKind,
    name: ?Symbol,
    is_optional: bool,
    has_default: bool,
    bound_text: str,
    span: SourceSpan,
}

pub struct FieldInfo {
    name: ?Symbol,
    index: Integer,
    field_type: Type,
    visibility: Visibility,
    has_default: bool,
    byte_offset: ?usize,
    bit_offset: ?usize,
    attributes: List<AttributeInfo>,
    span: SourceSpan,
}

impl FieldInfo {
    pub pure comptime fn FieldType(self) -> Type {
        self.field_type
    }
}

-- 1. 名义 struct 反射
pub struct StructInfo {
    name: Symbol,
    full_path: List<Symbol>,
    visibility: Visibility,
    clauses: List<ClauseInfo>,
    fields: List<FieldInfo>,
    repr: TypeRepr,
    attributes: List<AttributeInfo>,
    span: SourceSpan,
}

-- 2. 匿名记录 Ps 反射（Silver P0-2 核心阻塞项）
--    与 StructInfo 区分：匿名记录无名义 full_path、无声明可见性，但保留有序/具名结构槽
pub struct RecordInfo {
    fields: List<FieldInfo>,
    is_open_tail: bool,
    attributes: List<AttributeInfo>,
}

-- 3. enum 反射：覆盖 docs/type/user-defined-types/enums.md 全部 6 种变体形态 + 编码
pub enum VariantKind {
    trivial,        -- .none
    tuple,          -- .rgb(u8, u8, u8)
    struct_record,  -- .point { x: i32, y: i32 }
    pattern_variant,-- 模式变体 / 视图限定变体
    expr_variant,   -- 常量表达式或表达式派生判别值变体
    sub_enum,       -- 嵌套子枚举委托变体
}

pub enum EnumEncoding {
    default_tagged,
    symbol_coded(List<Symbol>),
    bits_coded {
        width: Integer,
        base_type: ?Type,
    },
}

pub struct VariantInfo {
    path: List<Symbol>,
    kind: VariantKind,
    fields: List<FieldInfo>,
    symbol_code: ?Symbol,
    bit_mask: ?Integer,
    bit_value: ?Integer,
    bit_offsets: List<Integer>,
    sub_enum_type: ?Type,
    pattern_or_expr_source: ?str,
    attributes: List<AttributeInfo>,
    span: SourceSpan,
}

impl VariantInfo {
    pub pure comptime fn SubEnumType(self) -> ?Type {
        self.sub_enum_type
    }
}

pub struct EnumInfo {
    name: Symbol,
    full_path: List<Symbol>,
    visibility: Visibility,
    clauses: List<ClauseInfo>,
    groups: List<List<Symbol>>,
    variants: List<VariantInfo>,
    encoding: EnumEncoding,
    attributes: List<AttributeInfo>,
    span: SourceSpan,
}

-- 4. union 反射（C ABI 原生联合体；反射安全观察 schema，成员读写仍属 runtime unsafe）
pub struct UnionFieldInfo {
    name: Symbol,
    index: Integer,
    field_type: Type,
    attributes: List<AttributeInfo>,
    span: SourceSpan,
}

impl UnionFieldInfo {
    pub pure comptime fn FieldType(self) -> Type {
        self.field_type
    }
}

pub struct UnionInfo {
    name: Symbol,
    full_path: List<Symbol>,
    visibility: Visibility,
    clauses: List<ClauseInfo>,
    fields: List<UnionFieldInfo>,
    is_c_abi: bool,
    attributes: List<AttributeInfo>,
    span: SourceSpan,
}

-- 5. newtype 与 tuple 反射
pub struct NewtypeInfo {
    name: Symbol,
    full_path: List<Symbol>,
    visibility: Visibility,
    clauses: List<ClauseInfo>,
    underlying: Type,
    is_transparent_abi: bool,
    attributes: List<AttributeInfo>,
    span: SourceSpan,
}

impl NewtypeInfo {
    pub pure comptime fn UnderlyingType(self) -> Type {
        self.underlying
    }
}

pub struct TupleInfo {
    elements: List<Type>,
    length: Integer,
    byte_offsets: List<?usize>,
}

-- 6. function / param / contract 反射
pub enum FunctionStage {
    runtime_fn,
    comptime_fn,
    pure_comptime_fn,
    quote_fn,
}

pub struct ParamInfo {
    name: ?Symbol,
    index: Integer,
    param_type: Type,
    is_named_optional: bool,
    has_default: bool,
    is_comptime_param: bool,
    is_quote_lift: bool,
    attributes: List<AttributeInfo>,
}

impl ParamInfo {
    pub pure comptime fn ParamType(self) -> Type {
        self.param_type
    }
}

pub struct ContractInfo {
    is_requires: bool,
    predicate_source: str,
    span: SourceSpan,
}

pub struct FunctionInfo {
    name: Symbol,
    full_path: List<Symbol>,
    visibility: Visibility,
    stage: FunctionStage,
    is_inline: bool,
    is_unsafe: bool,
    is_verified: bool,
    is_atomic: bool,
    abi: ?Symbol,
    clauses: List<ClauseInfo>,
    params: List<ParamInfo>,
    return_type: Type,
    error_row: ?Type,
    effect_row: ?Type,
    closure_env: ?Type,
    contracts: List<ContractInfo>,
    attributes: List<AttributeInfo>,
    span: SourceSpan,
}

impl FunctionInfo {
    pub pure comptime fn ReturnType(self) -> Type {
        self.return_type
    }
}

-- 7. trait / impl / module 反射
pub enum TraitPhaseKind {
    runtime_only,
    pure_comptime_only,
    dual_phase_member,
}

pub enum AssocItemKind {
    assoc_type,
    assoc_const,
}

pub struct AssocItemInfo {
    name: Symbol,
    kind: AssocItemKind,
    item_type: Type,
    has_default: bool,
    attributes: List<AttributeInfo>,
}

pub struct TraitInfo {
    name: Symbol,
    full_path: List<Symbol>,
    visibility: Visibility,
    phase_kind: TraitPhaseKind,
    clauses: List<ClauseInfo>,
    supertraits: List<Type>,
    assoc_items: List<AssocItemInfo>,
    methods: List<FunctionInfo>,
    attributes: List<AttributeInfo>,
    span: SourceSpan,
}

pub struct ImplInfo {
    target_type: Type,
    trait_type: ?Type,
    is_builtin_orphan_exempt: bool,
    clauses: List<ClauseInfo>,
    methods: List<FunctionInfo>,
    attributes: List<AttributeInfo>,
    span: SourceSpan,
}

pub enum ExportKind {
    type_item,
    function_item,
    const_item,
    trait_item,
    module_item,
    macro_item,
}

pub struct ExportInfo {
    name: Symbol,
    kind: ExportKind,
    visibility: Visibility,
    is_reexport: bool,
    target_type: ?Type,
    span: SourceSpan,
}

pub struct ModuleInfo {
    name: Symbol,
    full_path: List<Symbol>,
    visibility: Visibility,
    exports: List<ExportInfo>,
    submodules: List<List<Symbol>>,
    attributes: List<AttributeInfo>,
    span: SourceSpan,
}

-- 统一反射查询入口（同时保留既有 library/meta/reflection/structure.fl 方法）
pub pure comptime fn reflect_struct(t: Type) -> !ReflectErr StructInfo;
pub pure comptime fn reflect_record(ps: Type) -> !ReflectErr RecordInfo;
pub pure comptime fn reflect_enum(t: Type) -> !ReflectErr EnumInfo;
pub pure comptime fn reflect_union(t: Type) -> !ReflectErr UnionInfo;
pub pure comptime fn reflect_newtype(t: Type) -> !ReflectErr NewtypeInfo;
pub pure comptime fn reflect_tuple(t: Type) -> !ReflectErr TupleInfo;
pub pure comptime fn reflect_function(fn_value: Any) -> !ReflectErr FunctionInfo;
pub pure comptime fn reflect_trait(trait_type: Type) -> !ReflectErr TraitInfo;
pub pure comptime fn reflect_impls_of(subject: Type) -> !ReflectErr List<ImplInfo>;
pub pure comptime fn reflect_module(mod_item: Any) -> !ReflectErr ModuleInfo;

-- 属性与子句集中检查
pub pure comptime fn attributes_of(target: Any) -> List<AttributeInfo>;
pub pure comptime fn get_attribute(target: Any, attr_path: List<Symbol>) -> ?AttributeInfo;
pub pure comptime fn clauses_of(target: Any) -> List<ClauseInfo>;
```

#### 3.3.1 Silver P0-2 匿名记录 `Ps` 与 Occurrence Typing 联动示例

`Term::Record`（匿名记录类型 `Ps`）在支持 `Type.predicates.is_anonymous_record`、`has_field`、`FieldType`、`reflect_record` 以及 `struct_field_names`/`struct_field_types` 后，Silver 数据框架与外部 Tier 2 生态包（如位于六内置包之外的 `serialization` 协议派生）即可零开销按列投影：

```flurry
-- 结合 PTC has_field 与 inline if occurrence typing 安全访问匿名记录字段
pub fn read_id_or_default(row: Ps) -> u64
where
    Ps: Type,
    requires Type.predicates.is_anonymous_record(Ps)
{
    inline if Type.predicates.has_field(Ps, .id) do {
        inline if Type.predicates.FieldType(Ps, .id) is id_type? do {
            inline if Type.predicates.eq(id_type, u64) do {
                row.id  -- 分支局部 PTC 事实确认 row.id 存在且类型为 u64
            } else {
                0u64
            }
        } else {
            0u64
        }
    } else {
        0u64
    }
}

-- 编译期枚举匿名记录 Ps 的全部字段并生成列描述
pub pure comptime fn record_column_names(ps: Type) -> !ReflectErr List<Symbol> {
    let info = meta.reflection.reflect_record(ps)!;
    var names: List<Symbol> = [];
    inline for field in info.fields {
        inline if field.name is name? do {
            names.append(name);
        }
    }
    names
}
```

---

### 3.4 事务型 `meta.definitions` 构造器

`meta.definitions` 负责在编译期安全地声明和发布新类型与程序项。依据 [reflection-and-layout.md](file:///home/fin/luna/docs/type/reflection-and-layout.md)，**所有构造器草稿在调用 `.Build()` 或 `.publish()` 之前均仅为普通不可观测值**，且全部可变配置方法采用 `self` 值引用语义（禁止 `*self`、`.ref`、`.*`）。

> **迁移说明**：现有 [library/meta/definitions/structs.fl](file:///home/fin/luna/library/meta/definitions/structs.fl)、[enums.fl](file:///home/fin/luna/library/meta/definitions/enums.fl) 与 [newtypes.fl](file:///home/fin/luna/library/meta/definitions/newtypes.fl) 的 `.Build(self) -> Type` 直接触发编译报错；目标规格统一升级为返回 `!BuildErr Type` / `!BuildErr ItemHandle`，确保生成器可捕获构造错误并实现事务回滚。

#### 3.4.1 错误、规格描述与 9 类事务构造器

```flurry
use builtin.{Any, Integer, Symbol, Tuple, Type, bool, str}
use meta.ast.SourceSpan
use meta.code.ExprCode
use meta.data.{List, Object}
use meta.reflection.{AttributeArg, FunctionStage, TypeRepr, Visibility}

pub enum BuildErr {
    DuplicateField,
    DuplicateVariant,
    DuplicateMethod,
    InvalidLayout,
    CycleByValue,
    EmptyUnion,
    InvalidSymbolCode,
    BitWidthMismatch,
    InvalidVisibility,
    OrphanViolation,
    InvalidStage,
    UnresolvedClause,
    InvalidAttribute,
    ConflictingImpl,
    AlreadyPublished,
    ReservedName,
}

-- 统一元数据配置项
pub struct AttributeSpec {
    path: List<Symbol>,
    positional_args: List<AttributeArg>,
    named_args: List<Tuple<Symbol, AttributeArg>>,
}

pub struct ClauseSpec {
    is_value_param: bool,
    name: Symbol,
    bound_or_type: Type,
    is_optional: bool,
    default_expr: ?ExprCode,
}

pub struct ParamSpec {
    name: Symbol,
    param_type: Type,
    is_named_optional: bool,
    default_expr: ?ExprCode,
}

pub struct AssocItemSpec {
    name: Symbol,
    is_type: bool,
    item_type: ?Type,
    default_value: ?Any,
}

pub struct ItemHandle {
    full_path: List<Symbol>,
    span: SourceSpan,
}

-- 1. StructBuilder（扩展现有 StructBuilder：增加可见性、属性、子句、repr 与事务 Build）
pub struct StructBuilder {
    name: Symbol,
    visibility: Visibility,
    clauses: List<ClauseSpec>,
    attributes: List<AttributeSpec>,
    field_names: List<Symbol>,
    field_types: List<Type>,
    field_vis: List<Visibility>,
    repr: TypeRepr,
}

impl StructBuilder {
    pub pure comptime fn new(.name: Symbol = .Generated) -> Self;
    pub pure comptime fn from_type(subject: Type, .name: Symbol = .Generated) -> Self;
    pub pure comptime fn from_object(fields: Object, .name: Symbol = .Generated) -> ?Self;
    pub comptime fn set_visibility(self, vis: Visibility);
    pub comptime fn set_repr(self, repr: TypeRepr);
    pub comptime fn add_attribute(self, attr: AttributeSpec);
    pub comptime fn add_clause(self, clause: ClauseSpec);
    pub comptime fn add_field(self, name: Symbol, field_type: Type, .vis: Visibility = .public);
    pub comptime fn add_fields(self, names: List<Symbol>, types: List<Type>) -> bool;
    pub comptime fn insert_field(self, index: Integer, name: Symbol, field_type: Type) -> bool;
    pub comptime fn replace_field(self, index: Integer, name: Symbol, field_type: Type) -> bool;
    pub comptime fn remove_field(self, index: Integer) -> bool;
    pub comptime fn Build(self) -> !BuildErr Type;
}

-- 2. EnumBuilder（扩展现有 EnumBuilder：增加 pattern/expr/sub_enum 变体与可见性/属性）
pub struct EnumBuilder {
    name: Symbol,
    visibility: Visibility,
    clauses: List<ClauseSpec>,
    attributes: List<AttributeSpec>,
    groups: List<List<Symbol>>,
    variant_names: List<List<Symbol>>,
    shapes: List<Symbol>,
    field_names: List<List<Symbol>>,
    field_types: List<List<Type>>,
}

impl EnumBuilder {
    pub pure comptime fn new(.name: Symbol = .Generated) -> Self;
    pub pure comptime fn from_type(subject: Type, .name: Symbol = .Generated) -> Self;
    pub comptime fn set_visibility(self, vis: Visibility);
    pub comptime fn add_attribute(self, attr: AttributeSpec);
    pub comptime fn add_clause(self, clause: ClauseSpec);
    pub comptime fn add_group(self, name: Symbol, .parent: List<Symbol> = []);
    pub comptime fn add_unit(self, name: Symbol, .group: List<Symbol> = []);
    pub comptime fn add_tuple(self, name: Symbol, fields: List<Type>, .group: List<Symbol> = []);
    pub comptime fn add_named(self, name: Symbol, names: List<Symbol>, fields: List<Type>, .group: List<Symbol> = []);
    pub comptime fn add_named_from_object(self, name: Symbol, fields: Object, .group: List<Symbol> = []) -> bool;
    pub comptime fn add_pattern_variant(self, name: Symbol, pattern_code: Any, .group: List<Symbol> = []);
    pub comptime fn add_expr_variant(self, name: Symbol, value_expr: ExprCode, .group: List<Symbol> = []);
    pub comptime fn add_sub_enum(self, name: Symbol, sub_enum_type: Type, .group: List<Symbol> = []);
    pub comptime fn Build(self) -> !BuildErr Type;
    pub comptime fn BuildSymbol(self, codes: List<Symbol>) -> !BuildErr Type;
    pub comptime fn BuildBits(
        self,
        width: Integer,
        masks: List<Integer>,
        values: List<Integer>,
        offsets: List<List<Integer>>,
        .base_type: ?Type = null,
    ) -> !BuildErr Type;
}

-- 3. UnionBuilder（C ABI 联合体：Build 校验非空及 runtime 可表示性）
pub struct UnionBuilder {
    name: Symbol,
    visibility: Visibility,
    clauses: List<ClauseSpec>,
    attributes: List<AttributeSpec>,
    field_names: List<Symbol>,
    field_types: List<Type>,
    is_c_abi: bool,
}

impl UnionBuilder {
    pub pure comptime fn new(.name: Symbol = .Generated, .is_c_abi: bool = true) -> Self;
    pub comptime fn set_visibility(self, vis: Visibility);
    pub comptime fn add_attribute(self, attr: AttributeSpec);
    pub comptime fn add_clause(self, clause: ClauseSpec);
    pub comptime fn add_field(self, name: Symbol, field_type: Type);
    pub comptime fn remove_field(self, index: Integer) -> bool;
    pub comptime fn Build(self) -> !BuildErr Type;
}

-- 4. NewtypeBuilder（保持名义隔离，不继承底层 trait）
pub struct NewtypeBuilder {
    name: Symbol,
    visibility: Visibility,
    clauses: List<ClauseSpec>,
    attributes: List<AttributeSpec>,
    representation: Type,
    is_transparent_abi: bool,
}

impl NewtypeBuilder {
    pub pure comptime fn new(representation: Type, .name: Symbol = .Generated) -> Self;
    pub pure comptime fn from_type(subject: Type, .name: Symbol = .Generated) -> Self;
    pub comptime fn set_visibility(self, vis: Visibility);
    pub comptime fn set_transparent_abi(self, transparent: bool);
    pub comptime fn add_attribute(self, attr: AttributeSpec);
    pub comptime fn add_clause(self, clause: ClauseSpec);
    pub comptime fn set_representation(self, representation: Type);
    pub comptime fn Build(self) -> !BuildErr Type;
}

-- 5. TypeAliasBuilder
pub struct TypeAliasBuilder {
    name: Symbol,
    visibility: Visibility,
    clauses: List<ClauseSpec>,
    attributes: List<AttributeSpec>,
    target: ?Type,
}

impl TypeAliasBuilder {
    pub pure comptime fn new(name: Symbol) -> Self;
    pub comptime fn set_visibility(self, vis: Visibility);
    pub comptime fn add_attribute(self, attr: AttributeSpec);
    pub comptime fn add_clause(self, clause: ClauseSpec);
    pub comptime fn set_target(self, target: Type);
    pub comptime fn EvalType(self) -> !BuildErr Type;
    pub comptime fn publish(self) -> !BuildErr ItemHandle;
}

-- 6. FunctionBuilder
pub struct FunctionBuilder {
    name: Symbol,
    visibility: Visibility,
    stage: FunctionStage,
    is_inline: bool,
    is_unsafe: bool,
    abi: ?Symbol,
    clauses: List<ClauseSpec>,
    params: List<ParamSpec>,
    return_type: Type,
    error_row: ?Type,
    effect_row: ?Type,
    body_code: ?ExprCode,
    attributes: List<AttributeSpec>,
}

impl FunctionBuilder {
    pub pure comptime fn new(name: Symbol, return_type: Type) -> Self;
    pub comptime fn set_visibility(self, vis: Visibility);
    pub comptime fn set_stage(self, stage: FunctionStage);
    pub comptime fn set_inline(self, inline_flag: bool);
    pub comptime fn set_unsafe(self, unsafe_flag: bool);
    pub comptime fn set_abi(self, abi_name: ?Symbol);
    pub comptime fn add_attribute(self, attr: AttributeSpec);
    pub comptime fn add_clause(self, clause: ClauseSpec);
    pub comptime fn add_param(self, spec: ParamSpec);
    pub comptime fn set_error_row(self, err_row: ?Type);
    pub comptime fn set_effect_row(self, eff_row: ?Type);
    pub comptime fn set_body_code(self, body: ExprCode);
    pub comptime fn publish(self) -> !BuildErr ItemHandle;
}

-- 7. TraitBuilder
pub struct TraitBuilder {
    name: Symbol,
    visibility: Visibility,
    is_pure_comptime: bool,
    clauses: List<ClauseSpec>,
    supertraits: List<Type>,
    assoc_items: List<AssocItemSpec>,
    method_sigs: List<FunctionBuilder>,
    attributes: List<AttributeSpec>,
}

impl TraitBuilder {
    pub pure comptime fn new(name: Symbol, .is_pure_comptime: bool = false) -> Self;
    pub comptime fn set_visibility(self, vis: Visibility);
    pub comptime fn add_attribute(self, attr: AttributeSpec);
    pub comptime fn add_clause(self, clause: ClauseSpec);
    pub comptime fn add_supertrait(self, super_trait: Type);
    pub comptime fn add_assoc_item(self, spec: AssocItemSpec);
    pub comptime fn add_method(self, method: FunctionBuilder);
    pub comptime fn Build(self) -> !BuildErr Type;
}

-- 8. ImplBuilder（强制校验孤儿规则与六内置包对 builtin 类型的豁免边界）
pub struct ImplBuilder {
    target_type: Type,
    trait_type: ?Type,
    clauses: List<ClauseSpec>,
    assoc_bindings: List<Tuple<Symbol, Any>>,
    methods: List<FunctionBuilder>,
    attributes: List<AttributeSpec>,
}

impl ImplBuilder {
    pub pure comptime fn new_inherent(target: Type) -> Self;
    pub pure comptime fn new_trait(target: Type, trait_type: Type) -> Self;
    pub comptime fn add_attribute(self, attr: AttributeSpec);
    pub comptime fn add_clause(self, clause: ClauseSpec);
    pub comptime fn set_assoc_binding(self, name: Symbol, val_or_type: Any);
    pub comptime fn add_method(self, method: FunctionBuilder);
    pub comptime fn publish(self) -> !BuildErr ItemHandle;
}

-- 9. ModuleBuilder
pub struct ModuleBuilder {
    name: Symbol,
    visibility: Visibility,
    attributes: List<AttributeSpec>,
    items: List<ItemHandle>,
    submodules: List<ItemHandle>,
}

impl ModuleBuilder {
    pub pure comptime fn new(name: Symbol) -> Self;
    pub comptime fn set_visibility(self, vis: Visibility);
    pub comptime fn add_attribute(self, attr: AttributeSpec);
    pub comptime fn add_item(self, item: ItemHandle);
    pub comptime fn add_submodule(self, sub: ItemHandle);
    pub comptime fn publish(self) -> !BuildErr ItemHandle;
}
```

#### 3.4.2 事务不变量与拒绝例

1. **草稿孤立性**：构造过程中的 `StructBuilder`、`TraitBuilder` 或 `ImplBuilder` 在 `.Build()` / `.publish()` 成功返回前，**不向符号表或 trait 候选集注入任何部分符号**；返回 `error ...` 时内部事务全部回滚。
2. **禁止值循环与非法自引用**：无法生成未完成循环布局的直接按值递归定义，在 `Build()` 时报 `CycleByValue`（含自指针须经合法指针或自引用规则）。
3. **内置类型孤儿豁免边界**：非内置用户包使用 `ImplBuilder.new_trait(u64, ForeignTrait)` 必须报 `OrphanViolation`，仅六个内置包（`builtin`、`meta`、`build`、`attrs`、`std`、`verification`）享编译器 builtin 类型豁免权。

```flurry
-- [拒绝例 3.4-A] 重复字段与空 union 必须在 Build 时原子失败并回滚
comptime fn build_invalid_types() -> !BuildErr void {
    var sb = StructBuilder.new(.name = .DupStruct);
    sb.add_field(.id, u32);
    sb.add_field(.id, u64);
    let _ = sb.Build()!;  -- 返回 error DuplicateField，编译器内不存在 DupStruct 残留条目

    var ub = UnionBuilder.new(.name = .EmptyU);
    let _ = ub.Build()!;  -- 返回 error EmptyUnion
}

-- [拒绝例 3.4-B] comptime builder 严禁使用指针接收者或 .ref / .*
-- 以下在目标 Flurry 中完全非法，编译器应直接语法/语义报错：
-- pub comptime fn bad_mut(self: *StructBuilder) { self.*.add_field(.x, u8); }
```

---

### 3.5 `meta.data` 编译期容器与基础值类型（严格 `self` 值引用语义）

依据 [AGENTS.md](file:///home/fin/luna/AGENTS.md) 第 5 节与 [docs/comptime/index.md](file:///home/fin/luna/docs/comptime/index.md)：**Flurry 在 comptime 不存在指针类型、`.ref` 与 `.*` 语义**；所有可变方法必须通过 **`self` 值引用**完成更新，而既有的局部快照绑定（`let snapshot = list;`）继续保持独立副本语义。

#### 3.5.1 既有实现核查与偏差清单

对 `library/meta/data/` 逐文件核查结果显示：
1. **代码实现已采用值引用语义**：[list.fl](file:///home/fin/luna/library/meta/data/list.fl)、[object.fl](file:///home/fin/luna/library/meta/data/object.fl)、`map.fl`、`set.fl` 已全面采用 `pub comptime fn push/append/insert(self, ...)`，源码内部**零 `*self`、零 `*List<T>`、零 `.ref`、零 `.*`**。
2. **文档历史残留待清理**：[library/meta/README.md](file:///home/fin/luna/library/meta/README.md)（第 5、216 行）、`docs/comptime/index.md` 与 `docs/cmtt/patterns-and-arms.md` 第 220 行等仍残存 `*self` / `*List<T>` / `*bool` 过时描述，编译器与文档必须彻底封杀。
3. **`text.fl` 索引域不一致**：[library/meta/data/text.fl](file:///home/fin/luna/library/meta/data/text.fl) 当前使用目标相关整数 `u64` 作为长度与字节下标，而其余容器均统一使用任意精度 `Integer`，需收口到 `Integer`。
4. **`Float` 缺失**：`library/meta/data/float.fl` 尚不存在，编译期数学与坐标计算缺乏稳定接口。
5. **双阶段同名 trait**：`library/meta/data/{optional,errors}.fl` 与 `library/builtin/types/{option,result}.fl` 已分别提供 `ComptimeOptionalOps`/`OptionalOps` 与 `ComptimeErrorResultOps`/`ErrorResultOps` 且方法完全同名（**绝不加 `comptime_` 前缀**），本节补齐其完整契约。

#### 3.5.2 完整签名规范（零 `...` 省略）

```flurry
use builtin.{Any, Integer, Order, Symbol, Tuple, Type, bool, f64, str, u64, u8, void}

pub enum DataErr {
    IndexOutOfBounds,
    KeyNotFound,
    DuplicateKey,
    TypeMismatch,
    DomainOverflow,
    InvalidUtf8Slice,
    InvalidSymbolText,
    NaNOrInfinity,
    DivisionByZero,
}

-- 1. 双阶段对齐核心 trait（comptime 版均为 pure comptime trait，方法与 runtime trait 严格同名）
pub pure comptime trait ComptimeEq {
    pub pure comptime fn eq(self, other: Self) -> bool;
    pub pure comptime fn ne(self, other: Self) -> bool;
}

pub pure comptime trait ComptimeOrd :- ComptimeEq {
    pub pure comptime fn cmp(self, other: Self) -> Order;
    pub pure comptime fn lt(self, other: Self) -> bool;
    pub pure comptime fn le(self, other: Self) -> bool;
    pub pure comptime fn gt(self, other: Self) -> bool;
    pub pure comptime fn ge(self, other: Self) -> bool;
}

pub pure comptime trait ComptimeHash {
    pub pure comptime fn hash(self) -> u64;
}

-- 接收通用闭包 %c comptime fn 的组合子不标 pure，与 library/meta/data/optional.fl 对齐
pub pure comptime trait ComptimeOptionalOps
where
    T: Type
{
    pub pure comptime fn is_some(self) -> bool;
    pub pure comptime fn is_none(self) -> bool;
    pub pure comptime fn unwrap_or(self, default: T) -> T;
    pub comptime fn unwrap_or_else(self, fallback: %c comptime fn() -> T) -> T
    where
        c: Type.vars.Closure;
    pub comptime fn map(self, f: %c comptime fn(T) -> U) -> ?U
    where
        U: Type,
        c: Type.vars.Closure;
    pub comptime fn and_then(self, f: %c comptime fn(T) -> ?U) -> ?U
    where
        U: Type,
        c: Type.vars.Closure;
    pub comptime fn filter(self, predicate: %c comptime fn(T) -> bool) -> ?T
    where
        c: Type.vars.Closure;
    pub comptime fn inspect(self, visit: %c comptime fn(T)) -> ?T
    where
        c: Type.vars.Closure;
    pub comptime fn or_else(self, fallback: %c comptime fn() -> ?T) -> ?T
    where
        c: Type.vars.Closure;
    pub comptime fn zip_with(self, other: ?U, combine: %c comptime fn(T, U) -> R) -> ?R
    where
        U: Type,
        R: Type,
        c: Type.vars.Closure;
    pub pure comptime fn ok_or(self, err_val: E) -> !E T
    where
        E: Type;
}

pub pure comptime trait ComptimeErrorResultOps
where
    T: Type,
    E: Type
{
    pub pure comptime fn is_ok(self) -> bool;
    pub pure comptime fn is_err(self) -> bool;
    pub pure comptime fn unwrap_or(self, default: T) -> T;
    pub comptime fn unwrap_or_else(self, fallback: %c comptime fn(E) -> T) -> T
    where
        c: Type.vars.Closure;
    pub comptime fn map(self, f: %c comptime fn(T) -> U) -> !E U
    where
        U: Type,
        c: Type.vars.Closure;
    pub comptime fn map_err(self, f: %c comptime fn(E) -> F) -> !F T
    where
        F: Type,
        c: Type.vars.Closure;
    pub comptime fn and_then(self, f: %c comptime fn(T) -> !E U) -> !E U
    where
        U: Type,
        c: Type.vars.Closure;
    pub comptime fn or_else(self, fallback: %c comptime fn(E) -> !F T) -> !F T
    where
        F: Type,
        c: Type.vars.Closure;
    pub comptime fn inspect(self, visit: %c comptime fn(T)) -> !E T
    where
        c: Type.vars.Closure;
    pub comptime fn inspect_err(self, visit: %c comptime fn(E)) -> !E T
    where
        c: Type.vars.Closure;
    pub pure comptime fn ok(self) -> ?T;
    pub pure comptime fn err(self) -> ?E;
}

-- 2. List<T>（集中绑定：pub const List: pure comptime fn<T: Type> -> Type = .List'builtin）
impl List<T>
where
    T: Type
{
    pub pure comptime fn empty() -> List<T>;
    pub pure comptime fn length(self) -> Integer;
    pub pure comptime fn is_empty(self) -> bool;
    pub pure comptime fn get(self, index: Integer) -> ?T;
    pub pure comptime fn first(self) -> ?T;
    pub pure comptime fn last(self) -> ?T;
    pub pure comptime fn slice(self, start: Integer, end: Integer) -> ?List<T>;
    pub pure comptime fn concat(self, right: List<T>) -> List<T>;
    pub pure comptime fn reverse(self) -> List<T>;
    pub comptime fn map(self, transform: %c comptime fn(T) -> U) -> List<U>
    where
        U: Type,
        c: Type.vars.Closure;
    pub comptime fn filter(self, predicate: %c comptime fn(T) -> bool) -> List<T>
    where
        c: Type.vars.Closure;
    pub comptime fn flat_map(self, transform: %c comptime fn(T) -> List<U>) -> List<U>
    where
        U: Type,
        c: Type.vars.Closure;
    pub comptime fn fold(self, initial: A, combine: %c comptime fn(A, T) -> A) -> A
    where
        A: Type,
        c: Type.vars.Closure;
    pub comptime fn any(self, predicate: %c comptime fn(T) -> bool) -> bool
    where
        c: Type.vars.Closure;
    pub comptime fn all(self, predicate: %c comptime fn(T) -> bool) -> bool
    where
        c: Type.vars.Closure;
    pub pure comptime fn zip(self, other: List<U>) -> List<Tuple<T, U>>
    where
        U: Type;
    pub pure comptime fn enumerate(self) -> List<Tuple<Integer, T>>;
    pub pure comptime fn contains(self, target: T) -> bool
    where
        T :- ComptimeEq;
    pub pure comptime fn index_of(self, target: T) -> ?Integer
    where
        T :- ComptimeEq;
    pub pure comptime fn sort(self) -> List<T>
    where
        T :- ComptimeOrd;

    -- 值引用可变操作（修改调用点槽位，不影响既有不可变快照）
    pub comptime fn append(self, value: T);
    pub comptime fn append_all(self, values: List<T>);
    pub comptime fn prepend(self, value: T);
    pub comptime fn insert(self, index: Integer, value: T) -> bool;
    pub comptime fn replace(self, index: Integer, value: T) -> bool;
    pub comptime fn replace_range(self, start: Integer, end: Integer, values: List<T>) -> bool;
    pub comptime fn remove(self, index: Integer) -> ?T;
    pub comptime fn pop(self) -> ?T;
    pub comptime fn clear(self);
    pub comptime fn reverse_in_place(self);
    pub comptime fn sort_in_place(self)
    where
        T :- ComptimeOrd;
    pub comptime fn retain(self, predicate: %c comptime fn(T) -> bool)
    where
        c: Type.vars.Closure;
    pub comptime fn foreach(self, visit: %c comptime fn(T))
    where
        c: Type.vars.Closure;
    pub comptime fn try_foreach(self, visit: %c comptime fn(T) -> !E void) -> !E void
    where
        E: Type,
        c: Type.vars.Closure;
    pub comptime fn try_map(self, transform: %c comptime fn(T) -> !E U) -> !E List<U>
    where
        U: Type,
        E: Type,
        c: Type.vars.Closure;
    pub comptime fn try_fold(self, initial: A, combine: %c comptime fn(A, T) -> !E A) -> !E A
    where
        A: Type,
        E: Type,
        c: Type.vars.Closure;
    pub comptime fn try_flat_map(self, transform: %c comptime fn(T) -> !E List<U>) -> !E List<U>
    where
        U: Type,
        E: Type,
        c: Type.vars.Closure;
}

-- 3. Map<K, V>（确定性插入序关联容器）
pub struct MapEntry
where
    K: Type,
    V: Type
{
    key: K,
    value: V,
}

pub struct Map
where
    K: Type,
    V: Type
{
    entries: List<MapEntry<K, V>>,
}

impl Map<K, V>
where
    K :- ComptimeEq,
    V: Type
{
    pub pure comptime fn empty() -> Self;
    pub pure comptime fn length(self) -> Integer;
    pub pure comptime fn is_empty(self) -> bool;
    pub pure comptime fn contains_key(self, key: K) -> bool;
    pub pure comptime fn get(self, key: K) -> ?V;
    pub pure comptime fn keys(self) -> List<K>;
    pub pure comptime fn values(self) -> List<V>;
    pub pure comptime fn entries(self) -> List<MapEntry<K, V>>;
    pub comptime fn insert(self, key: K, value: V) -> ?V;
    pub comptime fn remove(self, key: K) -> ?V;
    pub comptime fn get_or_insert(self, key: K, default: V) -> V;
    pub comptime fn extend(self, other: Self);
    pub comptime fn clear(self);
    pub comptime fn foreach(self, visit: %c comptime fn(K, V))
    where
        c: Type.vars.Closure;
    pub comptime fn try_foreach(self, visit: %c comptime fn(K, V) -> !E void) -> !E void
    where
        E: Type,
        c: Type.vars.Closure;
}

-- 4. Set<T>（确定性唯一集合）
pub struct Set
where
    T: Type
{
    items: List<T>,
}

impl Set<T>
where
    T :- ComptimeEq
{
    pub pure comptime fn empty() -> Self;
    pub pure comptime fn from_list(values: List<T>) -> Self;
    pub pure comptime fn length(self) -> Integer;
    pub pure comptime fn is_empty(self) -> bool;
    pub pure comptime fn contains(self, value: T) -> bool;
    pub pure comptime fn to_list(self) -> List<T>;
    pub pure comptime fn union(self, other: Self) -> Self;
    pub pure comptime fn intersection(self, other: Self) -> Self;
    pub pure comptime fn difference(self, other: Self) -> Self;
    pub pure comptime fn symmetric_difference(self, other: Self) -> Self;
    pub pure comptime fn is_subset_of(self, other: Self) -> bool;
    pub pure comptime fn is_disjoint(self, other: Self) -> bool;
    pub comptime fn insert(self, value: T) -> bool;
    pub comptime fn remove(self, value: T) -> bool;
    pub comptime fn extend(self, other: Self);
    pub comptime fn clear(self);
    pub comptime fn foreach(self, visit: %c comptime fn(T))
    where
        c: Type.vars.Closure;
}

-- 5. Object（保留异构槽、可选 Symbol 键、重复键与声明序的编译期记录列表，不擦除到 Any）
impl Object {
    pub pure comptime fn empty() -> Object;
    pub pure comptime fn length(self) -> Integer;
    pub pure comptime fn is_empty(self) -> bool;
    pub pure comptime fn key_at(self, index: Integer) -> ?Symbol;
    pub pure comptime fn has_key(self, key: Symbol) -> bool;
    pub pure comptime fn EntryType(self, index: Integer) -> !DataErr Type;
    pub pure comptime fn value_at(self, index: Integer) -> ?T
    where
        T: Type;
    pub pure comptime fn get(self, key: Symbol) -> ?T
    where
        T: Type;
    pub pure comptime fn slice(self, start: Integer, end: Integer) -> Object;
    pub pure comptime fn concat(self, other: Object) -> Object;
    pub pure comptime fn append(self, value: Any, .key: ?Symbol = null) -> Object;
    pub pure comptime fn replace(self, index: Integer, value: Any) -> Object;
    pub pure comptime fn remove(self, index: Integer) -> Object;
    pub comptime fn push(self, value: Any, .key: ?Symbol = null);
    pub comptime fn extend(self, other: Object);
    pub comptime fn set_at(self, index: Integer, value: Any) -> bool;
    pub comptime fn remove_at(self, index: Integer) -> bool;
    pub comptime fn clear(self);
    pub comptime fn foreach(self, visit: %c comptime fn(Object))
    where
        c: Type.vars.Closure;
    pub comptime fn fold(self, initial: A, combine: %c comptime fn(A, Object) -> A) -> A
    where
        A: Type,
        c: Type.vars.Closure;
    pub comptime fn try_foreach(self, visit: %c comptime fn(Object) -> !E void) -> !E void
    where
        E: Type,
        c: Type.vars.Closure;
    pub comptime fn try_fold(self, initial: A, combine: %c comptime fn(A, Object) -> !E A) -> !E A
    where
        A: Type,
        E: Type,
        c: Type.vars.Closure;
    pub comptime fn try_flat_map(self, transform: %c comptime fn(Object) -> !E Object) -> !E Object
    where
        E: Type,
        c: Type.vars.Closure;
}

-- 6. Integer（编译期任意精度数学整数）与 Float（确定性编译期浮点）
impl Integer {
    pub pure comptime fn abs(self) -> Integer;
    pub pure comptime fn div_trunc(self, rhs: Integer) -> !DataErr Integer;
    pub pure comptime fn rem_trunc(self, rhs: Integer) -> !DataErr Integer;
    pub pure comptime fn div_euclid(self, rhs: Integer) -> !DataErr Integer;
    pub pure comptime fn rem_euclid(self, rhs: Integer) -> !DataErr Integer;
    pub pure comptime fn pow(self, exp: Integer) -> !DataErr Integer;
    pub pure comptime fn gcd(self, other: Integer) -> Integer;
    pub pure comptime fn min(self, other: Integer) -> Integer;
    pub pure comptime fn max(self, other: Integer) -> Integer;
    pub pure comptime fn bit_length(self) -> Integer;
    pub pure comptime fn to_str(self) -> str;
    pub pure comptime fn try_into_fixed(self) -> !DataErr T
    where
        T: Type;
}

pub struct Float {
    bits_f64: f64,
}

impl Float {
    pub pure comptime fn from_f64(value: f64) -> Float;
    pub pure comptime fn from_integer(value: Integer) -> !DataErr Float;
    pub pure comptime fn to_f64(self) -> f64;
    pub pure comptime fn is_nan(self) -> bool;
    pub pure comptime fn is_infinite(self) -> bool;
    pub pure comptime fn is_finite(self) -> bool;
    pub pure comptime fn abs(self) -> Float;
    pub pure comptime fn floor(self) -> !DataErr Float;
    pub pure comptime fn ceil(self) -> !DataErr Float;
    pub pure comptime fn round(self) -> !DataErr Float;
    pub pure comptime fn trunc(self) -> !DataErr Float;
    pub pure comptime fn add(self, other: Float) -> Float;
    pub pure comptime fn sub(self, other: Float) -> Float;
    pub pure comptime fn mul(self, other: Float) -> Float;
    pub pure comptime fn div(self, other: Float) -> Float;
    pub pure comptime fn try_to_integer_trunc(self) -> !DataErr Integer;
}

-- 7. Symbol 与 ComptimeTextBytes（与 text.fl / TextBytes 保持方法同名且收敛为 Integer 索引）
impl Symbol {
    pub pure comptime fn byte_length(self) -> Integer;
    pub pure comptime fn byte(self, index: Integer) -> ?u8;
    pub pure comptime fn as_str(self) -> str;
    pub pure comptime fn from_str(text: str) -> !DataErr Symbol;
    pub pure comptime fn concat(self, suffix: Symbol) -> !DataErr Symbol;
}

pub pure comptime trait ComptimeTextBytes {
    pub pure comptime fn byte_length(self) -> Integer;
    pub pure comptime fn is_empty(self) -> bool;
    pub pure comptime fn byte(self, index: Integer) -> ?u8;
    pub pure comptime fn slice(self, start: Integer, end: Integer) -> !DataErr str;
    pub pure comptime fn is_ascii(self) -> bool;
    pub pure comptime fn is_char_boundary(self, index: Integer) -> bool;
    pub pure comptime fn find(self, needle: str) -> ?Integer;
    pub pure comptime fn concat(self, other: str) -> str;
    pub pure comptime fn starts_with(self, prefix: str) -> bool;
    pub pure comptime fn ends_with(self, suffix: str) -> bool;
    pub pure comptime fn contains(self, needle: str) -> bool;
    pub pure comptime fn split(self, delimiter: str) -> List<str>;
    pub pure comptime fn trim(self) -> str;
    pub pure comptime fn parse_integer(self) -> !DataErr Integer;
    pub pure comptime fn to_symbol(self) -> !DataErr Symbol;
}
```

#### 3.5.3 值引用快照隔离与拒绝例

```flurry
-- [正例] 值引用 mutator 更新本地绑定槽，不影响先前的快照
comptime fn snapshot_isolation() -> bool {
    var xs: List<Integer> = [1, 2];
    let snap = xs;       -- 值快照固定为 [1, 2]
    xs.append(3);        -- xs 更新为 [1, 2, 3]，snap 保持 [1, 2]
    xs.length() == 3 and snap.length() == 2
}

-- [拒绝例 3.5-A] comptime 严禁指针接收者或 .ref / .* 模拟可变容器
-- comptime fn bad_append(xs: *List<Integer>, v: Integer) { xs.*.append(v); }  -- 拒绝：comptime 无指针

-- [拒绝例 3.5-B] 禁止使用 comptime_ 前缀区分双阶段方法
-- pub pure comptime trait BadOps { pub pure comptime fn comptime_unwrap_or(self, d: T) -> T }  -- 违反双阶段同名约定
```

---

### 3.6 `meta.code`（CMTT 分阶段代码组装与证据系统）

依据 [docs/cmtt/index.md](file:///home/fin/luna/docs/cmtt/index.md)、[contexts-and-substitution.md](file:///home/fin/luna/docs/cmtt/contexts-and-substitution.md)、[patterns-and-arms.md](file:///home/fin/luna/docs/cmtt/patterns-and-arms.md)、[any-and-code-evidence.md](file:///home/fin/luna/docs/cmtt/any-and-code-evidence.md)、[program-construction.md](file:///home/fin/luna/docs/cmtt/program-construction.md) 与 [quote-functions-and-templates.md](file:///home/fin/luna/docs/cmtt/quote-functions-and-templates.md)，`meta.code` 将 Flurry 的**两阶段（`comptime` / `runtime`）、rank-1 有序上下文 `< Ψ |- T >` / `< T -| Ψ >` 与每段 quote 至多一个 splice（单洞不变量）**暴露为安全可组合的编译期标准库接口。

#### 3.6.1 完整签名与核心契约

```flurry
use builtin.{Any, Integer, Symbol, Tuple, Type, bool, u64, void}
use meta.ast.SourceSpan
use meta.data.List

pub enum CodeErr {
    InvalidContext,
    MissingBinding,
    DuplicateBinding,
    TypeMismatch,
    RankViolation,
    MultipleSplicesInQuote,
    ControlFlowEscape,
    StageBoundaryViolation,
    EvidenceUnavailable,
    LinearResourceDoubleUse,
    NonExhaustiveMatch,
}

-- 1. Rank-1 有序上下文 Ψ 与显式/默认替换
pub struct ContextEntry {
    name: Symbol,
    binding_type: Type,
    is_mutable: bool,
    is_linear: bool,
    identity_token: u64,
}

impl ContextEntry {
    pub pure comptime fn BindingType(self) -> Type {
        self.binding_type
    }
}

pub struct Context {
    entries: List<ContextEntry>,
}

impl Context {
    pub pure comptime fn empty() -> Context;
    pub pure comptime fn from_entries(entries: List<ContextEntry>) -> !CodeErr Context;
    pub pure comptime fn length(self) -> Integer;
    pub pure comptime fn is_empty(self) -> bool;
    pub pure comptime fn contains(self, name: Symbol) -> bool;
    pub pure comptime fn EntryType(self, name: Symbol) -> ?Type;
    pub pure comptime fn is_subcontext_of(self, target: Context) -> bool;
    pub pure comptime fn concat(self, extension: Context) -> !CodeErr Context;
    pub comptime fn push(
        self,
        name: Symbol,
        binding_type: Type,
        .is_mutable: bool = false,
        .is_linear: bool = true,
    ) -> !CodeErr void;
    pub comptime fn drop_entry(self, name: Symbol) -> !CodeErr void;
}

pub struct Substitution {
    mappings: List<Tuple<Symbol, Symbol>>,  -- (interface_name, local_binding_name)，并行映射不按顺序覆写
}

impl Substitution {
    pub pure comptime fn empty() -> Substitution;
    pub pure comptime fn from_pairs(pairs: List<Tuple<Symbol, Symbol>>) -> !CodeErr Substitution;
    pub comptime fn bind(self, interface_name: Symbol, target_name: Symbol) -> !CodeErr void;
}

-- 2. 类型化代码句柄与代码证据句柄（保留绑定身份与控制流目标，绝不退化为未检查文本）
pub struct CodeBinding
where
    T: Type
{
    name: Symbol,
    binding_token: u64,
    span: SourceSpan,
}

pub struct ExprCode {
    ctx: Context,
    result_type: Type,
    span: SourceSpan,
}

impl ExprCode {
    pub pure comptime fn from_quoted(code: Any) -> !CodeErr ExprCode;
    pub pure comptime fn context(self) -> Context;
    pub pure comptime fn ResultType(self) -> Type;
    pub pure comptime fn is_closed(self) -> bool;
    pub pure comptime fn span(self) -> SourceSpan;
    -- 上下文弱化：向未使用的新端口安全扩展，不允许缩减被主体实际引用的自由绑定
    pub pure comptime fn weaken(self, superset: Context) -> !CodeErr ExprCode;
    -- 同时更新接口、主体、结果类型及非自由控制边界
    pub pure comptime fn rebind(self, subst: Substitution) -> !CodeErr ExprCode;
    -- 转回具名开放代码引用（要求接入时能恢复相同身份证明）
    pub pure comptime fn into_quoted_any(self) -> Any;
}

pub struct PatternCode {
    subject_type: Type,
    exports: Context,
    span: SourceSpan,
}

impl PatternCode {
    pub pure comptime fn from_quoted(pattern_code: Any) -> !CodeErr PatternCode;
    pub pure comptime fn SubjectType(self) -> Type;
    pub pure comptime fn exports(self) -> Context;
    pub pure comptime fn span(self) -> SourceSpan;
    pub pure comptime fn rename_exports(self, subst: Substitution) -> !CodeErr PatternCode;
    pub pure comptime fn into_quoted_any(self) -> Any;
}

pub struct ArmCode {
    pattern: PatternCode,
    guard: ?ExprCode,
    body: ExprCode,
    span: SourceSpan,
}

impl ArmCode {
    pub pure comptime fn new(pattern: PatternCode, guard: ?ExprCode, body: ExprCode) -> !CodeErr ArmCode;
    pub pure comptime fn SubjectType(self) -> Type;
    pub pure comptime fn ResultType(self) -> Type;
    pub pure comptime fn outer_context(self) -> Context;
    pub pure comptime fn span(self) -> SourceSpan;
}

-- 3. Type.FromCode 与 Any/any 运行时证据恢复（遵循 docs/cmtt/any-and-code-evidence.md）
pub enum CodeWitness {
    expr_witness(ExprCode),
    pattern_witness(PatternCode),
    arm_witness(ArmCode),
}

impl Type {
    -- 提取代码值的真实结果类型或模式主语类型；普通 any 无实际代码主体时拒绝 EvidenceUnavailable
    pub pure comptime fn FromCode(code_val: Any) -> !CodeErr Type;
}

pub pure comptime fn inspect_code_evidence(value: Any) -> !CodeErr CodeWitness;
pub pure comptime fn assert_expr_code(value: Any, expected_ctx: Context, expected_type: Type) -> !CodeErr ExprCode;
pub pure comptime fn assert_closed_expr(value: Any, expected_type: Type) -> !CodeErr ExprCode;

-- 4. 受检多分支程序构造组合子（绕开单 quote 单洞限制而不破坏线性资源与类型检查）
pub pure comptime fn code_sequence(stmts: List<ExprCode>, .tail: ?ExprCode = null) -> !CodeErr ExprCode;
pub pure comptime fn code_let(
    binding_name: Symbol,
    value_expr: ExprCode,
    body_expr: ExprCode,
    .is_var: bool = false,
) -> !CodeErr ExprCode;
pub pure comptime fn code_if(
    cond: ExprCode,
    then_branch: ExprCode,
    else_branch: ?ExprCode = null,
) -> !CodeErr ExprCode;
pub pure comptime fn code_match(subject: ExprCode, arms: List<ArmCode>) -> !CodeErr ExprCode;
pub pure comptime fn code_loop(init_ctx: Context, cond_or_iter: ExprCode, body: ExprCode, .is_while: bool = true) -> !CodeErr ExprCode;
```

#### 3.6.2 `quote fn`、`template` 与 Staged Comptime Closure 不变量

1. **`quote fn` 非 `comptime fn`**：依据 [AGENTS.md](file:///home/fin/luna/AGENTS.md) 第 5 节与 [quote-functions-and-templates.md](file:///home/fin/luna/docs/cmtt/quote-functions-and-templates.md)，`quote fn` 属于运行期语义范畴，普通实参只在进入函数时**求值一次**；其隐式 quote 绑定由编译器和 `^attrs.quote_fn_template.{for_match_arms, allow_non_exhaustive_instance}`（以及开放匹配 `use` 默认端口）驱动。
2. **Staged 闭包常量规范化（关联 [issues/0003](file:///home/fin/luna/issues/0003-builtin-packages-compiler-blockers.md) `iir/src/staging.rs:3173`）**：带有 comptime-captured 闭包与代码参数的调用必须在 staging 折叠期间产生规范常量身份，不得把高阶 runtime/comptime 混杂闭包逃逸成未规约残余。
3. **资源与控制流隔离**：在同一 `quote` 内通过循环构造多步程序（单洞链）时，每一步生成的 `ExprCode` 必须独立经过捕获与线形资源检查；`code_match` 的 `ArmCode` 保证 guard 中的 `return`/`break`/`continue` 指向正确作用域，且前一支失败时绝不泄露未初始化的模式绑定。

#### 3.6.3 拒绝例

```flurry
-- [拒绝例 3.6-A] 单个 quote 中禁止出现两个独立 splice（违反单洞约束）
pure comptime fn illegal_double_hole(a: < |- i32 >, b: < |- i32 >) -> < |- i32 > {
    '(~<a> + ~<b>)  -- 拒绝：MultipleSplicesInQuote（必须改用 code_let 或链式单洞构造）
}

-- [拒绝例 3.6-B] 禁止借助 Any 擦除上下文或偷换结果类型
fn illegal_any_coerce() -> i32 {
    const q: Any = '(true);  -- 真实证据为 < |- bool >
    ~<q>                     -- 拒绝：TypeMismatch（Any 不把 bool 改造成 i32）
}

-- [拒绝例 3.6-C] 禁止跨构造边界捕获外层循环的 break/return 目标
pure comptime fn escaped_break() -> < |- void > {
    '(break)  -- 拒绝：ControlFlowEscape（无所属内部 loop 边界）
}
```

---

### 3.7 `meta.ast`、`meta.macros` 与宏系统（4 种构造宏 + `define` 模板宏 + 内置宏）

#### 3.7.1 `meta.ast` 完整结构化 AST、`TokenBuffer`、`ToTokens` 与 `Parse`

针对当前 [library/meta/ast_impl/nodes.fl](file:///home/fin/luna/library/meta/ast_impl/nodes.fl) 仅提供不透明句柄 `{ tree: SyntaxTree, node: Integer }` 的局限，`meta.ast` 保留 `SourceSpan` / `TokenBuffer` 的精确词法回溯能力，同时补全强类型结构化 AST 节点与双向转换 trait：

```flurry
use builtin.{Integer, Symbol, Tuple, Type, bool, f64, str, void}
use meta.data.List

pub struct SourceSpan {
    file: str,
    start: Integer,
    end: Integer,
    expansion_id: Integer,
    origin_parent: ?SourceSpan,
}

impl SourceSpan {
    pub pure comptime fn synthetic() -> SourceSpan;
    pub pure comptime fn new(file: str, start: Integer, end: Integer, expansion_id: Integer) -> SourceSpan;
    pub pure comptime fn with_origin(self, parent: SourceSpan) -> SourceSpan;
    pub pure comptime fn is_synthetic(self) -> bool;
    pub pure comptime fn merge(self, other: SourceSpan) -> SourceSpan;
}

pub enum TokenSpacing {
    alone,
    joint,
    whitespace,
}

pub enum TokenKind {
    ident,
    keyword,
    literal,
    punct,
    delimiter,
    comment,
    eof,
    error_token,
}

pub struct Token {
    kind: TokenKind,
    text: str,
    spacing: TokenSpacing,
    span: SourceSpan,
    hygiene_id: Integer,
}

impl Token {
    pub pure comptime fn new(kind: TokenKind, text: str, spacing: TokenSpacing, span: SourceSpan) -> Token;
    pub pure comptime fn ident(text: str, .span: SourceSpan = SourceSpan.synthetic()) -> Token;
    pub pure comptime fn string_literal(text: str, .span: SourceSpan = SourceSpan.synthetic()) -> Token;
    pub pure comptime fn int_literal(text: str, .span: SourceSpan = SourceSpan.synthetic()) -> Token;
    pub pure comptime fn bool_literal(value: bool, .span: SourceSpan = SourceSpan.synthetic()) -> Token;
    pub pure comptime fn punct(text: str, spacing: TokenSpacing, .span: SourceSpan = SourceSpan.synthetic()) -> Token;
    pub pure comptime fn with_spacing(self, spacing: TokenSpacing) -> Token;
    pub pure comptime fn is_ident(self) -> bool;
    pub pure comptime fn is_literal(self) -> bool;
}

pub struct ParseErr {
    message: str,
    span: SourceSpan,
    expected: List<str>,
}

pub struct TokenBuffer {
    tokens: List<Token>,
}

impl TokenBuffer {
    pub pure comptime fn empty() -> TokenBuffer;
    pub pure comptime fn from_token(token: Token) -> TokenBuffer;
    pub pure comptime fn from_tokens(tokens: List<Token>) -> TokenBuffer;
    pub pure comptime fn from_str_literal(text: str, .span: SourceSpan = SourceSpan.synthetic()) -> TokenBuffer;
    pub pure comptime fn length(self) -> Integer;
    pub pure comptime fn is_empty(self) -> bool;
    pub pure comptime fn get(self, index: Integer) -> ?Token;
    pub pure comptime fn first(self) -> ?Token;
    pub pure comptime fn last(self) -> ?Token;
    pub pure comptime fn to_list(self) -> List<Token>;
    pub comptime fn push(self, token: Token);
    pub comptime fn append(self, other: TokenBuffer);
    pub pure comptime fn concat(self, other: TokenBuffer) -> TokenBuffer;
    pub pure comptime fn concat_all(buffers: List<TokenBuffer>) -> TokenBuffer;
    pub pure comptime fn with_trailing_spacing(self, spacing: TokenSpacing) -> TokenBuffer;
    pub pure comptime fn join(buffers: List<TokenBuffer>, separator: Token) -> TokenBuffer;
    pub pure comptime fn parse_as(self) -> !ParseErr T
    where
        T :- Parse;
    pub comptime fn foreach(self, visit: %c comptime fn(Token))
    where
        c: Type.vars.Closure;
    pub comptime fn map(self, transform: %c comptime fn(Token) -> Token) -> TokenBuffer
    where
        c: Type.vars.Closure;
    pub comptime fn filter(self, predicate: %c comptime fn(Token) -> bool) -> TokenBuffer
    where
        c: Type.vars.Closure;
    pub comptime fn fold(self, initial: A, combine: %c comptime fn(A, Token) -> A) -> A
    where
        A: Type,
        c: Type.vars.Closure;
}

-- 双向语法转换 trait（仅在 pure comptime 执行）
pub pure comptime trait ToTokens {
    pub pure comptime fn into_tokens(self) -> TokenBuffer;
    pub pure comptime fn into_tokens_with_spacing(self, trailing: TokenSpacing) -> TokenBuffer;
}

pub pure comptime trait Parse {
    pub pure comptime fn parse(input: TokenBuffer) -> !ParseErr Tuple<Self, TokenBuffer>;
}

-- 完整结构化 Flurry AST 节点核心结构（保留 SyntaxTree arena 兼容且无 raw_tokens 退化占位）
pub struct FlurryIdent {
    name: Symbol,
    hygiene_id: Integer,
    span: SourceSpan,
}

pub struct FlurryPath {
    segments: List<FlurryIdent>,
    leading_dot: bool,
    span: SourceSpan,
}

pub enum LiteralKind {
    int_lit(Integer),
    float_lit(f64),
    str_lit(str),
    char_lit(str),
    bool_lit(bool),
    symbol_lit(Symbol),
}

pub struct FlurryLiteral {
    kind: LiteralKind,
    raw_text: str,
    span: SourceSpan,
}

pub struct FlurryAttr {
    path: FlurryPath,
    positional_args: List<FlurryExpr>,
    named_args: List<Tuple<FlurryIdent, FlurryExpr>>,
    span: SourceSpan,
}

pub enum FlurryClause {
    type_param {
        name: FlurryIdent,
        bound: ?FlurryType,
        default_ty: ?FlurryType,
        span: SourceSpan,
    },
    value_param {
        name: FlurryIdent,
        val_type: FlurryType,
        default_val: ?FlurryExpr,
        span: SourceSpan,
    },
    trait_bound {
        subject: FlurryType,
        traits: List<FlurryType>,
        span: SourceSpan,
    },
    requires_clause {
        predicate: FlurryExpr,
        span: SourceSpan,
    },
    ensures_clause {
        predicate: FlurryExpr,
        span: SourceSpan,
    },
    quote_lift_clause {
        name: FlurryIdent,
        val_type: FlurryType,
        default_val: ?FlurryExpr,
        span: SourceSpan,
    },
}

pub enum FlurryType {
    path_type {
        path: FlurryPath,
        args: List<FlurryExpr>,
        span: SourceSpan,
    },
    tuple_type {
        elems: List<FlurryType>,
        span: SourceSpan,
    },
    anonymous_record_type {
        fields: List<Tuple<FlurryIdent, FlurryType>>,
        is_open: bool,
        span: SourceSpan,
    },
    pointer_type {
        pointee: FlurryType,
        span: SourceSpan,
    },
    optional_type {
        payload: FlurryType,
        span: SourceSpan,
    },
    error_qualified_type {
        err_row: List<FlurryType>,
        ok_type: FlurryType,
        span: SourceSpan,
    },
    effect_qualified_type {
        eff_row: List<FlurryType>,
        ok_type: FlurryType,
        span: SourceSpan,
    },
    reachability_qualified_type {
        reach_set: FlurryExpr,
        inner: FlurryType,
        span: SourceSpan,
    },
    for_type {
        binders: List<FlurryIdent>,
        body: FlurryType,
        span: SourceSpan,
    },
    refinement_type {
        binder: FlurryIdent,
        base: FlurryType,
        predicate: FlurryExpr,
        span: SourceSpan,
    },
    lift_type {
        inner: FlurryType,
        span: SourceSpan,
    },
    fn_type {
        closure_var: ?FlurryIdent,
        is_comptime: bool,
        is_pure: bool,
        params: List<FlurryType>,
        ret: ?FlurryType,
        span: SourceSpan,
    },
    contextual_expr_type {
        bindings: List<Tuple<FlurryIdent, FlurryType>>,
        result_type: FlurryType,
        span: SourceSpan,
    },
    contextual_pattern_type {
        subject_type: FlurryType,
        exports: List<Tuple<FlurryIdent, FlurryType>>,
        span: SourceSpan,
    },
    never_type(SourceSpan),
    void_type(SourceSpan),
    self_type(SourceSpan),
}

pub enum FlurryPattern {
    wildcard(SourceSpan),
    ident {
        name: FlurryIdent,
        is_mutable: bool,
        span: SourceSpan,
    },
    literal(FlurryLiteral),
    tuple {
        elems: List<FlurryPattern>,
        span: SourceSpan,
    },
    struct_pat {
        path: ?FlurryPath,
        fields: List<Tuple<FlurryIdent, FlurryPattern>>,
        has_rest: bool,
        span: SourceSpan,
    },
    enum_pat {
        path: FlurryPath,
        payload: List<FlurryPattern>,
        span: SourceSpan,
    },
    optional_some {
        inner: FlurryPattern,
        span: SourceSpan,
    },
    optional_null(SourceSpan),
    error_ok {
        inner: FlurryPattern,
        span: SourceSpan,
    },
    error_err {
        inner: FlurryPattern,
        span: SourceSpan,
    },
    or_pat {
        alts: List<FlurryPattern>,
        span: SourceSpan,
    },
    splice_pat {
        subst: List<Tuple<FlurryIdent, FlurryIdent>>,
        inner_expr: FlurryExpr,
        span: SourceSpan,
    },
}

pub struct FlurryArm {
    pattern: FlurryPattern,
    guard: ?FlurryExpr,
    body: FlurryExpr,
    span: SourceSpan,
}

pub enum FlurryExpr {
    literal(FlurryLiteral),
    ident(FlurryIdent),
    path(FlurryPath),
    tuple {
        elems: List<FlurryExpr>,
        span: SourceSpan,
    },
    record {
        fields: List<Tuple<FlurryIdent, FlurryExpr>>,
        span: SourceSpan,
    },
    object_lit {
        entries: List<Tuple<?FlurryIdent, FlurryExpr>>,
        span: SourceSpan,
    },
    list_lit {
        elems: List<FlurryExpr>,
        span: SourceSpan,
    },
    range_expr {
        start: ?FlurryExpr,
        end: ?FlurryExpr,
        inclusive: bool,
        span: SourceSpan,
    },
    block(FlurryBlock),
    unary {
        op: Symbol,
        operand: FlurryExpr,
        span: SourceSpan,
    },
    binary {
        op: Symbol,
        left: FlurryExpr,
        right: FlurryExpr,
        span: SourceSpan,
    },
    call {
        callee: FlurryExpr,
        args: List<Tuple<?FlurryIdent, FlurryExpr>>,
        span: SourceSpan,
    },
    extended_application {
        callee: FlurryExpr,
        args: List<Tuple<?FlurryIdent, FlurryExpr>>,
        trailing_block: FlurryBlock,
        span: SourceSpan,
    },
    method_call {
        receiver: FlurryExpr,
        method: FlurryIdent,
        args: List<Tuple<?FlurryIdent, FlurryExpr>>,
        span: SourceSpan,
    },
    field_access {
        receiver: FlurryExpr,
        field: FlurryIdent,
        span: SourceSpan,
    },
    index_expr {
        receiver: FlurryExpr,
        indices: List<FlurryExpr>,
        span: SourceSpan,
    },
    take_view {
        receiver: FlurryExpr,
        view_name: FlurryIdent,
        args: List<FlurryExpr>,
        span: SourceSpan,
    },
    ref_view {
        receiver: FlurryExpr,
        span: SourceSpan,
    },
    deref_view {
        receiver: FlurryExpr,
        span: SourceSpan,
    },
    as_cast {
        receiver: FlurryExpr,
        target_type: FlurryType,
        span: SourceSpan,
    },
    dyn_trait {
        receiver: FlurryExpr,
        trait_type: FlurryType,
        span: SourceSpan,
    },
    optional_unwrap {
        operand: FlurryExpr,
        span: SourceSpan,
    },
    error_propagate {
        operand: FlurryExpr,
        span: SourceSpan,
    },
    if_expr {
        is_inline: bool,
        cond: FlurryExpr,
        then_block: FlurryBlock,
        else_expr: ?FlurryExpr,
        span: SourceSpan,
    },
    match_expr {
        is_inline: bool,
        subject: FlurryExpr,
        arms: List<FlurryArm>,
        span: SourceSpan,
    },
    case_expr {
        subject: ?FlurryExpr,
        arms: List<FlurryArm>,
        span: SourceSpan,
    },
    use_expr {
        bindings: List<FlurryExpr>,
        body: FlurryExpr,
        span: SourceSpan,
    },
    lambda_expr {
        params: List<FlurryParamDecl>,
        return_type: ?FlurryType,
        body: FlurryExpr,
        span: SourceSpan,
    },
    loop_expr {
        is_inline: bool,
        is_while: bool,
        header: FlurryExpr,
        body: FlurryBlock,
        span: SourceSpan,
    },
    control_expr {
        kind: Symbol,
        value: ?FlurryExpr,
        span: SourceSpan,
    },
    quote_expr {
        subst: List<Tuple<FlurryIdent, FlurryIdent>>,
        body: FlurryExpr,
        span: SourceSpan,
    },
    splice_expr {
        subst: List<Tuple<FlurryIdent, FlurryIdent>>,
        body: FlurryExpr,
        span: SourceSpan,
    },
    macro_invocation {
        macro_path: FlurryPath,
        tokens: TokenBuffer,
        span: SourceSpan,
    },
}

pub enum FlurryStatement {
    let_stmt {
        pattern: FlurryPattern,
        ann: ?FlurryType,
        init: FlurryExpr,
        span: SourceSpan,
    },
    var_stmt {
        pattern: FlurryPattern,
        ann: ?FlurryType,
        init: FlurryExpr,
        span: SourceSpan,
    },
    assign_stmt {
        target: FlurryExpr,
        op: Symbol,
        value: FlurryExpr,
        span: SourceSpan,
    },
    expr_stmt {
        expr: FlurryExpr,
        span: SourceSpan,
    },
    defer_stmt {
        -- Flurry 仅支持 defer expr，无 errdefer 关键字
        body: FlurryExpr,
        span: SourceSpan,
    },
    use_stmt {
        path: FlurryPath,
        span: SourceSpan,
    },
    derive_stmt {
        trait_expr: FlurryExpr,
        target_type: FlurryType,
        span: SourceSpan,
    },
    item_stmt {
        item: FlurryItem,
        span: SourceSpan,
    },
}

pub struct FlurryBlock {
    statements: List<FlurryStatement>,
    tail_expr: ?FlurryExpr,
    span: SourceSpan,
}

-- 程序项结构化成员节点（取代 raw_tokens 退化字段）
pub struct FlurryFieldDef {
    name: FlurryIdent,
    field_type: FlurryType,
    is_pub: bool,
    default_expr: ?FlurryExpr,
    attrs: List<FlurryAttr>,
    span: SourceSpan,
}

pub enum FlurryVariantShape {
    unit,
    tuple(List<FlurryType>),
    struct_fields(List<FlurryFieldDef>),
    pattern_variant(FlurryPattern),
    expr_variant(FlurryExpr),
    sub_enum(FlurryType),
}

pub struct FlurryVariantDef {
    name: FlurryIdent,
    shape: FlurryVariantShape,
    attrs: List<FlurryAttr>,
    span: SourceSpan,
}

pub struct FlurryParamDecl {
    name: ?FlurryIdent,
    param_type: FlurryType,
    is_named_optional: bool,
    is_comptime: bool,
    default_expr: ?FlurryExpr,
    span: SourceSpan,
}

pub struct FlurryAssocDecl {
    name: FlurryIdent,
    assoc_type: FlurryType,
    default_val: ?FlurryExpr,
    clauses: List<FlurryClause>,
    span: SourceSpan,
}

pub struct FlurryMethodDecl {
    name: FlurryIdent,
    attrs: List<FlurryAttr>,
    is_comptime: bool,
    is_pure: bool,
    params: List<FlurryParamDecl>,
    return_type: ?FlurryType,
    clauses: List<FlurryClause>,
    body: ?FlurryBlock,
    span: SourceSpan,
}

pub enum FlurryItem {
    struct_item {
        name: FlurryIdent,
        attrs: List<FlurryAttr>,
        clauses: List<FlurryClause>,
        fields: List<FlurryFieldDef>,
        span: SourceSpan,
    },
    enum_item {
        name: FlurryIdent,
        attrs: List<FlurryAttr>,
        clauses: List<FlurryClause>,
        variants: List<FlurryVariantDef>,
        span: SourceSpan,
    },
    union_item {
        name: FlurryIdent,
        attrs: List<FlurryAttr>,
        clauses: List<FlurryClause>,
        fields: List<FlurryFieldDef>,
        span: SourceSpan,
    },
    newtype_item {
        name: FlurryIdent,
        attrs: List<FlurryAttr>,
        clauses: List<FlurryClause>,
        underlying: FlurryType,
        span: SourceSpan,
    },
    typealias_item {
        name: FlurryIdent,
        attrs: List<FlurryAttr>,
        clauses: List<FlurryClause>,
        target: FlurryType,
        span: SourceSpan,
    },
    fn_item {
        name: FlurryIdent,
        attrs: List<FlurryAttr>,
        params: List<FlurryParamDecl>,
        return_type: ?FlurryType,
        clauses: List<FlurryClause>,
        body: FlurryBlock,
        span: SourceSpan,
    },
    trait_item {
        name: FlurryIdent,
        attrs: List<FlurryAttr>,
        is_pure_comptime: bool,
        supertraits: List<FlurryType>,
        clauses: List<FlurryClause>,
        assoc_items: List<FlurryAssocDecl>,
        methods: List<FlurryMethodDecl>,
        span: SourceSpan,
    },
    impl_item {
        attrs: List<FlurryAttr>,
        is_comptime: bool,
        trait_type: ?FlurryType,
        target_type: FlurryType,
        clauses: List<FlurryClause>,
        assoc_bindings: List<FlurryAssocDecl>,
        methods: List<FlurryMethodDecl>,
        span: SourceSpan,
    },
    effect_item {
        name: FlurryIdent,
        attrs: List<FlurryAttr>,
        clauses: List<FlurryClause>,
        params: List<FlurryParamDecl>,
        resume_type: FlurryType,
        span: SourceSpan,
    },
    effect_enum_item {
        name: FlurryIdent,
        attrs: List<FlurryAttr>,
        clauses: List<FlurryClause>,
        operations: List<FlurryMethodDecl>,
        span: SourceSpan,
    },
    derive_item {
        trait_expr: FlurryExpr,
        target_type: FlurryType,
        clauses: List<FlurryClause>,
        span: SourceSpan,
    },
    deriver_item {
        name: FlurryIdent,
        attrs: List<FlurryAttr>,
        clauses: List<FlurryClause>,
        body: FlurryBlock,
        span: SourceSpan,
    },
    test_item {
        name: ?FlurryLiteral,
        attrs: List<FlurryAttr>,
        body: FlurryBlock,
        span: SourceSpan,
    },
    const_item {
        name: FlurryIdent,
        attrs: List<FlurryAttr>,
        ty: ?FlurryType,
        init: FlurryExpr,
        span: SourceSpan,
    },
    mod_item {
        name: FlurryIdent,
        attrs: List<FlurryAttr>,
        items: List<FlurryItem>,
        span: SourceSpan,
    },
    use_item {
        path: FlurryPath,
        span: SourceSpan,
    },
}
```

#### 3.7.2 `meta.macros`：4 种构造宏、`define` 模板宏与内置宏

依据 [construction-macro.md](file:///home/fin/luna/docs/macro/construction-macro.md)、[template-macro.md](file:///home/fin/luna/docs/macro/template-macro.md)、[library/attrs/macros_impl/handlers.fl](file:///home/fin/luna/library/attrs/macros_impl/handlers.fl) 与 [issues/0004](file:///home/fin/luna/issues/0004-macro-system-carbon.md)：

1. **4 种构造宏属性与编译期展开通道**（统一通过 `^attrs.macros.*` 导出并在 `library/meta/macros_impl/attributes.fl` 补齐 `ast` 与 `peg`）：

```flurry
use attrs.macros.{ast, macro3, macro4, peg}
use builtin.{Integer, Symbol, Type, str}
use meta.ast.{FlurryExpr, SourceSpan, TokenBuffer, ToTokens}
use meta.data.List

pub enum MacroErr {
    SyntaxRejected(str, SourceSpan),
    PegMatchFailed(str, SourceSpan),
    ExpansionFuelExceeded,
    HygieneEscape(Symbol, SourceSpan),
    InvalidOutputKind,
    IncludePathViolation(str),
    MissingEnvVar(str),
}

-- Mode 2 PEG 文法模型
pub enum PegExpr {
    literal(str),
    char_range(str, str),
    rule_ref(Symbol),
    seq(List<PegExpr>),
    choice(List<PegExpr>),
    repeat0(PegExpr),
    repeat1(PegExpr),
    optional(PegExpr),
    and_ahead(PegExpr),
    not_ahead(PegExpr),
    capture(Symbol, PegExpr),
}

pub struct PegRule {
    name: Symbol,
    expr: PegExpr,
}

pub struct PegGrammar {
    entry: Symbol,
    rules: List<PegRule>,
}

pub struct PegNode {
    rule: Symbol,
    span: SourceSpan,
    text: str,
    children: List<PegNode>,
}

pub typealias Repetition<T: Type> = List<T>;
pub typealias Optional<T: Type> = ?T;

-- Mode 1（AST 宏）：^attrs.macros.ast(.variadic = true)
^attrs.macros.ast(.variadic = true)
pub comptime fn sum_exprs(items: List<FlurryExpr>) -> !MacroErr TokenBuffer {
    if items.is_empty() {
        return TokenBuffer.from_str_literal("0");
    }
    var buf = items[0].into_tokens();
    var i: Integer = 1;
    while i < items.length() {
        buf.append({{ + $(items[i]) }});
        i = i + 1;
    }
    buf
}

-- Mode 2（PEG 文法宏）：^attrs.macros.peg(...)
^attrs.macros.peg("entry <- key '=' val; key <- [a-z]+; val <- [0-9]+;")
pub comptime fn kv_pair(tree: PegNode) -> !MacroErr TokenBuffer {
    -- 从 tree.children 读取解析好的 PEG 捕获节点并拼装结果
    TokenBuffer.from_str_literal(tree.text)
}

-- Mode 3（词法 Token 流宏）：^attrs.macros.macro3
^attrs.macros.macro3
pub comptime fn trace_val(tokens: TokenBuffer) -> !MacroErr TokenBuffer {
    {{
        do {
            let _val = $tokens;
            _val
        }
    }}
}

-- Mode 4（Token 输入 -> 结构化 AST/定义直接输出宏）：^attrs.macros.macro4
^attrs.macros.macro4
pub comptime fn dsl_decl(tokens: TokenBuffer) -> !MacroErr TokenBuffer {
    -- 自定义 DSL 解析并可直接调用 meta.definitions 发布或生成结构化代码
    tokens
}
```

2. **`define` 模板宏（卫生与捕获规范）**：
   - 支持模式分支 `(arm1) => { template1 }`、片段指示符 `N`（`expr`, `pat`, `ty`, `stmt`, `item`, `ident`, `tt`, `block`, `path`, `lit`）、重复模式 `N ...` 以及可控卫生逃逸列表 `(ids)`。
   - 默认采用严格**定义点卫生（def-site hygiene）**：模板体声明的临时绑定自动带独立 `hygiene_id`，不会捕获或遮蔽调用点同名局部变量；只有参数位置和显式列于 `(ids)` 中的标识符才接入调用点作用域。
   - 暴露编译期检查接口：`pub pure comptime fn expand_define_macro_once(call_tokens: TokenBuffer) -> !MacroErr TokenBuffer;`。

3. **9 项官方内置宏总表**：

| 内置宏调用 | 参数签名 | 返回类型 / 作用 | 安全、卫生与构建依赖要求 |
|---|---|---|---|
| `@builtin.str.raw(...)` | 原始字符流 | `str` | 无转义原始文本（`registry.rs` 已有基础）。 |
| `@builtin.str.raw_aligned(...)` | 原始缩进块 | `str` | 自动剥离最小公共前导空白（已有 `BUILTIN_RAW_STR_ALIGNED`）。 |
| `@builtin.include_str(rel_path)` | 相对字符串字面量 | `str` | 相对调用点源码目录，校验合法 UTF-8，**禁止逃出包根目录并登记构建重编依赖**。 |
| `@builtin.include_bytes(rel_path)` | 相对字符串字面量 | `Array<u8, N>` | 同沙盒路径限制与构建失效追踪；嵌入静态只读数据段。 |
| `@builtin.env(name, .default = null)` | 变量名与可选默认值 | `str` | 白名单沙盒控制，未设置且无默认值直接报错；记录环境变量依赖键。 |
| `@builtin.line()` | 无参 | `usize` | 当前有效调用位置（含宏透明 span 归属）行号。 |
| `@builtin.file()` | 无参 | `str` | 包内相对逻辑路径（保证跨机器可重现构建）。 |
| `@builtin.stringify(tokens)` | 任意 token 序列 | `str` | 保留规范间隔后的源码文本字符串。 |
| `@builtin.concat_idents(...)` | `Ident`/`Symbol` 列表 | 标识符 | 验证拼接后为合法非关键字 Flurry 标识符并继承调用点卫生上下文。 |

4. **宏系统拒绝例**：

```flurry
-- [拒绝例 3.7-A] 禁止使用裸属性键 ^macro3 绕过 attrs.macros 命名空间
-- ^macro3  -- 拒绝：正式属性必须使用 ^attrs.macros.macro3（或 use attrs.macros.macro3 后写 ^macro3）

-- [拒绝例 3.7-B] @builtin.include_str 禁止访问包根外任意绝对路径
comptime fn leak_etc_passwd() -> str {
    @builtin.include_str("/etc/passwd")  -- 拒绝：IncludePathViolation
}
```

---

### 3.8 `meta.compilation`、`meta.diagnostics` 与 `meta.runtime`

当前 [library/meta/compilation/README.md](file:///home/fin/luna/library/meta/compilation/README.md)、[library/meta/diagnostics/README.md](file:///home/fin/luna/library/meta/diagnostics/README.md) 与 [library/meta/runtime/README.md](file:///home/fin/luna/library/meta/runtime/README.md) 均为空占位说明文件；本节按 [backends-and-artifacts.md](file:///home/fin/luna/docs/pipeline-projection/backends-and-artifacts.md)、[effect-abi.md](file:///home/fin/luna/docs/algebraic-effect-and-delimited-continuation/effect-abi.md) 与 [04-compiler-wishlist.md](file:///home/fin/luna/tmp/silver-design/04-compiler-wishlist.md) 给出完整公开接口。

#### 3.8.1 `meta.diagnostics`（多跨度来源链诊断）

```flurry
use builtin.{str, void}
use meta.ast.SourceSpan
use meta.data.List

pub enum DiagnosticLevel {
    error_level,
    warning_level,
    note_level,
    help_level,
}

pub enum LabelStyle {
    primary,
    secondary,
}

pub struct SpanLabel {
    span: SourceSpan,
    message: str,
    style: LabelStyle,
}

pub struct Suggestion {
    message: str,
    span: SourceSpan,
    replacement: str,
}

pub struct Diagnostic {
    level: DiagnosticLevel,
    code: ?str,
    title: str,
    labels: List<SpanLabel>,
    notes: List<str>,
    suggestions: List<Suggestion>,
}

pub struct DiagnosticBuilder {
    level: DiagnosticLevel,
    code: ?str,
    title: str,
    labels: List<SpanLabel>,
    notes: List<str>,
    suggestions: List<Suggestion>,
}

-- 所有更新方法严格遵守 comptime self 值引用语义
impl DiagnosticBuilder {
    pub pure comptime fn new(level: DiagnosticLevel, title: str) -> Self;
    pub pure comptime fn error_msg(title: str) -> Self;
    pub pure comptime fn warning_msg(title: str) -> Self;
    pub comptime fn with_code(self, code: str);
    pub comptime fn add_primary_label(self, span: SourceSpan, message: str);
    pub comptime fn add_secondary_label(self, span: SourceSpan, message: str);
    pub comptime fn add_note(self, note: str);
    pub comptime fn add_suggestion(self, message: str, span: SourceSpan, replacement: str);
    pub pure comptime fn build(self) -> Diagnostic;
    pub comptime fn emit(self) -> void;
}
```

#### 3.8.2 `meta.compilation`（诊断、环境、Fuel/记忆化与冻结 IIR 投影）

```flurry
use build.TargetInfo
use builtin.{Integer, Symbol, Type, bool, never, str, void}
use meta.ast.SourceSpan
use meta.data.{ComptimeEq, ComptimeHash, List}
use meta.diagnostics.Diagnostic

pub enum CompileErr {
    FuelExhausted,
    UnfrozenIrObserved,
    MissingEnvVar,
}

pub struct SourceLocationInfo {
    file: str,
    line: Integer,
    column: Integer,
    expansion_chain: List<SourceSpan>,
}

-- 诊断退出与提示
pub pure comptime fn err(message: str, .span: ?SourceSpan = null) -> never;
pub comptime fn warn(message: str, .span: ?SourceSpan = null) -> void;
pub comptime fn note(message: str, .span: ?SourceSpan = null) -> void;
pub comptime fn emit_diagnostic(diag: Diagnostic) -> void;

-- 受追踪的环境与目标配置（不与 build 重复定义 TargetInfo）
pub comptime fn env_var(name: str) -> ?str;
pub pure comptime fn cfg() -> TargetInfo;
pub pure comptime fn cfg_flag(key: Symbol) -> bool;
pub pure comptime fn source_info(span: SourceSpan) -> SourceLocationInfo;

-- 计算预算与确定性纯函数记忆化
pub comptime fn remaining_fuel() -> Integer;
pub comptime fn consume_fuel(cost: Integer) -> !CompileErr void;
pub pure comptime fn memoize(
    key: K,
    compute: pure comptime fn(K) -> V,
) -> V
where
    K :- ComptimeEq + ComptimeHash,
    V: Type;

-- 管线投影接口：对齐 docs/pipeline-projection/backends-and-artifacts.md
-- 配合 ^attrs.compilation.pipeline（__flurry_pipeline 待正式登记，见第 5 节 OQ-5.9）使用
-- 强约束：只接受 comptime 已完成且不含未冻结推断变量的 IIR 句柄
pub mod IIR {
    pub struct Func {
        symbol_path: List<Symbol>,
    }

    pub struct AccessToken {
        ir_digest: Integer,
    }

    pub pure comptime fn access(ir: Func) -> !CompileErr AccessToken;
}
```

#### 3.8.3 `meta.runtime`（Silver P0-3 `bounded_stack_bytes` 与管线/宿主桥接）

```flurry
use builtin.{Any, bool, i32, str, u8, usize}
use meta.compilation.IIR
use meta.data.List
use meta.diagnostics.Diagnostic

pub enum HostInvokeErr {
    Diagnostic(Diagnostic),
    CapabilityDenied,
    NonZeroExit(i32),
    ProcessSpawnFailed,
    InvalidArtifact,
    OutputTooLarge,
    TimeoutExceeded,
}

pub struct HostCapability {
    allow_subprocess: bool,
    allow_read_paths: List<str>,
    allow_write_scratch: bool,
    timeout_ms: usize,
}

pub struct HostOutput {
    exit_code: i32,
    stdout_bytes: List<u8>,
    stderr_bytes: List<u8>,
}

-- Silver P0-3 核心阻塞项：计算函数在至多抵达最近 delimited effect 边界前所需静态最大栈字节数
-- 契约边界：
--   1) 要求目标 cfg 已固定且函数调用图完成单态化
--   2) 若调用图含静态未定环（无界递归）、未静态标注栈上界的间接/dyn 调用、或动态大小栈分配，必须保守返回 null
--   3) 计算范围在最近定界 effect/continuation 边界截断，不计入外层 handler 帧
pub pure comptime fn bounded_stack_bytes(entry_fn: Any) -> ?usize;

-- 对齐 docs/pipeline-projection/backends-and-artifacts.md:60-66：
-- 使用冻结 IIR.access(ir) 访问令牌在受控编译器管线/宿主内求值并回传 Diagnostic 错误载荷
pub comptime fn invoke(target_fn: Any, access: IIR.AccessToken) -> !HostInvokeErr Any;

-- 沙盒化外部构建工具链进程调用（默认零权限；不得隐式访问网络或包外宿主文件系统）
pub comptime fn invoke_host_command(
    command: str,
    args: List<str>,
    cap: HostCapability,
) -> !HostInvokeErr HostOutput;
```

#### 3.8.4 `meta.compilation` / `meta.runtime` 拒绝例

```flurry
-- [拒绝例 3.8-A] 无限/静态无界递归入口用于固定栈池时必须返回 null，禁止静默编造字节数
fn unbounded_rec(n: usize) -> usize {
    if n == 0usize { 0usize } else { 1usize + unbounded_rec(n - 1usize) }
}

comptime {
    let bound = meta.runtime.bounded_stack_bytes(unbounded_rec);
    -- 必须返回 null；若跳过 null 检查直接使用或估算绑定值必须编译期拦截：
    bound match {
        bytes? => { let _ = bytes; },
        null => meta.compilation.warn("unbounded recursion cannot use fixed BoundedStackPool"),
    }
}

-- [拒绝例 3.8-B] 未获 HostCapability 授权时调用 meta.runtime.invoke_host_command 必须返回 CapabilityDenied
comptime fn deny_unauthorized_host() -> !HostInvokeErr HostOutput {
    let empty_cap = HostCapability {
        allow_subprocess: false,
        allow_read_paths: [],
        allow_write_scratch: false,
        timeout_ms: 1000usize,
    }
    meta.runtime.invoke_host_command("as", [], empty_cap)  -- 执行外部进程前即触发 error CapabilityDenied
}
```

---

## 4. 排期优先级与分阶段交付路线

下表将 `WL-05-01`..`WL-05-32` 按解锁阻塞程度划分为 3 个纵向交付阶段，并与 [issues/0003](file:///home/fin/luna/issues/0003-builtin-packages-compiler-blockers.md)、[issues/0004](file:///home/fin/luna/issues/0004-macro-system-carbon.md)、[ptc_atoms_status_report_2026-11-21.md](file:///home/fin/luna/ai_dev_logs/ptc_atoms_status_report_2026-11-21.md) 及 [04-compiler-wishlist.md](file:///home/fin/luna/tmp/silver-design/04-compiler-wishlist.md) 双向绑定：

| 阶段 | 包含 Wishlist ID | 核心交付内容与编译器依赖闭合 | 真实下游解锁对象 | 阶段验收门禁 |
|---|---|---|---|---|
| **Phase P0（语言地基与 Silver/内置包解锁）** | `WL-05-01`, `WL-05-05`, `WL-05-06`, `WL-05-07`, `WL-05-10`, `WL-05-11`, `WL-05-13`, `WL-05-16`, `WL-05-20`, `WL-05-23`, `WL-05-25`, `WL-05-27`, `WL-05-29`, `WL-05-31` | 1. 补全 PTC 核心原子 `A02`–`A06`、`A08`–`A12`、`A16`–`A19` 与 `Type.vars.*` 全 5 个开槽；2. `Term::Record` 匿名记录 `Ps` 一等反射 + `inline if has_field` occurrence typing（**Silver P0-2**）；3. `__flurry_packed_mode` 与 `__flurry_macro_*` 顶层属性消费者接通并移除 `.packed` 裸键；4. 彻底封禁 comptime 指针并完成 `library/meta/README.md` 纠偏；5. 接通 `iir/src/staging.rs:3173` staged closure 常量、基础 `meta.code` 与 `^attrs.macros.{ast, peg, macro3, macro4}`；6. 接通 `meta.compilation.{err, warn, cfg}` 与 **`meta.runtime.bounded_stack_bytes`（Silver P0-3）**。 | `Silver P0-2 / P0-3`、`std` 依赖 allocator 容器（`Box`/`Vec`/`String`）、Tier 2 薄协议（如外部 `serialization` 派生）、`issues/0003` 与 `issues/0004`。 | 真实 sysroot + `luna` 端到端集成测试覆盖匿名记录 `Ps` 反射、4 种宏属性展开、`bounded_stack_bytes` 有界/无界区分、`A01`–`A19` 正反/残余/前提错误矩阵。 |
| **Phase P1（完整元编程、CMTT 证据与事务定义）** | `WL-05-02`, `WL-05-03`, `WL-05-04`, `WL-05-08`, `WL-05-09`, `WL-05-12`, `WL-05-14`, `WL-05-15`, `WL-05-17`, `WL-05-19`, `WL-05-21`, `WL-05-22`, `WL-05-24`, `WL-05-26`, `WL-05-28`, `WL-05-30` | 1. `field_offset_of`/`repr_of`/`has_niche`/`needs_drop`/`has_padding`/`type_id`（128-bit Merkle + `__flurry_stable_since`）；2. `enum` 6 种变体完整反射 + `function`/`trait`/`impl`/`module` 全量 `*Info`；3. 事务型 `StructBuilder`/`EnumBuilder`/`TraitBuilder`/`ImplBuilder`/`FunctionBuilder`（`!BuildErr` 原子回滚）；4. `meta.data.Float` 与 `ComptimeTextBytes` `Integer` 化；5. `Any` CMTT 证据见证、`Type.FromCode`、`code_match`/`code_if`/`code_let`；6. 结构化 `meta.ast` (`Parse`/`ToTokens`)、全 9 项 `@builtin.*` 内置宏、`define` 卫生反射与 `DiagnosticBuilder`。 | Tier 2 生态派生物宏与编解码生成器（**位于六内置包之外**的生态库）、跨版本 ABI 检查、复杂嵌入式/内核比特位布局库。 | 事务构造失败零符号泄露回归测试；`type_id` 循环指针防无限递归与 `__flurry_stable_since` 测试；CMTT `Any` 拒绝篡改上下文与双重消费线性资源测试。 |
| **Phase P2（高阶扩展与宿主桥接）** | `WL-05-18`, `WL-05-32` | 1. `UnionBuilder` / `TypeAliasBuilder` 事务扩展；2. `meta.runtime.invoke` 沙盒化编译期宿主能力桥接、确定性增量构建缓存键校验；3. 冻结 `IIR` 管线投影插件扩展口。 | 外部构建管线投影器、自定义资产/着色器编译期流水线与高级 FFI 联合体生成工具。 | `HostCapability` 默认零权限拦截测试；确定性缓存失效/命中测试；未冻结推断状态拒绝对话测试。 |

---

## 5. 待作者裁决的语义空缺与冲突（Open Questions）

以下仅列出在当前规范与代码核查中发现的**真实语义冲突、命名分歧或文档缺口**，每项均附具体文件/行号证据与最小可行默认建议：

### OQ-5.1 `Type.predicates.*` 的短名称与既有 `projections.rs` 长名称拼写冲突
- **来源依据**：`compiler/elaboration/src/program/projections.rs:156-172` 当前硬编码接收 `is_boolean`、`is_integer`、`is_signed_integer`、`is_unsigned_integer`、`is_floating_point`、`is_function`（共 12 个长命名）；而 [atoms.md](file:///home/fin/luna/docs/type/propositional-constraints/atoms.md) 虽写明 `is_tuple` 与 `is_not_sized` 并声明“表格中文描述不额外规定 API 拼写”，下游规格惯例则偏向与语言关键字 `bool`/`fn` 对齐的短命名（`is_bool`、`is_int`、`is_signed_int`、`is_unsigned_int`、`is_float`、`is_fn`）。
- **受影响行为**：所有调用 `Type.predicates.*` 的 `where` 约束与 `inline if` 编译期分支。
- **最小默认建议**：统一以本 Wishlist 第 3.2 节的短命名（`is_bool`/`is_int`/`is_signed_int`/`is_unsigned_int`/`is_float`/`is_fn`）作为正式标准，在编译器迁移窗口期对旧长名发出废弃警告并在正式发布前清理。

### OQ-5.2 返回 `Type` / `?Type` / `!Err Type` 的查询函数命名 `PascalCase` 边界
- **来源依据**：`AGENTS.md` 与 [docs/grammar/index.md](file:///home/fin/luna/docs/grammar/index.md) 规定“返回 `Type` 的函数使用 `PascalCase`”，现有代码 [newtypes.fl:33](file:///home/fin/luna/library/meta/definitions/newtypes.fl#L33)（`Build(self) -> Type`）、[structure.fl:41](file:///home/fin/luna/library/meta/reflection/structure.fl#L41)（`NewtypeRepresentation(self) -> Type`）与 [bindings.fl:20](file:///home/fin/luna/library/meta/bindings.fl#L20)（`CompilerObjectEntryType(...) -> Type`）均遵守此规则；但同文件 [structure.fl:50](file:///home/fin/luna/library/meta/reflection/structure.fl#L50) 存在小写 `enum_bit_base_type(self) -> ?Type`，且下游常把 `Type.from_code`、`Type.predicates.head_of` / `optional_payload` / `field_type` / `return_type` 习惯写作 `snake_case`。
- **受影响行为**：`Type.FromCode` vs `Type.from_code`、`Type.predicates.HeadOf` vs `head_of`、以及返回 `?Type` / `!BuildErr Type` 的函数是否属于“Type-returning”。
- **最小默认建议**：**凡直接或通过可选/错误包装最终交付单个 `Type` 值的函数**（如 `Build() -> !BuildErr Type`、`FieldType(...) -> ?Type`、`FromCode(...) -> !CodeErr Type`、`EnumBitBaseType() -> ?Type`）统一采用 `PascalCase`（`List<Type>` 等容器集合返回函数如 `struct_field_types` 保持 `snake_case`）；待作者确认是否对 `?Type` 豁免或允许别名过渡。

### OQ-5.3 `Type.vars.Reachability` 与 `Type.vars.ReachSet` 命名冲突及文档空白
- **来源依据**：[docs/resource-management/reachability.md:132](file:///home/fin/luna/docs/resource-management/reachability.md#L132) 在现行文档中使用了 `Type.vars.ReachSet`，且 [docs/type/qualified-types/reachability-type.md](file:///home/fin/luna/docs/type/qualified-types/reachability-type.md) 当前为空文件；而规划文档与任务需求普遍使用 `Type.vars.Reachability`。
- **受影响行为**：引用/视图可达性泛型签名 `where r: Type.vars.Reachability` vs `ReachSet`。
- **最小默认建议**：统一规范名为 `Type.vars.Reachability` 并同步更新 `reachability.md` 与 `reachability-type.md`，由作者裁决是否保留或弃用 `ReachSet`。

### OQ-5.4 `ComptimeTextBytes` 索引从 `u64` 迁回 `Integer`
- **来源依据**：[library/meta/data/text.fl](file:///home/fin/luna/library/meta/data/text.fl) 使用 `u64` 作为长度/切片索引，而同目录 `list.fl`、`object.fl`、`map.fl`、`set.fl` 及 `library/meta/bindings.fl` 中的 `str_byte_length`/`str_slice` 原语均统一使用任意精度 `Integer`。
- **受影响行为**：`str` 与 `List`/`Object` 在编译期循环和 `inline for` 下标混用时的无谓类型转换。
- **最小默认建议**：将 `ComptimeTextBytes` 全部长度与索引参数收口为 `Integer`，实现目标无关的编译期文本操作。

### OQ-5.5 宏属性顶层 `__flurry_macro_*` 放行与 `library/meta/macros_impl/attributes.fl` 缺口
- **来源依据**：[library/attrs/macros_impl/handlers.fl](file:///home/fin/luna/library/attrs/macros_impl/handlers.fl) 已定义 `ast`、`peg`、`macro3`、`macro4`，但 [library/meta/macros_impl/attributes.fl](file:///home/fin/luna/library/meta/macros_impl/attributes.fl) 仅再导出 `macro3`、`macro4`；同时 [issues/0004](file:///home/fin/luna/issues/0004-macro-system-carbon.md) 记录 `compiler/elaboration/src/program/attributes.rs` 尚未在顶层声明处放行 `__flurry_macro_*`，而 `compiler/macro_expander/src/registry.rs` 仍把 `@define_macro1`/`@define_macro2` 当作特殊内置名。
- **受影响行为**：用户能否直接以官方 `^attrs.macros.{ast, peg, macro3, macro4}` 属性统一定义全部 4 类构造宏。
- **最小默认建议**：由作者确认保留 `@define_macro1`/`@define_macro2` 仅为兼容糖衣或全面废弃，主线统一通过 `^attrs.macros.*`（`__flurry_macro_*`）进入通用宏注册表。

### OQ-5.6 `library/meta/ast_impl/nodes.fl` 不透明句柄 vs 结构化 AST 模型迁移路径
- **来源依据**：[library/meta/ast_impl/nodes.fl](file:///home/fin/luna/library/meta/ast_impl/nodes.fl) 的 10 个 `Flurry*` 类型（`FlurryExpr`, `FlurryPattern` 等）目前全部只是 `{ tree: SyntaxTree, node: Integer }` 的索引包装，无内部结构字段。
- **受影响行为**：Mode 1/4 宏对语法节点的模式解构与按成分构造能力。
- **最小默认建议**：采用第 3.7.1 节的强类型结构化枚举/结构体作为 `meta.ast` 正式主接口，同时提供 `.to_syntax_tree()` / `from_syntax_tree` 与 `SyntaxTree` 底层存储互转（待作者确认是否需要过渡兼容期）。

### OQ-5.7 `meta.runtime.bounded_stack_bytes` 的目标相关性与无界判定策略
- **来源依据**：[04-compiler-wishlist.md](file:///home/fin/luna/tmp/silver-design/04-compiler-wishlist.md) `P0-3` 要求 `bounded_stack_bytes` 为 `BoundedStackPool` 提供静态精确或保守上界，但 Flurry 的跨架构编译模式允许尚未锁定最终代码生成 ABI 细节的抽象目标配置。
- **受影响行为**：未锁定完整栈帧布局配置、动态间接调用或未单态化函数上调用 `bounded_stack_bytes` 的返回值语义。
- **最小默认建议**：规定未定目标 `cfg` 时按 `Unknown` 编译期拒绝，而动态/间接/递归无界调用统一安全返回 `null`，绝不退化为运行时猜测值。

### OQ-5.8 `@builtin.include_str` / `@builtin.include_bytes` / `meta.runtime.invoke` 的能力授权机制
- **来源依据**：[backends-and-artifacts.md](file:///home/fin/luna/docs/pipeline-projection/backends-and-artifacts.md) 使用 `IIR.AccessToken` 与 `Access` 控制管线/宿主求值，但 `@builtin.include_*` 与 `@builtin.env` 的默认沙盒范围（包根目录相对路径、工程配置白名单变量）尚未在 `docs/code-organization/` 落盘细则。
- **受影响行为**：增量构建缓存精确失效、跨平台路径规范化与 CI 沙盒安全。
- **最小默认建议**：默认仅允许读取包清单所在目录及子目录下的相对路径资源（禁止 `..` 越界与绝对路径），环境变量默认拒绝未在工程构建配置中声明白名单的读取，并全部接入编译器构建依赖图。

### OQ-5.9 未正式登记的官方属性键与管线错误变体命名冲突
- **来源依据**：
  1. [official-attribute-keys.md](file:///home/fin/luna/docs/comptime/official-attribute-keys.md) 与 `library/attrs/` 尚未正式登记 `^attrs.layout.align`（`__flurry_align`，[04-compiler-wishlist.md:91](file:///home/fin/luna/tmp/silver-design/04-compiler-wishlist.md) 提议）、`^attrs.quote_fn_template.{for_match_arms, allow_non_exhaustive_instance}`（[quote-functions-and-templates.md:139-140](file:///home/fin/luna/docs/cmtt/quote-functions-and-templates.md)）及 `^attrs.compilation.pipeline`（`__flurry_pipeline`，[backends-and-artifacts.md:60-66](file:///home/fin/luna/docs/pipeline-projection/backends-and-artifacts.md)）。
  2. [backends-and-artifacts.md:65](file:///home/fin/luna/docs/pipeline-projection/backends-and-artifacts.md) 在 `meta.runtime.invoke(...)` 错误捕获示例中使用了小写 `.diagnostic(diag)` 分支模式，与 `AGENTS.md` 第 5 节“错误枚举变体必须为 `PascalCase`”（即 `Diagnostic(diag)`）冲突。
- **受影响行为**：管线投影脚本、CMTT 开放模板匹配与对齐属性的编译期属性校验通过率及错误模式书写规范。
- **最小默认建议**：在 `docs/comptime/official-attribute-keys.md` 与 `library/attrs/` 正式增补上述 `__flurry_*` 键并保持 `^attrs.*` 封装出口；同时将 `HostInvokeErr` 变体及 `backends-and-artifacts.md` 文档示例统一修订为 `PascalCase` 的 `Diagnostic(Diagnostic)`。
