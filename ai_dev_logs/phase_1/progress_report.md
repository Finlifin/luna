# Phase 1 进度报告

> 初次调查: 2026-03-12
> 最近更新: 2026-03-13 (MIR / Codegen 已创建，端到端编译成功)

## 总览

Phase 1 的目标是建立一个能够端到端编译简单 Flurry 程序的最小可行管线，涵盖：基本 item 功能、基本泛型（clause decl 子句量化）、基于 query 的编译管线、Typed HIR、MIR、Codegen。

| 模块                          | 进度   | 状态 |
|-------------------------------|--------|------|
| 基本 item 功能 (AST/Parse/HIR/Lowering/Resolve) | 95%    | ✅ 已连接到 driver |
| 基本泛型 / clause decl         | 70%    | ⚠️ 数据结构完整，语义处理缺失 |
| 基于 query 的编译管线          | 30%    | ⚠️ 框架就绪，无实际 query 接入 |
| Typed HIR (类型检查/推断)      | 40%    | ⚠️ 基础 typeck 已实现，路径解析待完善 |
| MIR 阶段                      | 60%    | ✅ 基础 MIR 完成，函数/控制流/调用已工作 |
| Codegen 阶段                   | 50%    | ✅ C 代码生成已工作，`factorial(10) = 3628800` |

### 本次推进成果

1. **`intrinsic::initialize()` 已实现** — 不再 `todo!()` panic，注册了所有原始类型 lang items
2. **Name Resolution 已接入 driver** — `main.rs` 中调用 `resolve::build_module_tree()`，输出 4 defs / 4 scopes
3. **typeck crate 已创建** — 基础类型检查框架：
   - `resolve_ty` 模块: HIR 类型表达式 → `Ty<'tcx>` 转换（原始类型、指针、可选、函数类型、元组）
   - `check` 模块: 遍历 HIR 包，为函数签名和表达式赋予类型
   - 已验证: `fn factorial(n: Integer) -> Integer` 正确推断为 `fn(Int) -> Int)`
4. **Property 语法已更新** — 从 `.id expr` 改为 `id: expr`
5. **MIR crate 已创建** (`compiler/mir/`):
   - 完整的 MIR 数据结构: `Body`, `BasicBlockData`, `Statement`, `Terminator`, `Place`, `Operand`, `Rvalue`, `Constant`
   - 索引类型: `Local`, `BasicBlock`
   - 终结器: `Goto`, `SwitchInt`, `Return`, `Call`, `Unreachable`
   - MIR pretty-printing (`display.rs`)
6. **MIR builder 已创建** (`compiler/mir_build/`):
   - HIR → MIR 降级: 函数体、字面量、二元/一元运算、函数调用、if-else 控制流、let 绑定、return、赋值
   - 正确的基本块生成和控制流连接
7. **C 代码生成已创建** (`compiler/codegen/`):
   - MIR → C99 代码生成
   - 类型映射: `Int → int64_t`, `Float → double`, `Bool → bool`, etc.
   - 基本块 → C labels + goto
   - 自动生成 test main
8. **端到端编译已成功**: `test.fl` → lex → parse → resolve → lower → typeck → MIR → C → executable
   - `factorial(10) = 3628800` ✅

---

## 1. 基本 Item 功能 — 90% ✅

### 已完成

- **AST** (`compiler/ast/src/lib.rs`): 扁平索引式 `NodeKind` 枚举，100+ 节点类型，覆盖：
  - `Function`, `StructDef`, `EnumDef`, `TraitDef`, `ImplDef`, `ImplTraitDef`
  - `ExtendDef`, `ExtendTraitDef`, `TypealiasDef`, `NewtypeDef`, `ModuleDef`
  - `CaseDef` (normal-form/case 方法)、`UseStatement`
  - 完整的表达式、模式、参数变体

- **Lexer** (`compiler/lex/src/`): 完整的 tokenizer，86+ token 类型，错误恢复

- **Parser** (`compiler/parse/src/`): 手写 PEG 解析器
  - Pratt 优先级攀升的表达式解析
  - 完整的 item/表达式/模式/语句解析
  - case 定义解析 (`items.rs` L1655-1715)

- **HIR** (`compiler/hir/src/`): Arena 分配的类型化 IR
  - `Package<'hir>` 包含 `owners: IndexVec<LocalDefId, OwnerInfo>` + `bodies: FxHashMap<BodyId, Body>`
  - 完整的 item 定义：`FnSig`, `StructDef`, `EnumDef`, `TraitDef`, `ImplDef`, `ModDef`, `UsePath`
  - 表达式：值表达式 + 类型表达式共 30+ 种
  - 模式：`Wild`, `Binding`, `Const`, `Tuple`, `Struct`, `TupleStruct`, `Or`, `Ref`, `Path`, `Range`
  - HirId 体系：`OwnerId + ItemLocalId`

- **AST Lowering** (`compiler/ast_lowering/src/`): AST → HIR 完整转换
  - `lower_to_hir()` 入口，item/expr/pattern/clause/path 各模块完整
  - clause 声明拆分为 `ClauseParam` + `ClauseConstraint`

- **Name Resolution** (`compiler/resolve/src/`): 两阶段解析
  - 阶段 1: VFS 扫描 + AST 扫描，构建 `ScopeTree`
  - 阶段 2: 导入解析，不动点循环 + 循环检测
  - `Resolver` 提供名称查询接口
  - Rib 栈式词法作用域解析

### 未完成

- ~~**Name Resolution 未接入 driver**~~ ✅ 已完成
- ~~**`intrinsic::initialize()` 未实现**~~ ✅ 已完成
- **Resolver 未与 AST Lowering 深度集成**: lowering 仍不使用 resolver 做路径解析

---

## 2. 基本泛型 / Clause Decl — 70% ⚠️

### 已完成

- **AST 层**: `TraitBoundDeclClause`, `TypeBoundDeclClause`, `VarargDeclClause`, `QuoteDeclClause`, `TypeDeclClause` 节点类型

- **HIR 层** (`compiler/hir/src/clause.rs`, `ty.rs`):
  ```
  ClauseParam { ident, bounds: &[TraitBound], hir_id, span }
  ClauseConstraint { kind: Param(Ident) | Bound(Ident, &[TraitBound]) | Predicate(&Expr) }
  TraitBound { kind: Trait(Path) }
  ```
  - 所有定义 item (fn/struct/enum/trait/impl) 均携带 `clause_params` + `clause_constraints`

- **Lowering** (`compiler/ast_lowering/src/clause.rs`): AST clause → HIR ClauseParam/ClauseConstraint 拆分

- **类型表示** (`compiler/ty/src/types.rs`):
  - `TyKind::Param(ParamTy)` — 类型参数
  - `TyKind::Adt(AdtId, &[Ty])` — 带泛型实参的 ADT

### 未完成

- **泛型实例化 (substitution)**: 无替换/单态化逻辑
- **约束求解**: 无 trait bound 检查、无 where clause 验证
- **泛型参数作用域**: resolve 中泛型参数的作用域绑定未实现
- **类型参数到 `Ty::Param` 的映射**: typeck 阶段缺失导致无法建立

---

## 3. 基于 Query 的编译管线 — 30% ⚠️

### 已完成

- **Query 基础设施** (`compiler/query/src/lib.rs`):
  - `QueryCache<K, V>`: 带 memoization 的缓存
  - `QueryEngine`: 全局协调器，活跃查询栈，循环检测
  - `QueryInvocation` / `CycleError`: 诊断支持
  - RAII guard 保证 panic 安全性
  - 详尽的文档说明了如何添加新 query

- **Compiler handle** (`compiler/interface/src/lib.rs`):
  - `CompilerInstance<'sess>`: 拥有所有编译时数据
  - `Compiler<'c>`: 薄 `Copy` handle，解引用到 `CompilerInstance`
  - `enter()` / `compiler()` 方法提供只读上下文

### 未完成

- **无任何实际 query 定义**: 没有 `Queries` struct，没有 `type_of` 等 query provider
- **管线仍为过程式**: `main.rs` 中 lex → parse → lower 是命令式调用，未使用 query
- **按需计算未启用**: 所有阶段无法被 query engine 调度和缓存
- **缓存和增量编译**: query 文档提及的 incremental compilation 尚未实现

---

## 4. Typed HIR (类型检查/推断) — 30% ⚠️

### 已完成

- **类型数据结构** (`compiler/ty/src/`):
  - `Ty<'tcx>`: interned thin pointer，`Copy + Hash + Eq`
  - `TyKind<'tcx>`: 15 种语义类型
    - `Primitive`, `Unit`, `Tuple`, `Ref`, `Ptr`, `Optional`, `Fn`, `Array`, `Slice`
    - `Adt`, `Param`, `Infer`, `Never`, `Error`
  - `PrimTy`: i8/i16/i32/i64/i128, u8/u16/u32/u64/u128, f32/f64, bool, string, char
  - `InferTy`: `TyVar(u32)` / `IntVar(u32)` / `FloatVar(u32)`
  - `ParamTy`, `AdtId`

- **TyCtxt** (`compiler/ty/src/context.rs`):
  - `TypedArena<TyKind>` 内存管理
  - `TyInterner` 去重表
  - `CommonTypes` 缓存常用类型

- **typeck crate** (`compiler/typeck/src/`):
  - `resolve_ty`: HIR 类型表达式 → `Ty<'tcx>` (原始类型、指针、可选、函数、元组、never、void)
  - `check`: 函数签名类型注册、body 遍历、表达式类型推断
  - 支持: 字面量、二元/一元运算、if、block、return、let、tuple、ref/deref、match、array、cast
  - 已集成到 driver 中

### 未完成

- **类型推断引擎**: 无 unification table、无约束收集、无约束求解
- **类型检查 pass**: 无 HIR 遍历为节点赋予 `Ty<'tcx>`
- **类型标注解析**: HIR 中的类型表达式 (`TyPtr`, `TyFn`, `TyOptional` 等) 无法转换为 `Ty<'tcx>`
- **方法解析**: 无方法查找、trait 方法分派
- **Typed HIR 输出**: HIR 节点上没有附加类型信息的字段

---

## 5. MIR 阶段 — 0% ❌

**完全不存在。** 代码库中没有任何 MIR 相关的代码、模块或数据结构。

需要从零设计和实现：
- MIR 数据结构（基本块、语句、终结器、Place、Operand）
- HIR → MIR lowering
- 控制流图构建
- MIR 优化 pass（至少需要去糖/简化）

---

## 6. Codegen 阶段 — 0% ❌

**完全不存在。** 代码库中没有任何代码生成后端。

需要从零设计和实现：
- 后端选择（LLVM / Cranelift / 自定义）
- MIR → 后端 IR 翻译
- 目标文件生成
- 链接器集成

---

## 基础设施状态

| 组件 | 状态 | 说明 |
|------|------|------|
| Diagnostics | ✅ 完成 | ariadne 彩色输出，源码位置，非致命错误恢复 |
| VFS | ✅ 完成 | 文件管理、AST 存储、AstNodeId 全局唯一标识 |
| Source Map | ✅ 完成 | 使用 rustc_span |
| Sysroot | ✅ 完成 | builtin/std 包扫描和加载 |
| Intrinsics | ✅ 完成 | `initialize()` 已实现，注册所有原始类型 lang items |
