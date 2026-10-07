# 编译：正式文档与实现边界

- 日期：2026-10-07
- global_index：080
- theme：compilation
- subindex：02
- 本轮目标：按作者批准的 077 方案及新增约束，完成编译主题正式文档。

## 作者输入与授权

作者本轮说“已回复”。prompt 末尾明确“已审阅，批准”，并补充：

1. Parser 允许任意 multi 产生式前任意深度的 attribute 前缀，沿用该设计。
2. Expr 与 pattern 的上下文敏感设计用于避免后缀扩展应用与外层结构的歧义。
3. 每一种 take view 操作均应在 AST lowering 时直接形成对应 HIR node kind。
4. Lex/parse 现有实现有较大复用价值，其他阶段可以重新组织。
5. 运行时依赖类型采用两层表示：底层为便于 intern 的带洞统一类型结构，引用端带 substitution。
6. IIR 尚未到 SSA 层级，保留很多高级语言特征；其中类型的 comptime 部分必须计算完成。
7. Elaboration 与 comptime 交织进行。

作者答复全文已按原始字节追加到被回答的 [077](077-compilation-01.md)，随后清空 prompt。本轮批准包含原 C1、C2、C3 的 A 方案；不再把它们当作未决题重复询问。上一轮拼写更正继续适用，统一使用 comptime invocation。

## 已确认与实现推导

| 约束 | 本轮落实 |
|---|---|
| C1：冻结稳定残余 | 定义结果可保留显式绑定、契约和合法残余；活动推断状态不进入持久缓存 |
| C2：一次调用的双向约束 | 后方实参可帮助前方 lambda；事实仍受作用域、阶段、分支和候选边界限制 |
| C3：不可重放的执行 | 候选试探仅做可隔离工作；不可重放的 comptime 执行等待选择确定 |
| Multi 属性 | 每个语法列表元素可有递归前缀；不等于属性附着整个列表，也不把所有 token* 变成属性入口 |
| Expr/pattern 上下文 | 外层块的边界传入 matches 模式；括号与封闭内部范围另建上下文 |
| Take view | 每种操作都有专用 HIR 种类；不在 HIR 继续以普通字符串投影表示已识别操作 |
| 两层类型 | 不可变 TypeCore 与 TypeRef(core, substitution)；环境取得另行保存 |
| IIR 类型完成 | 所有类型 comptime 工作完成、洞均有合法替换；允许 runtime binder 和合法范式项 |
| 高级 IIR | 保留控制流、闭包、模式、效应、资源关系，不强制先转 SSA |

内部结构名称、伪代码及 pass 组织用于说明可机械执行的算法，不是新增标准库 API 或源语法。共享 core 不能代替完整类型相等；substitution 不等于环境在任意使用点都可取得。范式不展开、PTC 只处理类型命题、不同错误／效应 ABI 等既有规则完整继承。

## 内容规划与交付

沿 077 的规划编写总览及主体；作者新增的两层类型表示单列一篇，共九篇：

1. [编译流水线](../docs/compilation/phases.md)：阶段产物、相互请求、两阶段检查与 IIR 入口。
2. [源码准备、解析与 HIR](../docs/compilation/syntax-and-hir.md)：工程输入、multi 属性、上下文边界及专用 view 节点。
3. [Elaboration 与上下文检查](../docs/compilation/elaboration.md)：双向信息、新鲜变量、刚性参数与统一调用计划。
4. [局部依赖与暂停](../docs/compilation/local-dependencies.md)：goal 边界、版本票据、代表元合并、去重、失败和取消。
5. [候选求解与提交](../docs/compilation/candidate-solving.md)：完整索引、隔离试探、PTC Unknown 与执行边界。
6. [两阶段计算与冻结](../docs/compilation/staging-and-freezing.md)：interpretation、invocation、稳定残余与独立实例状态。
7. [运行时依赖类型的表示](../docs/compilation/runtime-dependent-types.md)：带洞结构、替换组合、相等、缓存及环境交付。
8. [增量查询与诊断](../docs/compilation/incremental-queries.md)：三种图、查询粒度、失效、循环与诊断恢复。
9. [IIR 与代码交付](../docs/compilation/iir-and-codegen.md)：高级实例、后续转换契约及目标交付门槛。

同步 grammar/syntax、grammar/index、comptime/attribute、comptime/normalization-and-instantiation、type/runtime-dependent-type、type/substitution-inference、functions/parameter-elaboration 与 docs/README.md。正式文档不指向编译器具体文件、函数或 crate。

## 独立来源核对

根代理唯一写入。两个只读任务分别核对 parser 和依赖类型材料，交付源码与正式文档锚点；根代理合并并读回关键来源。已知范围明确，因此未创建 Graphify 全库索引。

Parser 来源：compiler/parse/src/basic.rs:30、142 的属性链与逐元素包装；expressions.rs:794、1209 的外层块限制及向 matches 传播；patterns.rs:349 的模式扩展应用边界。AST 的通用 TakeView 仅说明解析形态，作者明确要求后续每种 view 都直接进入专用 HIR kind。

运行时依赖来源：runtime-dependent-type 的依赖输入、本体/环境及编译器引导；substitution-inference 的身份、取得方式和无捕获替换；type-identity 的语义关系；reflection-and-layout 的值相关大小与表示依赖；作者新增两层表示要求。没有从现有后续编译阶段反推语言规则。

跨语言与 IR 对照仅使用各章直接链接的 Rust compiler guide、Zig 文档、C++ 草案、GCC internals 和 LLVM 语言参考，于本轮查阅；不宣称 Flurry 架构已有性能实测。

## Lex/parse 复用时需核验的实现差异

这些是历史代码与已确认规则的差异，不写成新语言特例，也未在本轮修改编译器：

- patterns.rs:747 将括号模式统一建为 TuplePattern；现行 grammar 已区分分组与单元素元组。
- expressions.rs:231、244 对元组首元素与后续元素使用不同入口；不能据此给 multi 属性规则添加首元素例外。
- basic.rs:16、operators.rs:71、expressions.rs:1265 的组合可能把属性后接的标识符目标吞入 LiteralExtension；复用 parser 时应验证属性边界。
- patterns.rs:355 的扩展模式内容比现行 grammar 窄；不能借旧实现缩减已允许的模式。

作者允许复用 lex/parse 的架构与可用实现；这些缺口按作者及现行 grammar 的契约验证即可，无需让作者重新确认已经确定的分组规则。

## 示例与验证边界

每篇按七段组织，Flurry 示例围绕具体问题：实参共同推断、produce 一次求值、属性链、条件/模式分组、精确实现、SelectType 的两个实例、Identity 保留、不同校准对象、缓存变化、循环、闭包、错误与效应。测试示例显式提供标准 prelude；教学定义、拒绝例和内部伪代码分别标明。

示例对照 grammar/syntax 和 grammar/index 自检；使用 where 子句、后缀投影、.ref / .*、and/or/not、标准分隔符和 Type 结果函数的 PascalCase 命名。没有把历史 parser 的接受行为当成新示例的语义证明。

本轮验证限于文件结构、引用、源证据、示例审阅和独立内容检查，不以尚未实现的 luna 编译结果裁定新文档。程序验证的高级算法继续延期；文档保留其必要交付边界。没有启动编译器实现、提交或推送。

## 问题状态

本轮没有需要作者重新拍板的语义。下一步可以依据章节中的不变量和验收案例组织编译器开发；本轮交付先完成已授权的文档。

## 最终验收

独立审阅者未参与编写，直接读取作者原答复、最终文件、现行语法及引用来源，并重新运行检查。适用的 13 项检查通过；C3 为并行写入隔离，本轮仅根代理写入，因此不适用。

| 检查项 | 证据类别 | 结果 |
|---|---|---|
| C1 依赖图、C2 任务边界、C7 冲突与修复、C8 预算 | record：过程记录与代理报告，属于记录性证据 | 通过 |
| C4 来源、C5 可检索锚点、C9 授权边界、C10 陈述准确性 | artifact：独立读回文件和来源 | 通过 |
| C6 检查重跑、C11 章节与示例、C12 作者约束、C13 同步、C14 归档与范围 | rerun：重新执行检查并对照最终材料 | 通过 |

审阅发现三个独立 `println` 示例依赖前一代码块的 prelude 导入：elaboration 的显式 implicit 示例，以及 IIR 的效应、错误示例。根代理已为三个代码块分别补充导入；独立审阅者读回确认，并重跑检查通过。没有遗留缺陷。

检查结果：九篇七段式章节，共 29 个 Flurry 代码块；章节结构、代码围栏、局部链接与锚点、术语、差异空白检查通过。原 prompt 答复完整归档，消费后的 prompt 为零字节。对本轮开始时 1,754 个已有文件的摘要比对确认未修改范围外文件；新文件核对仅包含八篇主体和本轮 history。

执行预算记录：参与者 4/4，最高并发 3/3，波次 2/3，重试 0/1，嵌套深度 1/1，worker turns 4/8，独立审阅轮数 1/2。这些数值用于约束本轮协作，不是编译器性能测量。

本轮检查锚点为 `/tmp/luna-080-check.py`，结果为 `/tmp/luna-080-check-result.json`，协作记录为 `/tmp/luna-080-graph.json`；这些临时文件不承担长期文档入口，本节保留验收结论。自动检查没有把文件级导入检查当作独立代码块完整性的证明；三个导入问题通过逐块审阅发现并修复。示例未运行 Flurry 编译，未运行 runtime 测试或性能基准。

## 对作者的完整交付回复

编译主题已完成：[总览与章节入口](../docs/compilation/phases.md)。共九篇、29 个 Flurry 代码块，覆盖递归属性、上下文敏感解析、take view 专用 HIR 节点、elaboration 与 comptime 交织、两层类型表示和保留高级结构的 IIR，并同步语法及相邻章节。

结构、链接和独立审阅已通过；示例未运行编译。

原答复已完整归档到 077，prompt 已清空，本轮说明与验收记录在 080。
