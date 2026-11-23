# Wave 1 深度差距审计报告：前端词法、语法、AST/HIR、宏系统、名称解析与工具链（`01-frontend-syntax-macros-resolve`）

> 审计基线：仓库分支 `dev`，HEAD commit `b9a52238`
> 审计人：`@frontend-auditor`
> 交付边界：仅分析 `compiler/{lex, parse, ast, hir, ast_lowering, macro_expander, resolve, vfs, symbol, diagnostic, fmt, anar}` 及 `compiler/luna/src/{macro_host.rs, frontend.rs}`（交叉核对 `compiler/luna/src/packages.rs`、`compiler/elaboration/src/program/attributes.rs` 与 `library/{attrs, meta}` 边界）。全文零推断猜测，所有结论均基于实读源码与测试固件。

---

## 1. 审计范围、检查基线与设计标尺

### 1.1 检查范围与代码基线

本次审计覆盖 Flurry 编译器前端全部 12 个独立 crate 及驱动胶水层，非测试源码统计基准如下（排除 `#[cfg(test)]` 与 `tests/`）：
- 扫描范围：`114` 个非测试 `.rs` 源文件，合计 `67,258` 行代码。
- 核心 crate：`lex`、`parse`、`ast`、`hir`、`ast_lowering`、`macro_expander`、`resolve`、`vfs`、`symbol`、`diagnostic`、`fmt`、`anar`。
- 驱动/交叉核对入口：`compiler/luna/src/frontend.rs`、`compiler/luna/src/macro_host.rs`、`compiler/luna/src/packages.rs`（prelude 接线）、`compiler/elaboration/src/program/attributes.rs`（顶层 `__flurry_macro_*` 属性校验）与 `library/{attrs, meta}`（属性与宏接口）。

### 1.2 设计标尺与规范分层

审计严格区分**核心语言规范（Core Norm Target）**与**愿望清单扩展目标（Extended Wishlist Target）**，严禁将任何非权威教学示例或早期历史草案作为语义依据：

1. **核心规范标尺（Core Norm Target）**：
   - 协作与不可破坏边界：`AGENTS.md`（§1 单二进制 `luna`、取消独立 `anar`；§3 六内置包与 `math` 归属；§5 点路径、无 `Option`/`Result`、IR S-expression `Display`、NF 不展开、`Once`、无 comptime 指针等）。
   - 词法与文法：`docs/basics/{lexical.md, literals.md, primitives.md, symbols.md, unit-null-undefined.md}`、`docs/grammar/{index.md, syntax}`、`docs/compilation/syntax-and-hir.md`。
   - 特性专题规范：`docs/others/view-taking.md`、`docs/macro/{construction-macro.md, template-macro.md}`、`docs/code-organization/{index, builtin-packages, imports-and-reexports, modules-and-sources, packages-and-umbrella, scope-and-paths, visibility, configuration-and-scripts}.md`、`docs/comptime/{attribute.md, official-attribute-keys.md}`、`docs/traits/declarations.md`、`docs/functions/{templates.md, calls-and-returns.md}`、`docs/algebraic-effect-and-delimited-continuation/algebraic-effect.md`、`docs/program-synthesis/bitvec-algebra.md`、`issues/0004-macro-system-carbon.md`。
2. **愿望清单扩展标尺（Wishlist Extension Target）**：
   - `tmp/wishlist/00-index-and-roadmap.md`、`tmp/wishlist/01-syntax-protocols-and-operators.md`（如 `WL-01-19/20/29/30`）、`tmp/wishlist/02-pattern-matching-and-views.md`（如 `WL-02-10/20/23/25`）、`tmp/wishlist/05-meta-cmtt-macros-reflection.md`（如 `WL-05-26/27/28`）、`tmp/wishlist/06-attrs-build-pipeline-projection.md`（如 `WL-06-06/07/08`）。

---

## 2. 成熟度总览与统计表

### 2.1 分子系统特性成熟度总表（对应第 3 节 39 项 `GAP-01-NN`）

| 子系统 | `🟢 Full` | `🟡 Partial` | `🟠 Frontend-Only` | `🔴 Missing` | `⚠️ Divergent` | `⚪ Deferred` | 合计 |
|---|---:|---:|---:|---:|---:|---:|---:|
| 词法与语法解析（`lex`, `parse`） | 0 | 5 | 0 | 2 | 5 | 0 | **12** |
| AST / HIR / Lowering / `fmt` | 2 | 2 | 2 | 1 | 3 | 1 | **11** |
| 宏系统与宏宿主（`macro_expander`, `macro_host`） | 2 | 5 | 0 | 0 | 1 | 0 | **8** |
| 解析、基础设施与 LSP（`resolve`, `vfs`, `symbol`, `diagnostic`, `anar`, `frontend`） | 3 | 3 | 0 | 0 | 2 | 0 | **8** |
| **总计** | **7** | **15** | **2** | **3** | **11** | **1** | **39** |

> 注：6 级成熟度严格对应全链路实现深度：`🟢 Full`（前端至下游主路径对齐设计并通过测试）、`🟡 Partial`（核心能力工作但存在规格缺口/边界不齐）、`🟠 Frontend-Only`（仅前端 parse/AST/HIR/lowering 有表示，下游 `elaboration` 等未消费）、`🔴 Missing`（完全未解析或 lowering 直接报错未实现）、`⚠️ Divergent`（live 实现主动违反 `docs/` 或 `AGENTS.md` 规范）、`⚪ Deferred`（按 `AGENTS.md` §3 暂缓高级内容）。

### 2.2 健全性 · 完备性 · 鲁棒性缺陷分级总表（对应第 6 节 `SND / CMP / ROB`）

| 专项维度 | `Critical` | `High` | `Medium` | `Low` | 合计 |
|---|---:|---:|---:|---:|---:|
| 6.1 健全性（`SND-01-NN`） | 3 | 3 | 0 | 0 | **6** |
| 6.2 完备性（`CMP-01-NN`） | 0 | 3 | 3 | 0 | **6** |
| 6.3 鲁棒性（`ROB-01-NN`） | 1 | 2 | 2 | 0 | **5** |
| **总计** | **4** | **8** | **5** | **0** | **17** |

---

## 3. 全量特性差距矩阵（GAP-01-NN）

> 归属标记说明：`[Core]` = 核心 `docs/`/`AGENTS.md` 契约；`[Wishlist]` = `tmp/wishlist/` 扩展需求；`[Core + Wishlist]` = 两者共同覆盖。

| 编号 | 子系统 / 特性项 | 标尺来源 | 成熟度 | 现状与核心源码证据（`file:line`） | 核心差距或偏离摘要 |
|---|---|---|---|---|---|
| `GAP-01-01` | 空格敏感二元算符词法（`+ - * / % < > <= >=`） | [Core] `lexical.md:50`, `grammar/index.md:124` | `🟡 Partial` | `lex/src/lexer.rs:186-267,309-330,413-441` 对 `+ - * / % < >` 输出 `Separated*`，`<=`/`>=` 为普通 `LtEq`/`GtEq`。 | `lexer.rs` 符合 `lexical.md:50`，但 `grammar/index.md:124` 多写了 `<= >=`、`%` 不在其中，`docs/` 与实现尚有文档微调对齐项。 |
| `GAP-01-02` | 标识符字符集（ASCII 限制 vs Emoji 扩展） | [Core] `lexical.md:9` | `⚠️ Divergent` | `lex/src/lexer.rs:7-15` `is_identifier_start` 放行 `U+1F000..U+1F9FF`、`U+2600..U+26FF`、`U+2700..U+27BF`。 | `lexical.md:9` 明确标识符仅以 `a-z`/`A-Z`/`_` 起始；当前 lexer 允许 emoji 标识符属于越界分歧。 |
| `GAP-01-03` | 废弃 `'{ ... }` 词法形态与旧 `::` token | [Core] `construction-macro.md:18-19`, `AGENTS.md` §5 | `⚠️ Divergent` | `lex/src/lexer.rs:514-561,576-606`、`token.rs:169-171`、`error.rs:275-286` 仍保留并词法化 `TokenKind::MacroContent`。 | `'{ ... }` 为旧调用语法遗留（调用点已被 `@path { ... }` 取代；`{{ ... }}` 为宏体内 `TokenBuffer` 构造语法，`^macro4` 为 `^attrs.macros.macro4` 重导出），却仍在词法层有效识别；`ColonColon` 仅用于诊断提示。 |
| `GAP-01-04` | 中缀管线算符 `\|>`（`PipePrepend`）优先级 | [Core + Wishlist] `grammar/syntax`, `WL-01-29` | `⚠️ Divergent` | `parse/src/operators.rs:49-50`、`fmt/src/exprs.rs:2617` 将 `TokenKind::PipeGt` 与 `Pipe` 同设为优先级 `80`。 | 优先级 `80` 高于 `*`/`/`(`70`) 与 `+`/`-`(`60`)，使 `a + b \|> f` 误解析为 `a + (b \|> f)`；按 `WL-01-29` 应为低优先级 `35` 或 `15`，且 `ast_lowering` 尚不支持。 |
| `GAP-01-05` | 逻辑蕴含 `==>`（`BoolImplies`）结合性 | [Core + Wishlist] `grammar/syntax`, `WL-01-30` | `⚠️ Divergent` | `parse/src/expressions.rs:194-198` 仅 `TokenKind::Arrow` 右结合，`operators.rs:20` `EqEqEq` 优先级 `10` 按左结合解析；`fmt/src/exprs.rs:2622-2631` 按右结合格式化。 | `A ==> B ==> C` 被 `parse` 解析为 `(A ==> B) ==> C`，违反蕴含右结合语义并与 `fmt` 规则矛盾；`ast_lowering` 亦未实现降级。 |
| `GAP-01-06` | 保留字与成员/路径/定义位关键字逃逸 | [Core + Wishlist] `lexical.md:23-42`, `WL-06-06` | `⚠️ Divergent` | `parse/src/basic.rs:192-203`、`expressions.rs:1629` 与 `statements.rs:434-438,446-451,463` 对 `default` 放行成员/投影/路径段（仅 `statements.rs:437` 非成员根与 `:523` `as` 别名拒收）；`items.rs:741,1011,1110` 与 `basic.rs:777` 用 `try_id()` 拒收 `default`；`token.rs:868-960` 将 `inline`/`static`/`case`/`catch`/`spec`/`verified` 等作纯关键字。 | 成员投影与路径段仅特判豁免 `TokenKind::Default`，其余 15+ 关键字在 `expr.kw` / 路径成员（如 `attrs.mem.static`）均失败；结构体字段/关联项/属性键/函数定义位则由 `try_id()` 拒绝 `default`（阻塞或限制相关命名一致性）。 |
| `GAP-01-07` | `struct`/`enum`/`trait`/`union` 用 `where` vs `<...>` 与 NF 头参数 | [Core] `traits/declarations.md`, `grammar/syntax:364` | `🟡 Partial` | `parse/src/items.rs:707-787,953-980,1107-1133,1704-1709` 强制类型声明用 `where` 并拒绝运行时 `fn<...>`；`items.rs:1071-1105` 解析 `NormalFormHeadDef`。 | `NormalFormHeadDef` (`items.rs:1077`) 直接复用 `try_clause_with_delimiter(true)`，误放行 `requires`/`ensures`/`decreases` 而未接受 `syntax:364` 允许的 `attr_param`/`comptime`/`implicit` 参数。 |
| `GAP-01-08` | `static_def`, `CaseDef`, `TestDef`/`TestTemplate`, `pure comptime trait`, `effect enum` | [Core] `grammar/syntax:352-375`, `algebraic-effect.md` | `🟡 Partial` | `parse/src/items.rs:1591-1701,1811-1859,1957-2130` 与 `statements.rs:1085-1111` 全部覆盖上述声明与修饰词；`fmt/src/items.rs:2133` 揭示前置 `inline` 顺序限制。 | `CaseDef` 允许 `= expr` 体超出 `syntax:369` 仅列 `block`；声明前写 `inline` 先于 `pub`/`pure` 时会退化成 `NodeKind::InlineStatement` 包裹。 |
| `GAP-01-09` | 模式匹配语法、`BitVec` 模式与 `++` 拼接模式 | [Core + Wishlist] `bitvec-algebra.md`, `WL-02-10` | `🟡 Partial` | `parse/src/patterns.rs:617-735` 解析 `0b`/`0o`/`0x` 多段 `BitVec` 模式（`id`/`int`/`<expr>`/`(pat : expr)`）；`operators.rs:77-99` 定义模式算符表。 | 缺少模式中缀 `++`（`PlusPlus`）算符绑定（Wishlist `WL-02-10`）；且或模式使用 `Or` 关键词（`prec=30`）。 |
| `GAP-01-10` | 多元素属性前缀、分号规则与字面量后缀边界 | [Core] `syntax-and-hir.md`, `grammar/syntax` | `🟡 Partial` | `parse/src/statements.rs:1085-1123` 与 `parse/tests/attribute_boundaries.rs` 支持多属性、`matches` 边界与 `;` 省略；`basic.rs:904-927` 含旧死枚举 `BuiltinAttribute`。 | `expressions.rs:1955-1988` `try_literal_extension_expr` 未检查 token 字节相邻性（对比 `fragment.rs:372` 做了检查）；`basic.rs` 遗留无 `__` 前缀死枚举。 |
| `GAP-01-11` | 未实现的文法产生式（`Outcomes`, `ForType`, `Subtype`, `Predicate`） | [Core] `grammar/syntax:218`, `token.rs:138` | `🔴 Missing` | `ast/src/lib.rs:522,301` 存在 `Outcomes`/`Subtype` `// TODO` 变体；`lex/src/token.rs:138` 词法化 `TokenKind::Predicate`。 | Python AST `NodeKind` 覆盖审计确认：`ForType`、`Outcomes`、`Subtype`（及 `DeclarationBound`、`FieldMethodBound`）在 `parse/src/**/*.rs` 中完全无构造；`TokenKind::Predicate` 未匹配。 |
| `GAP-01-12` | AST 与 HIR S-expression `Display` 完备性 | [Core] `AGENTS.md` §5 | `⚠️ Divergent` | `hir/src/display.rs:1-1157` 完整实现确定性且正确转义的 S-expr `Display`；`ast/src/lib.rs:1261-1653` 仅有 `dump_to_s_expression` 与 `ast_visitor::dump_ast_to_s_expression`。 | `ast` 未实现 `std::fmt::Display` 或带上下文 `Display` 适配器，且 `ast/src/lib.rs:1290` 对叶子节点 `Char`/`Str` 直接拼接裸文本 `format!("({} {})", kind, snippet)`，违反 `AGENTS.md` §5 转义要求。 |
| `GAP-01-13` | 9 项主链路 Take View（`'type`, `'tag_name`, `'old`, `'bitvec`, `'attribute`, `'tuple_enumerate`, `'tuple_indexed`, `'intrinsic`, `'builtin`） | [Core] `view-taking.md`, `attribute.md:106` | `🟡 Partial` | `hir/src/expr.rs:239-253`、`ast_lowering/src/expr.rs:410-495,1276-1307` 与 `elaboration/expression_values/{views.rs,mod.rs}`/`intrinsics.rs` 已全链路接通此 9 种。 | 功能层面主路径打通（`tuple_enumerate` 混杂运行期值场景仍有 `WL-02-24` 约束）；`docs/others/view-taking.md` 文档自身遗漏 `'attribute`。 |
| `GAP-01-14` | `CaseDef` 与 `TestTemplatePort` / `FunctionTemplate` | [Core] `templates.md`, `syntax-and-hir.md` | `🟢 Full` | `ast_lowering/src/item.rs:154-156,566-570,891-892` 降为 `ItemKind::Case` 与 `TestTemplatePort`；`test_templates.rs` / `function_templates.rs` 链接；`elaboration/cases.rs` 与 `luna/tests/{case_definitions.rs,test_templates.rs}` 验证。 | 核心解析、降级、链接与消费闭环完成（函数级开放 `fn foo(case ...)` 属 `WL-02-20` 扩展待裁决）。 |
| `GAP-01-15` | `static_def`（`StaticDef` -> `Static`）与 `InlineControl` | [Core] `grammar/syntax:352`, `AGENTS.md` | `🟢 Full` | `ast_lowering/src/item.rs:2624-2673` 降级 `ItemKind::Static`；`expr.rs:1098-1118` 统一 `InlineControl`；`luna/tests/static_def.rs` 全链路执行。 | 主链路完整（仅 `hir/src/expr.rs:178-190` 残留未清理的 `InlineIf`/`InlineMatch`/`InlineFor` `/// TODO` 死变体）。 |
| `GAP-01-16` | `ast_lowering` 丢弃顶层可见性 `Pub`/`Private` | [Core] `code-organization/visibility.md` | `⚠️ Divergent` | `ast_lowering/src/item.rs:366-367` 注释 `TODO` 后直接 `self.lower_top_level_node(children[0])` 忽略 `Pub`/`Private`；`item.rs:1327-1331` 拒绝字段可见性。 | 虽 `resolve` 保留了 `Visibility`，但 HIR `Item` 丢失显式 `pub`/`private` 结构记录与结构体字段可见性，且阻止后续基于 HIR 的统一检查。 |
| `GAP-01-17` | 裸键属性特判（`intrinsic_enum` / `packed`） | [Core] `official-attribute-keys.md` | `⚠️ Divergent` | `ast_lowering/src/item.rs:201-224` 检查裸标识符 `"intrinsic_enum"`；`elaboration/src/program/attributes.rs:145` 保留裸键 `"packed"`。 | 违背 `docs/comptime/official-attribute-keys.md` 禁止裸键官方属性与禁止 `.id expr` 形式的硬契约（应迁移至 `^attrs.compilation.intrinsic_enum` 等 `__flurry_*` 键）。 |
| `GAP-01-18` | `ast_lowering` 未降级节点与 HIR 存根（`AtomicBlock`, 字符串插值, 逻辑/类型命题等） | [Core] `literals.md:87`, `bitvec-algebra.md` | `🔴 Missing` | `ast_lowering/src/expr.rs:79-81,822-826,1177-1179,3277`；`hir/src/expr.rs:209,223-237`。 | 字符串插值 `$name`/`${expr}` 返回 `E2001`；`AtomicBlock` 报错未支持；`AsyncBlock`/`ComptimeBlock` 退化为普通 `Block`；`ReachabilityType`/`Forall`/`Exist`/`Implication`/`Subtype`/`TypedWith` 全为 HIR 空变体。 |
| `GAP-01-19` | `compiler/fmt` 格式化与跨构造自验证 | [Core] `grammar/syntax`, `fmt/tests/` | `🟡 Partial` | `fmt/src/{formatter,bridge,items,exprs,patterns,verify}.rs` 与 `tests/stage{1,2}_*.rs` 实现源码格式化、指令（`.off`/`.skip`）与重解析 AST 校验。 | 依赖 `bridge.rs` 预处理 `@use`/`define`/`@define_macro1/2`/`@path{...}`；8 个 `NodeKind` 未匹配（如 `TokenBuffer*`/`ClosureEnvironmentBinding`）；共享 `\|>` 优先级 `80` 缺陷（`exprs.rs:2617`）。 |
| `GAP-01-20` | 模板宏（`define` / `$` / `$$` 及卫生性扩展） | [Core + Wishlist] `template-macro.md:34,52`, `WL-05-28` | `🟡 Partial` | `macro_expander/src/template.rs:139-149,416-1040` 与 `luna/tests/macros_template.rs` 完整实现核心 `define N`/`define N ...`/`define(params)`、`$0`/`$name`/`$$` 及嵌套 `{{ ... }}` 跳过；无 def-site hygiene。 | 核心 `template-macro.md` 无卫生 token 替换已完备（`🟢`）；但愿望清单扩展 `WL-05-28`（`tmp/wishlist/00` §5.4）要求的严格 def-site hygiene、模式分支与 `(ids)` 卫生逃逸尚未实现（展开标识符仅赋 `expansion_id` 直接克隆）。 |
| `GAP-01-21` | Mode 1 构造宏（`ast` 片段输入与编解码） | [Core] `macro/construction-macro.md` | `🟢 Full` | `macro_expander/src/ast_input.rs`、`luna/src/macro_host.rs:1505-1673,1880-1975` 与 `luna/tests/macros_construction_ast.rs` 完整实现。 | 覆盖 `Flurry*` / `Optional<T>` / `Repetition<T>` 校验、AST 子树快照、与 `meta.ast` 往返编解码及位置映射。 |
| `GAP-01-22` | Mode 2 构造宏（`peg` 规则与 Packrat 解释） | [Core] `macro/construction-macro.md` | `🟡 Partial` | `macro_expander/src/peg.rs` 与 `luna/tests/macros_construction_peg.rs` 实现 `Entry` 检查、`~ \| & ! * + ?`、Packrat `rule_memo`/`flurry_memo` 与预算上限。 | `peg.rs:332-460` 基于最小不动点空串可达性分析静态拒绝所有直接与间接左递归文法；部分非左递归扩展文法能力受限。 |
| `GAP-01-23` | Mode 3 (`.tokens`)、Mode 4 (`.source`)、`{{ ... }}` 与 `@builtin.str.raw*` | [Core + Wishlist] `construction-macro.md:19`, `WL-05-28` | `🟢 Full` | `macro_expander/src/{raw_source,construction,materialize}.rs`、`luna/src/macro_host.rs:1080-1118,2182-2234` 与 `luna/tests/{macros_construction_raw,macros_token_buffer_and_raw}.rs`。 | 核心 `TokenBuffer` 重词法校验、`@builtin.str.raw`/`raw_aligned` 与 `{{ ... }}`/`$` 物化完备（`WL-05-28` 其余内置宏如 `@builtin.{include_str,line}` 属愿望清单未实现）。 |
| `GAP-01-24` | `ExpansionMap` 宏展开溯源链与 Span 映射 | [Core] `macro/construction-macro.md` | `🟡 Partial` | `macro_expander/src/{provenance.rs,lib.rs,materialize.rs}` 实现 `ExpansionId`/`ExpansionRecord` 父子链与误差回溯报告。 | `ast_input.rs:231`、`raw_source.rs:1243`、`peg.rs:895` 在若干节点构造与内联语法报错时仍回退 `DUMMY_SP` 或粗粒度 `call_site`。 |
| `GAP-01-25` | 顶层 `__flurry_macro_*` 属性接通、旧键清理与 `meta.macros` 导出 | [Core + Wishlist] `official-attribute-keys.md`, `issues/0004`, `WL-05-27` | `🟡 Partial` | `elaboration/src/program/attributes.rs:126-139`、`luna/src/macro_host.rs:1449-1501`、`library/attrs/macros_impl/handlers.fl:3-14`、`library/meta/macros_impl/{attributes.fl:1,mod.fl:2}`。 | 编译器已全面放行 `__flurry_macro_{kind,variadic,grammar}` 并清除旧 `flurry_macro_kind`，`attrs.macros` 已导出位置参 `ast(variadic: bool)` / `peg(grammar: str)` / `macro3` / `macro4`；**但 `meta/macros_impl/{attributes.fl,mod.fl}` 仍仅重导出 `{macro3, macro4}` 而遗漏 `ast`/`peg`**。 |
| `GAP-01-26` | `@define_macro1` / `@define_macro2` 编译器硬编码特判债务 | [Core] `issues/0004` | `⚠️ Divergent` | `macro_expander/src/{registry.rs:19-40,244-285, raw_source.rs:1074-1097, template.rs:159-188, define_macros.rs:142-177,615-691}`。 | 仍特殊注册 `BUILTIN_DEFINE_MACRO1/2`、在词法流截获 `const ... = @define_macro1/2` 改写成 `^attrs.macros.ast/peg(...)` 并硬编码补全 `meta.ast.*`，违背统一宏机制设计。 |
| `GAP-01-27` | `@use` 导入与 `raw_source` 预词法扫描约束 | [Core] `macro/construction-macro.md` | `🟡 Partial` | `macro_expander/src/{imports.rs,raw_source.rs}` 实现 `@use` 分组/通配/别名/菱形去重、歧义与访问控制。 | `raw_source.rs:1160` 在预词法阶段即强制解析全文所有 `@` 路径；`scan_raw_balanced_braces` (`131-163`) 不跳过字面量/注释中的 `{`/`}`。 |
| `GAP-01-28` | 6 内置包 5 默认 Prelude 注入（排除自包与 `std`） | [Core] `builtin-packages.md`, `AGENTS.md` §5 | `🟡 Partial` | `resolve/src/module_builder.rs:1009-1140`、`luna/src/frontend.rs:707-714`、`resolve/tests/prelude_providers.rs`、`luna/tests/sysroot_preludes.rs`。 | `build_package_set_with_preludes` 与 `compile_sysroot` 正确排除了提供者自身与 `std`；但 `packages.rs:459,604` 通用兼容入口默认仅传 `[builtin]` 单一 prelude。 |
| `GAP-01-29` | 统一命名空间、3 级可见性（`pub`/默认 package/`private`）与环状 `pub use *` | [Core] `visibility.md`, `imports-and-reexports.md`, `scope-and-paths.md` | `🟢 Full` | `resolve/src/{binding.rs,module_builder.rs,item_scope.rs}` 与 `resolve/tests/{access.rs,single_imports.rs}`。 | 统一类型/值/模块/标签表；`Binding.accessible` 同步检查 `vis` 与重导出 `entry`；`rebuild_globs` 不动点精确收敛有环 glob 并保持顺序无关性。 |
| `GAP-01-30` | `.path` / `..path` 锚点可越过 Package 根进入 `ScopeKind::Root` | [Core] `scope-and-paths.md:60` | `⚠️ Divergent` | `resolve/src/module_builder.rs:657-670,1105-1120`；`ast_scanner.rs:845-874`。 | `Super(n)` 未拦截跨出 `ScopeKind::Package` 边界，在包根写 `use .builtin...` 可退入注册了外部依赖别名的合成 `ScopeKind::Root`，违反明确静态错误要求。 |
| `GAP-01-31` | 延迟名称查找（`NameResolutionPlan`）与 `TraitSourcePlan` | [Core] `traits/declarations.md` | `🟢 Full` | `resolve/src/{deferred.rs,resolver.rs,resolver/trait_sources.rs}` 与 `resolve/tests/{deferred_outputs.rs,trait_sources.rs}`。 | 保留 pending 站位直到生成器完成且按词法顺序遮蔽，确定性收集可见 trait 拼写集合与父 trait 路径。 |
| `GAP-01-32` | VFS、全局 `Symbol` 驻留、多文件包发现与 `scripts/` 隔离 | [Core] `modules-and-sources.md`, `configuration-and-scripts.md` | `🟢 Full` | `vfs/src/lib.rs`、`symbol/src/lib.rs`、`luna/src/frontend.rs:430-472`、`resolve/src/scanner/vfs_scanner.rs`。 | `FileId`/`AstNodeId`/`DefIdGen` 多包隔离与 `lib.fl`/`main.fl`/`mod.fl` 目录模块发现稳定，支持 `scripts/` 自动分离与编译期构建依赖隔离。 |
| `GAP-01-33` | 诊断模型、`failure_summary` 与跨位置 span 精度 | [Core] `diagnostic/src/lib.rs` | `🟡 Partial` | `diagnostic/src/lib.rs:193-446` 提供结构化稳定 `failure_summary` 与 `ariadne` 彩色渲染。 | `resolve/scanner/ast_scanner.rs:1040`、`module_builder.rs:312`、`vfs_scanner.rs:142,238,265` 等多处核心报错硬编码 `DUMMY_SP` 丢失源码位置。 |
| `GAP-01-34` | `anar` 单二进制融合定位与残留命名/阶段债务 | [Core] `AGENTS.md` §1 | `⚠️ Divergent` | `Cargo.toml:29`、`compiler/anar/Cargo.toml:5` (`autobins = false`)、`luna/src/main.rs:24` (`luna lsp`)、`anar/src/{server.rs:88,580, analysis.rs:79, cli.rs:12}`。 | 已遵从 `AGENTS.md` §1 取消独立 `anar` 二进制并合入 `luna lsp`；但服务器标识、诊断源 `"anar"` 与 `settings.flurry.anar.sysroot` 命名遗留未清理，且能力架构停留在 HIR 前期。 |
| `GAP-01-35` | `anar` LSP 编辑支持完整度（13 项 LSP Provider） | [Core] `AGENTS.md` §1, `anar/tests/lsp_integration.rs` | `🟡 Partial` | `anar/src/{server.rs,analysis.rs,index.rs}` 实现初始化/增量编辑/跳转/引用/高亮/悬停/补全/签名/大纲/重命名/折叠/语义 token/格式化。 | `analyze_package` (`analysis.rs:378-580`) 不执行 `macro_expander` 与 `elaboration`：无类型/借用/comptime 错误诊断、无法解析宏展开生成代码，且 `.` 补全依赖文本 `detail` 截串与词法启发式；跨包解析失败时静默丢弃错误回退单包（`495`）。 |
| `GAP-01-36` | 多维与切片下标语法 `xs[0..2, ..]` / `SpaceApply` 索引适配 | [Wishlist] `WL-01-19`, `WL-01-20` | `🔴 Missing` | `parse/src/expressions.rs:1502` 仅解析单个表达式下标 `[expr]`。 | 不支持逗号分隔多下标/切片索引形式；属 `tmp/wishlist/01` 明确规划缺失项。 |
| `GAP-01-37` | Take View `'multi_deref`（`ExprKind::MultiDeref`） | [Core + Wishlist] `view-taking.md`, `WL-02-25` | `🟠 Frontend-Only` | `parse` -> `hir/src/expr.rs:251` -> `ast_lowering/src/expr.rs:486` -> `hir/src/display.rs:941` -> `elaboration/lexical_inputs/dependencies.rs:937`。 | `elaboration/src/program/expression_values/{mod,views,places}.rs` 完全缺失 `MultiDeref` 消费分支，使用即触发未实现错误。 |
| `GAP-01-38` | `UnionDef` 与 `ExtendDef` / `ExtendTraitDef` 语法声明 | [Core] `grammar/syntax`, `traits/declarations.md` | `🟠 Frontend-Only` | `parse/src/items.rs` 与 `fmt/src/bridge.rs:515,518` 构造并格式化；`resolve/scanner/ast_scanner.rs:74` 仅扫描 `UnionDef`。 | `hir/src/item.rs` 无 `Union`/`Extend` 项；`ast_lowering/src/item.rs` 无匹配分支（从 `expr.rs:1396-1398` 回落时直接报 `emit_unsupported_node`）。 |
| `GAP-01-39` | 形式化验证语句（`axiom`, `asserts`, `invariant`） | [Core — 暂缓] `AGENTS.md` §3 | `⚪ Deferred` | `parse/src/statements.rs`、`fmt/src/items.rs:1772-1778` 支持解析与格式化；`ast_lowering/src/**/*.rs` 零匹配。 | 按 `AGENTS.md` §3“暂缓 `verification` 高级内容”原则保留前端语法边界，待验证子系统展开后再打通 HIR 与证明传递。 |

---

## 4. 关键缺口深度剖析（含源码证据）

### 4.1 算符优先级与结合性核心冲突：`|>`, `==>`, 与 `parse` / `fmt` 规则分裂（`GAP-01-04`, `GAP-01-05`）

1. **管线算符 `|>` (`PipePrepend`) 优先级反转**：
   - 在 `compiler/parse/src/operators.rs:49-50` 中：
     ```rust
     TokenKind::Pipe => ExprOpInfo::new(80, NodeKind::Pipe),        // |
     TokenKind::PipeGt => ExprOpInfo::new(80, NodeKind::PipePrepend), // |>
     ```
   - `compiler/fmt/src/exprs.rs:2617` 复制了同一错误值：`NodeKind::Pipe | NodeKind::PipePrepend => 80`。
   - 由于 `+`/`-` 为 `60`、`*`/`/`/`%`/`++` 为 `70`、`->` 为 `50`、比较/`:`/`:-`/`matches` 为 `40`、`and` 为 `30`、`or` 为 `20`、`==>` 为 `10`（`operators.rs:15-50`），`80` 意味着 `1 + 2 |> f` 被解析为 `1 + (2 |> f)`，`a == b |> f` 被解析为 `a == (b |> f)`，彻底违背管线操作符“接收左侧整体表达式送入右侧”的设计初衷。更进一步，`ast_lowering/src/expr.rs` 中完全未匹配 `NodeKind::Pipe` 与 `NodeKind::PipePrepend`（直接掉入 `E2001` unsupported node）。
2. **逻辑蕴含 `==>` (`BoolImplies`) 左结合错误与内部工具撕裂**：
   - `compiler/parse/src/expressions.rs:194-198` 在 Pratt 循环计算右结合最小优先级时硬编码：
     ```rust
     let right_prec = if token.kind == TokenKind::Arrow {
         op_info.prec
     } else {
         op_info.prec + 1
     };
     ```
   - 这使得所有非 `->` 二元操作符——包括 `==>`（`prec = 10`）、`:`（`TypedWith`, `40`）、`:-`（`TraitBound`, `40`）、`<:`（待实现）——均按**左结合**解析（`(A ==> B) ==> C`）。然而同一仓库的 `compiler/fmt/src/exprs.rs:2622-2631` 中 `is_right_assoc_binary` 却将 `NodeKind::BoolImplies | TraitBound | Subtype | TypedWith` 判定为**右结合**！这意味着同一段未加括号的链式蕴含或约束表达式在 `parse` 与 `fmt` 之间的 AST 优先级语义直接打架，甚至可能被 `fmt/src/verify.rs` 的重解析结构等价检查拦截。

### 4.2 关键字上下文逃逸残缺：`default` 单点特判与 `attrs.derivation.default` 阻塞（`GAP-01-06`）

- `docs/basics/lexical.md:23-30` 定义完整关键字表，`:36-42` 列出 10 个核心保留词（`Fn`, `axiom`, `default`, `extend`, `ghost`, `lemma`, `move`, `opaque`, `opens`, `static`）并规定已定义属性路径成员（如 `attrs.derivation.default`）按路径位置解释。
- 在 `compiler/parse/src/basic.rs:192-203`，`try_member_id` 函数仅针对 `TokenKind::Default` 做了单点特判：
  ```rust
  TokenKind::Id => self.try_id(),
  TokenKind::Default => { ... NodeKind::Id ... }
  ```
- 受此影响，`parse/src/expressions.rs:1629`（`try_select_expr` 调用 `self.try_member_id()?`）已接受 `obj.default`，`parse/src/statements.rs:434-438,446-451,463`（`try_path_component` 在 `member = true`、`.` 后或 `{ ... }` 内调用 `try_member_id()?`）也接受 `default` 作为导入路径成员（仅 `:437` 非成员根及 `:523` `as` 别名处调用 `try_id()`）。**真正拒绝 `default` 的是定义与属性键位**：`parse/src/items.rs:741`（`try_struct_field`）、`parse/src/items.rs:1011`（`try_assoc_decl`）、`parse/src/basic.rs:777`（`try_property`）与 `parse/src/items.rs:1110`（`try_function`）均直接调用 `try_id()`。同时，其余 15+ 关键字（`static`、`inline`、`atomic`、`case`、`catch`、`spec`、`verified`、`ghost`、`move`、`opaque`、`opens`、`extend`、`axiom`、`lemma`、`Fn`）甚至连 `try_member_id` 成员/路径位置（如 `attrs.mem.static`、`expr.inline`）都未放行。

### 4.3 Take View 全景现状：`9 + 1` 分化与文档滞后（`GAP-01-13`, `GAP-01-37`）

与部分早期猜测文档不同，实读 `HEAD` (`b9a52238`) 证实：**全部 10 个 Take View 均已拥有独立的 `hir::ExprKind` 变体并在 `ast_lowering` 完成降级**（`compiler/hir/src/expr.rs:239-253`、`compiler/ast_lowering/src/expr.rs:410-495,1276-1307`、`compiler/hir/src/display.rs:932-941`）：
- **9 项已完整接入 `elaboration`**：`TakeOld` (`'old`)、`TakeTupleEnumerate` (`'tuple_enumerate`)、`TakeTupleIndexed` (`'tuple_indexed`)、`TakeIntrinsic` (`'intrinsic`)、`TakeAttribute` (`'attribute`)、`TakeTagName` (`'tag_name`)、`TakeBitVec` (`'bitvec`)、`TypeOf` (`'type`)、`Builtin` (`'builtin`) 均在 `compiler/elaboration/src/program/expression_values/{views.rs,mod.rs}` 或 `intrinsics.rs` 中实际消费。
- **唯一 `🟠 Frontend-Only` 断层**：`MultiDeref` (`'multi_deref`) 虽在 `ast_lowering/src/expr.rs:486` 生成 `ExprKind::MultiDeref` 且在 `elaboration/src/program/lexical_inputs/dependencies.rs:937` 遍历其依赖，但在 `elaboration/src/program/expression_values/{mod,views,places}.rs` 完全缺失匹配分支（与 `WL-02-25` 吻合）。
- **核心文档遗漏**：`docs/others/view-taking.md` 仅列出 7 个用户态 + 2 个内置视角，**漏列已在 `docs/comptime/attribute.md:106` 与编译器中正式实现的 `'attribute`**（与 `WL-02-23` 吻合）。

### 4.4 宏系统：`__flurry_macro_*` 已打通，但 `@define_macro1/2` 特判与 `raw_source` 括号扫描仍是硬伤（`GAP-01-25`, `GAP-01-26`, `GAP-01-27`）

1. **与过时 `issues/0004` Phase 1 的关键差异与 `meta` 重导出遗漏（`GAP-01-25`）**：
   - `tmp/wishlist/06` 与早期记录曾称 `elaboration/attributes.rs` 拦截顶层 `__flurry_macro_*`。但经核实当前 `HEAD`：**`compiler/elaboration/src/program/attributes.rs:126-139` 已明确放行 `__flurry_macro_kind` (`Symbol`)、`__flurry_macro_variadic` (`Bool`)、`__flurry_macro_grammar` (`Str`)**，且仓库全量搜索已无任何 `flurry_macro_kind` 残留；`library/attrs/macros_impl/handlers.fl:3-14` 已导出位置参 `ast(variadic: bool)`、`peg(grammar: str)`、`macro3`、`macro4`，`compiler/luna/src/macro_host.rs:1449-1501` 已完整读取该套官方键。**但注意库接口尚未全闭环**：`library/meta/macros_impl/attributes.fl:1` 仍只包含 `pub use attrs.macros.{macro3, macro4};`（且 `library/meta/macros_impl/mod.fl:2` 仅 `pub use attributes.{macro3, macro4}`），尚未重导出 `ast` 与 `peg`。
2. **尚存的两处深层宏架构债**：
   - **`@define_macro1`/`@define_macro2` 特判未拆解（`GAP-01-26`）**：`macro_expander/src/registry.rs:19-40,244-285`、`raw_source.rs:1074-1097`、`template.rs:159-188`、`define_macros.rs:142-177,615-691` 仍把这两个定义宏当成特权内建 `MacroId`（`BUILTIN_DEFINE_MACRO1`/`2`），在 token 流阶段扫描 `const ... = @define_macro1/2 { ... }`、硬编码 `is_meta_ast_exported_type` 自动补全 `meta.ast.` 前缀并拼出 `^attrs.macros.ast/peg(...)`。同时 `raw_source.rs:1081` 把 `["meta", "macros", "define_macro1" | "define_macro2"]` 与裸名 `@define_macro1/2` 硬编码豁免为无需跨包准备的“内置根”。
   - **Mode 4 大括号平衡扫描忽略字符串/注释，以及模板宏卫生扩展缺口（`GAP-01-20`, `GAP-01-27`）**：`raw_source.rs:131-163` 的 `scan_raw_balanced_braces` 逐字节无脑统计 `b'{'`/`b'}'`，不跳过字符串字面量或 `--` / `{- -}` 注释，若 `@my_raw_macro { "hello }" }` 内含未配对字面量花括号会立即在 `}` 处提前截断宏输入。此外，`process_raw_source_and_imports` (`raw_source.rs:1160`) 要求词法前即能对文件内所有 `@call` 完成 `resolve_call_path`。在模板宏方面，核心 `docs/macro/template-macro.md:34,52` 按设计执行无卫生 token 替换（`template.rs:139-149,571` 同时跳过嵌套 `{{ ... }}` `TokenBuffer`），而按 `WL-05-28`（`[Wishlist]`）规划的严格 def-site 词法卫生与逃逸列表尚未实现（当前仅赋 `expansion_id` 用于诊断溯源）。

### 4.5 名称解析与包组织：5 默认 Prelude 已就位，但 `SuperPath` 越包根边界漏守（`GAP-01-28`, `GAP-01-30`）

- **5 默认 Prelude 机制**：`resolve/src/module_builder.rs:1009-1140` (`build_package_set_with_preludes`) 与 `compiler/luna/src/frontend.rs:707-714` 已按设计实现：从 sysroot 的 `["builtin", "std", "meta", "build", "attrs", "verification"]` 中过滤排除 `std`，将余下 5 个包的 `prelude.*` 作为 `Visibility::Package` 通配导入注入除提供者自身之外的其他所有包（测试覆盖 `resolve/tests/prelude_providers.rs` 与 `luna/tests/sysroot_preludes.rs`）。但驱动兼容函数 `luna/src/packages.rs:459,604`（如 `compile_with_builtin`）仍默认只注入 `[builtin]`。
- **`SuperPath` 越界进入 `ScopeKind::Root` 漏洞（`GAP-01-30`）**：
  - `docs/code-organization/scope-and-paths.md:60` 严格要求：**“越过 package 根是静态错误，umbrella 不提供更外层语言作用域。”**
  - 然而在 `resolve/src/module_builder.rs:1055-1121` 中，每个 `ScopeKind::Package` 的 `parent` 被设为全编译共享的合成 `ScopeKind::Root`，且该 `Root` 中以公开绑定注册了所有依赖包别名（如 `builtin`, `std`）。而在 `resolve_path_segments` (`module_builder.rs:657-670`) 处理 `PathAnchor::Super(n)` 时：
    ```rust
    for _ in 0..n {
        sid = self.scope_tree.get(sid).and_then(|s| s.parent)...
    }
    ```
    未检查步进中是否已越出 `ScopeKind::Package`！当用户在包顶层模块书写 `use .builtin.foo;`（1 个前导点）时，`sid` 从当前 `Package` 合法滑入 `Root` 并成功找到依赖包 `builtin`，悄然绕过了“不允许前导点跳出当前包”的静态语义规则。

### 4.6 `anar` LSP 与单二进制 `luna`：已合并 CLI 但深度停留在 HIR 前期（`GAP-01-34`, `GAP-01-35`）

- 按照 `AGENTS.md` §1“取消独立 anar 计划，保持 `luna` 单二进制 CLI”的决定，`compiler/anar/Cargo.toml:5` 已设 `autobins = false`，`compiler/luna/src/main.rs:24` 通过 `luna lsp` 调度 `anar::cli::run`。
- 然而 `anar/Cargo.toml` 不依赖 `macro_expander`、`middle`、`elaboration` 或 `nir`。`WorkspaceAnalyzer::analyze_package` (`anar/src/analysis.rs:378-580`) 仅串联 `lex -> parse -> resolve -> ast_lowering -> SymbolIndex`：
  1. **零宏展开与零类型检查**：遇到使用 `@define_macro1/2` / `^macro3/4` / `define` 的源码时直接报错或丢失生成符号；无法提供类型推断报错、效应/资源报错或准确重载/trait 解析，与 `anar/src/cli.rs:20` 自述“For full type checking, use luna check”一致但远未达到完整语言服务器预期。
  2. **文本切片推断类型补全**：`lookup_dot_members` (`analysis.rs:1260-1347`) 通过扫描前缀 token（`lexical_local_types_before`）及按 `:` / `.` 字符串分割 `SymbolInfo.detail`（`1308-1324`）来猜测链式成员类型，无法处理泛型实例化、类型别名、依赖类型或函数返回推断。

---

## 5. 架构偏离与技术债清单（`⚠️ Divergent` 专项）

下表汇聚第 3 节全部 11 项直接违反 `AGENTS.md` 或现行 `docs/` 规范的实装偏离：

| 编号 | 偏离实装 | 违背的核心规范依据 | 源码位置 | 架构风险与清理优先级 |
|---|---|---|---|---|
| `GAP-01-02` | `lex` 放行 3 段 Emoji 起始标识符 | `docs/basics/lexical.md:9`（仅 `a-z`/`A-Z`/`_`） | `lex/src/lexer.rs:7-15` | 破坏词法跨工具一致性，应立即收紧（P1）。 |
| `GAP-01-03` | 仍词法化废弃 `'{ ... }` (`MacroContent`) | `docs/macro/construction-macro.md`（仅 `{{ ... }}`） | `lex/src/lexer.rs:514-561`, `token.rs:169-171`, `error.rs:275-286` | 若与 char 字面量边界交互会误词法化，建议降级为显式语法迁移错误后删除（P2）。 |
| `GAP-01-04` | `\|>` 优先级错设为 `80` | `docs/grammar/syntax`, `WL-01-29` | `parse/src/operators.rs:50`, `fmt/src/exprs.rs:2617` | 静默改变表达式求值树，必须与 `fmt` 同步降至低优先级（P0）。 |
| `GAP-01-05` | `==>` 按左结合解析（`fmt` 按右结合） | `docs/grammar/syntax`, `WL-01-30` | `parse/src/expressions.rs:194-198`, `fmt/src/exprs.rs:2622-2631` | 解析与格式化语义分裂，改坏蕴含逻辑链（P0）。 |
| `GAP-01-06` | 成员/路径/定义位关键字逃逸缺失 | `docs/basics/lexical.md:36-42`, `WL-06-06` | `parse/src/basic.rs:192-203`, `statements.rs:434-523` | `try_member_id` 仅特判 `default`，阻塞 `attrs.derivation.default` 等内置库路径（P0）。 |
| `GAP-01-12` | `ast` 缺 `Display` 且 S-expr 不转义叶子字面量 | `AGENTS.md` §5（所有 IR 须实装转义 S-expr `Display`） | `ast/src/lib.rs:1261-1653`, `ast/src/ast_visitor.rs` | 含 `)`/`"` 的字符或字符串可直接破坏 `luna --emit ast` S-expr 可解析性（P1）。 |
| `GAP-01-16` | `ast_lowering` 丢弃顶层 `Pub`/`Private` 并拒字段可见性 | `docs/code-organization/visibility.md` | `ast_lowering/src/item.rs:366-367,1327-1331` | 切断 HIR 对可见性的精确携带与错误诊断能力（P1）。 |
| `GAP-01-17` | 硬编码裸属性名 `intrinsic_enum` 与 `packed` | `docs/comptime/official-attribute-keys.md` | `ast_lowering/src/item.rs:201-224`, `elaboration/attributes.rs:145` | 沿用禁止的裸键，应统一经 `__flurry_*` 键值读取（P1）。 |
| `GAP-01-26` | `@define_macro1`/`@define_macro2` 编译器硬编码特判 | `issues/0004`, `AGENTS.md` §1（拒绝局部特判） | `macro_expander/src/{registry,raw_source,template,define_macros}.rs` | 维持专有 `BUILTIN_DEFINE_MACRO1/2` 通道、预词法白名单及 AST 重词法重写（P1）。 |
| `GAP-01-30` | `SuperPath` 可越过 `ScopeKind::Package` 进入 `Root` | `docs/code-organization/scope-and-paths.md:60` | `resolve/src/module_builder.rs:657-670,1105-1120` | 包顶层 `.dep` 隐式跨越包根界线偷读依赖表（P0）。 |
| `GAP-01-34` | `anar` LSP 残留独立品牌/配置路径债 | `AGENTS.md` §1（取消独立 `anar` 计划） | `anar/src/{server.rs:88,580, analysis.rs:79, cli.rs:12}` | `serverInfo`/诊断源/配置项仍以 `"anar"` 暴露而非 `"luna-lsp"`（P3）。 |

---

## 6. 健全性 · 完备性 · 鲁棒性专项审计（SND / CMP / ROB）

### 6.1 健全性审计（Soundness, `SND-01-NN`）

| 编号 | 严重度 | 缺陷标题 | 证据与静态/语义破坏机理 |
|---|---|---|---|
| `SND-01-01` | **Critical** | `\|>` 与 `==>` 静默误解析及与 `fmt` 语义撕裂 | `parse/src/operators.rs:50`（`\|>`=`80`）、`expressions.rs:194-198`（`==>` 左结合）与 `fmt/src/exprs.rs:2617,2626`（`\|>`=`80`、`==>` 右结合）。`a + b \|> f` 与 `A ==> B ==> C` 均不报任何语法错误即悄然产出错误表达式树，且格式化重构时产生解析/打印不一致。 |
| `SND-01-02` | **Critical** | `ast_lowering` 静默丢弃可见性与异步/编译期块标记 | `ast_lowering/src/item.rs:366-367` 直接忽略 `NodeKind::Pub`/`Private` 包装；`expr.rs:822-826` 把 `AsyncBlock`/`ComptimeBlock` 均降作普通 `ExprKind::Block` 并依赖旁路 `pending_semantics`，若旁路消费缺失即导致块执行域失真。 |
| `SND-01-03` | **Critical** | `SuperPath` 突破包根界线泄漏外层合成 `Root` 绑定 | `resolve/src/module_builder.rs:657-670` 对 `Super(n)` 步数仅判断 `s.parent.is_some()`，不拦截跨过 `ScopeKind::Package`，导致包顶层 `.builtin...` 能跳过正规路径锚点直接读取 `Root` 内注册的依赖别名（`1105-1120`）。 |
| `SND-01-04` | **High** | 模板宏 `define` 无词法卫生与绑定捕获风险（Wishlist `WL-05-28` 扩充项） | `docs/macro/template-macro.md:34,52` 核心设计原意即无卫生 token 替换；但依 `WL-05-28`（`[Wishlist]`）高阶卫生目标审视：`macro_expander/src/template.rs:993-1040` 中 `substitute_group` 克隆字面标识符仅赋 `expansion_id`（`81-84`）而无作用域卫生集合，宏体 `let tmp = ...` 会捕获或遮蔽调用点同名局部绑定。 |
| `SND-01-05` | **High** | 字面量后缀未验紧邻与 `NormalFormHeadDef` 吞子句 | `parse/src/expressions.rs:1955-1988` 对 `123 m`（带空格）不校验 `to == from` 即尝试识别字面量扩展（区别于 `fragment.rs:372`）；`items.rs:1077` 复用 `try_clause_with_delimiter(true)` 使 `pure comptime fn F<...>` 误纳契约子句并拒收合法律定参数。 |
| `SND-01-06` | **High** | `anar` 以空 AST 替身与文本启发式掩盖跨包/宏失败 | `anar/src/analysis.rs:495-504` 在 `build_package_set_with_preludes` 失败时直接 `let _ = err;` 并退回单包解析；`841` 对坏文件注入 `build_empty_ast`；`1260-1347` 用字符串切割 `detail` 推断类型，造成假阳性与漏报。 |

### 6.2 完备性审计（Completeness, `CMP-01-NN`）

| 编号 | 严重度 | 缺陷标题 | 证据与受阻范围 |
|---|---|---|---|
| `CMP-01-01` | **High** | `parse` 对 5 个 `Ast::NodeKind` 及 `Predicate` / 多维索引零支持 | `ast/src/lib.rs:203-608` 共定义 **245** 个 `NodeKind`（含 `Invalid = 0`，有效语法节点 **244** 个），其中 `Outcomes`、`ForType`、`Subtype`、`DeclarationBound`、`FieldMethodBound` 在 `parse/src/**/*.rs` 中零构造；`TokenKind::Predicate`（`token.rs:138`）未解析；`expressions.rs:1502` 不支持 `xs[0..2, ..]`。 |
| `CMP-01-02` | **High** | `ast_lowering` 对 28 个 `NodeKind` 无真实降级及 HIR 空变体堆积 | `ast_lowering/src/**/*.rs` 完全不匹配 `UnionDef`/`ExtendDef`/`AtomicBlock`/逻辑量化与蕴含等节点，直接报 `E2001`；字符串插值（`expr.rs:79-81,3277`）显式拒绝；`hir/src/expr.rs:209-237` 大量命题节点仍是无法承载子节点的 unit 存根。 |
| `CMP-01-03` | **High** | 成员/路径/定义位关键字逃逸覆盖不足 | `parse/src/basic.rs:192-203` 仅救回 `default` 成员/路径位，其余 15+ 关键字在投影、路径段（如 `attrs.mem.static`、`module.case`）全部失败；同时 `items.rs:741,1011,1110` 与 `basic.rs:777` 直接用 `try_id()` 拒收 `default`。 |
| `CMP-01-04` | **Medium** | `'multi_deref` 前端断层与 `fmt` 8 个节点直配缺失 | `hir::ExprKind::MultiDeref`（`ast_lowering/src/expr.rs:486`）止步于前端与依赖图遍历，下游 `elaboration/expression_values` 零处理；`fmt` 对 `ClosureEnvironmentBinding` 等 8 个节点仅靠跳过或后备路径应对。 |
| `CMP-01-05` | **Medium** | `ast` S-expr `Display` 缺失与 `@define_macro1/2` 一等属性未统一 | `ast/src/lib.rs:1261-1653` 缺带上下文 `Display` 实现且不转义输出字符；`macro_expander` 尚未取消 `@define_macro1/2` 硬编码直通以纯 `^attrs.macros.*` 统领。 |
| `CMP-01-06` | **Medium** | `packages.rs` 默认入口未带 5 prelude 与 PEG 静态拒左递归 | `luna/src/packages.rs:459,604` 默认单设 `builtin` prelude；`macro_expander/src/peg.rs:332-460` 静态禁止所有左递归规则而未提供受控右扩机制。 |

### 6.3 鲁棒性审计（Robustness, `ROB-01-NN`）

#### 非测试源码 Panic 面与非确定性定量普查（67,258 行 / 114 个 `.rs` 文件）

以下数据通过 AST 作用域外精确扫描 `compiler/{lex,parse,ast,hir,ast_lowering,macro_expander,resolve,vfs,symbol,diagnostic,fmt,anar}/src/**/*.rs` 与 `compiler/luna/src/{frontend.rs,macro_host.rs}`（已剔除全部 `#[cfg(test)]` 与 `tests/` 目录）实测得出：

| crate / 模块 | `.unwrap()` | `.expect()` | `unreachable!` | `panic!` | `todo!` / `unimpl!` | `[idx]` 裸索引 | `DUMMY_SP` / 默认 Span | `HashMap`/`HashSet` 新建 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `ast_lowering` | 20 | 18 | 35 | 0 | 0 | 387 | 4 | 12 |
| `ast` | 40 | 2 | 0 | 2 | 0 | 141 | 2 | 0 |
| `resolve` | 20 | 1 | 0 | 0 | 0 | 93 | 8 | 19 |
| `macro_expander` | 9 | 0 | 1 | 0 | 0 | 96 | 13 | 13 |
| `parse` | 6 | 2 | 1 | 0 | 0 | 25 | 14 | 3 |
| `fmt` | 6 | 1 | 0 | 0 | 0 | 619 | 1 | 2 |
| `luna` (`frontend`+`macro_host`) | 4 | 0 | 0 | 0 | 0 | 37 | 0 | 11 |
| `hir` | 2 | 3 | 0 | 0 | 0 | 8 | 0 | 30 |
| `lex` | 2 | 0 | 0 | 0 | 0 | 0 | 0 | 2 |
| `anar` | 1 | 0 | 1 | 0 | 0 | 27 | 5 | 22 |
| `symbol` | 0 | 1 | 0 | 0 | 0 | 0 | 0 | 0 |
| `diagnostic` | 0 | 0 | 0 | 0 | 0 | 0 | 2 | 0 |
| `vfs` | 0 | 0 | 0 | 0 | 0 | 0 | 1* | 0 |
| **合计** | **110** | **28** | **38** | **2** | **0** | **1,433** | **49** | **114** |

> 注：以上显式可 panic 调用总计 **178** 处（`.unwrap()` 110 + `.expect()` 28 + `unreachable!` 38 + `panic!` 2）；另有安全回退 `.unwrap_or*` **91** 处未计入危险 panic 面。（* `vfs` 仅有 1 处 `.unwrap_or_default` 与 0 处 `DUMMY_SP`。）`DUMMY_SP` 列统计全部 **49** 处非测试源码引用（`4+2+8+13+14+1+5+2 = 49`，包含 `use` 导入与 `==`/`!=` 判等过滤），其中实际**发射或回退为 `DUMMY_SP` 的非判等位置共 27 处**（`resolve` 8、`macro_expander` 6、`parse` 5、`ast_lowering` 4、`anar` 2、`ast` 1、`diagnostic` 1）。热点 panic 文件集中于 `ast_lowering/src/item.rs`（27 处）、`ast_lowering/src/expr.rs`（25 处）、`ast/src/ast_visitor.rs`（22 处）、`ast/src/lib.rs`（22 处）、`resolve/src/module_builder.rs`（17 处）。

| 编号 | 严重度 | 缺陷标题 | 源码证据与崩溃 / 非确定性触发场景 |
|---|---|---|---|
| `ROB-01-01` | **Critical** | AST 访问器、S-expr 转储与目录模块生成高危 Panic 点 | `ast/src/ast_visitor.rs:235` 直接 `panic!("Cannot visit invalid node")` 及 `:278-554` 计 20 处 `.unwrap()`；`ast/src/lib.rs:1269-1632` `dump_to_s_expression` 内计 20 处 `.unwrap()`，在错误恢复残缺 AST 或无 span 节点上可崩溃；`ast_lowering/src/providers.rs:107-135` 对 `scope.name.unwrap()` 与 `definition_owner(...).unwrap()` 在残缺/空包或 LSP `build_empty_ast` 场景存在 panic 风险。 |
| `ROB-01-02` | **High** | Parser 恢复 span 断言与 Mode 4 花括号截断脆弱性 | `parse/src/expressions.rs:270,280,1404-1405,1980-1981` 对 `self.ast.get_span(...)` 直接 `.unwrap()`/`.expect()`；`macro_expander/src/raw_source.rs:131-163` `scan_raw_balanced_braces` 不跳过字符串字面量和注释中的 `{`/`}`，导致含字面括号的 Mode 4 输入提前闭合或触发语法混乱；`raw_source.rs:1160` 在预词法扫描期对尚未生成作用域的 `@` 调用过早报错。 |
| `ROB-01-03` | **High** | 语法分析、lowering 与 PEG Packrat 缺少显式递归深度上限 | `macro_expander/src/peg.rs:1003-1263` 虽有步数与 `max_memo_entries` 预算，`parse/src/{expressions.rs,patterns.rs}` 与 `ast_lowering/src/{expr.rs,item.rs}` 均有高深度嵌套递归但无 `max_recursion_depth` 计数器（`frontend_recovery.rs` 仅测了小片段单字节变异）。极端嵌套如 `((((...))))` 或深链式 PEG 规则在达到步数上限前即可触发 OS 线程栈溢出（`SIGABRT`）。 |
| `ROB-01-04` | **Medium** | `HashMap`/`HashSet` 无序遍历影响诊断顺序稳定性 | 全前端存在 114 处 `HashMap`/`HashSet` 构建；虽 `hir/display.rs`、`trait_sources.rs:134` 与 `module_builder.rs:456` 做了显式排序，但 `resolve/src/module_builder.rs:297-315` 按 `selected_candidates()` 哈希顺序遍历并发射 `"ambiguous public glob name"` 诊断、`trait_sources.rs:60-66` 收集拼写键依赖哈希序，跨运行或缓存变动下报错顺序不稳定。 |
| `ROB-01-05` | **Medium** | 非判等 `DUMMY_SP`（27 处发射/回退）诊断定位丢失 | `resolve/src/{module_builder.rs:312, ast_scanner.rs:1040, vfs_scanner.rs:142,238-239,265}` 多处关键报错（公开通配歧义、重复定义首位置、同模块多文件冲突）均直接发射 `DUMMY_SP` 丢失波浪线锚点；`macro_expander/src/peg.rs:895-900` 与 `ast_lowering/src/providers.rs:110-112` 在规则校验与生成模块项处亦注入 `DUMMY_SP`（注：`luna/src/frontend.rs:445` 的 `file_name().unwrap()` 位于 `:430-433` `Some("main.fl" \| "lib.fl")` 守卫内，属已防御场景）。 |

---

## 7. 修复路径、依赖与验收建议

按正确性优先（`AGENTS.md` §1 拒绝用局部特判凑测试）编排 P0–P3 演进里程碑：

### P0（阻断语义正确性与静态边界，须最先修复）
1. **统一 `|>` 优先级与 `==>` 右结合性（`GAP-01-04`, `05`, `SND-01-01`）**：
   - 将 `parse/src/operators.rs:50` 与 `fmt/src/exprs.rs:2617` 中的 `|>` (`PipePrepend`) 优先级从 `80` 降至与算术/比较协调的低优先级（按 `WL-01-29` 定案选用 `35` 或 `15`），并在 `parse/src/expressions.rs:194-198` 将 `EqEqEq` (`==>`) 及 `Colon`/`TraitBound`/`Subtype` 与 `fmt/src/exprs.rs:2622-2631` 统一设为右结合；同步更新 `parse/tests/precedence.rs` 与 `fmt/tests/`。
2. **修复 `SuperPath` 越出 Package 根边界漏洞（`GAP-01-30`, `SND-01-03`）**：
   - 在 `resolve/src/module_builder.rs:657-670` 的 `PathAnchor::Super(n)` 循环中显式禁止穿过 `ScopeKind::Package` 步入合成 `ScopeKind::Root`（步出即报明确静态路径错误），并补写 `resolve/tests/single_imports.rs` 负例。
3. **补全关键字路径/成员/定义位逃逸与 `try_literal_extension_expr` 紧邻检查（`GAP-01-06`, `10`, `SND-01-05`, `CMP-01-03`）**：
   - 梳理 `docs/basics/lexical.md:36-42` 的 10 个核心保留词与普通关键字矩阵，在 `parse/src/basic.rs:192-203` 将路径与成员投影扩展至合理关键字集（解锁 `attrs.derivation.default` 与 `attrs.mem.static`）；在 `parse/src/expressions.rs:1955-1988` 补齐 `tokens_adjacent` 校验。

### P1（核心语言特性缺口与基础设施契约对齐）
4. **保真 HIR 可见性与块种类 + 消除裸属性键特判（`GAP-01-16`, `17`, `SND-01-02`）**：
   - 停止在 `ast_lowering/src/item.rs:366-367` 剥离 `NodeKind::Pub`/`Private`，将其结构化存入 HIR `Item` 与结构体字段；区分 `AsyncBlock`/`ComptimeBlock`/`AtomicBlock` HIR 表达；清理 `ast_lowering/src/item.rs:201-224` 裸 `"intrinsic_enum"` 与 `elaboration/attributes.rs:145` 裸 `"packed"`。
5. **消除 AST S-expr 转义与 `ast` / `ast_lowering` Panic 面（`GAP-01-12`, `ROB-01-01`, `02`, `03`）**：
   - 为 `ast::Ast` 实现携带 `SourceMap` 的 `Display` 适配器（对齐 `hir/src/display.rs` 的 Lisp S-expr 字符串转义规范），将 `ast_visitor.rs` 与 `ast/src/lib.rs` 共 44 处 `.unwrap()`/`.expect()`/`panic!` 转为可恢复错误结果；为 Pratt 解析、`ast_lowering` 与 `peg.rs` 增加显式 `recursion_depth` 上限保护。
6. **推进缺失文法与 HIR 降级（`GAP-01-11`, `18`, `37`, `38`, `CMP-01-01/02/04`）**：
   - 按需求批次补全 `Outcomes`、`ForType`、`Subtype (<:)`、`TokenKind::Predicate`、`BitVec`/拼接模式 `++`、字符串插值、类型/量化命题节点与 `'multi_deref` 下游 `elaboration` 消费者；清理 `hir/src/expr.rs` `InlineIf`/`InlineMatch`/`InlineFor` 与 `parse/src/basic.rs` `BuiltinAttribute` 死代码。

### P2（宏系统通用化与工具确定性完善）
7. **消除 `@define_macro1/2` 特判并强化 `raw_source` / `meta.macros` 重导出（`GAP-01-03`, `20`, `25`, `26`, `27`, `SND-01-04`）**：
   - 顺延 `issues/0004` 遗留 Phase 4–6 目标，在 `library/meta/macros_impl/{attributes.fl,mod.fl}` 补齐 `ast`/`peg` 重导出并将 `@define_macro1/2` 降为普通库宏与 `^attrs.macros.*` 属性入口，消除 `macro_expander/src/{registry,raw_source,template,define_macros}.rs` 硬编码旁路；改进 `scan_raw_balanced_braces` 跳过字符串/注释；清理废弃词法分支 `'{ ... }`，并按 `WL-05-28` 路线决定是否引入 def-site hygiene。
8. **消除前端 `HashMap` 诊断非确定性与 `DUMMY_SP` 热点（`GAP-01-33`, `ROB-01-04`, `05`）**：
   - 对 `module_builder.rs:297-315` 等多处哈希迭代加稳定排序，并在 `ast_scanner.rs:1040` / `vfs_scanner.rs` / `module_builder.rs:312` / `peg.rs:895-900` / `providers.rs:110-112` 补齐首定义 span 与错误定位；统一 `packages.rs` 5 prelude 入口。

### P3（LSP 与文档一致性收口）
9. **升级 `luna lsp`（`anar`）至真实宏展开与类型/elaboration 驱动，完成去 `anar` 命名债（`GAP-01-34`, `35`）**：
   - 让 `anar/src/analysis.rs` 按需接入 `macro_expander` 与轻量 `middle`/`elaboration` 增量查询以取代字符切割 `detail` 推断与单包静默吞错降级，并将 `"anar"` 元信息与配置键平稳迁移至 `luna lsp`；同步更新 `docs/others/view-taking.md`（补 `'attribute`）与 `docs/grammar/index.md`（对齐空格算符表）。

### 验收建议与检查闭环
- 每次修复按单一纵向功能提交本地 commit，优先覆盖：`compiler/parse/tests/precedence.rs`（新增 `|>` 优先级与 `==>` 右结合正反例）、`compiler/resolve/tests/single_imports.rs`（新增包根 `.` 越界静态拒绝测试）、`compiler/ast/tests/`（S-expr 转义与残缺恢复无 panic 测试）、`compiler/macro_expander/tests/`（Mode 4 含字面量 `}` 测试与卫生冲突正反例）及 `compiler/luna/tests/frontend_recovery.rs`（深度嵌套与恢复压测）。
