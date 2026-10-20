# Flurry 语言文档

Flurry 是一门面向系统级编程的语言，在正确性、性能、抽象能力、开发体验与语义健全性之间寻求统一。本文档集从八个核心维度组织 Flurry 的设计与用法；各维度下的详细内容仍在逐步完善中。

→ [语言愿景与定位](introduction.md)

---

## 基础语言

| 文档 | 内容 |
|------|------|
| [词法结构](basics/lexical.md) | 标识符、关键字、运算符空格规则、注释、分号省略 |
| [字面量](basics/literals.md) | 整数、浮点、字符、字符串、布尔、符号；后缀与 comptime 类型 |
| [单元 / null / undefined](basics/unit-null-undefined.md) | `()`、`null`、`_`、`undefined`（非字面量） |
| [基本数据类型](basics/primitives.md) | 整数、浮点、bool、char、void、指针、编译期类型 |
| [符号](basics/symbols.md) | 入口重定向 → 字面量章「符号」 |
| [变量绑定](resource-management/bindings.md) | `let` / `var`、覆盖赋值、失败状态与遮蔽 |
| [控制流](control-flow/index.md) | 总览：基本控制流与模式匹配 |
| [基本控制流](control-flow/basic.md) | `if`/`when`、`for`/`while`、标签、`break`/`continue`/`return` |
| [模式匹配](control-flow/pattern-matching/index.md) | 模式语法、`match`/`if is`/`while is`、穷尽性 |
| [位向量模式](control-flow/pattern-matching/bitvec.md) | `BitVec` 分段匹配、base type、跨元素 |
| [函数与闭包](functions/index.md) | 调用、参数、阶段、捕获、高阶接口、quote fn 与 template |
| [调用与返回](functions/calls-and-returns.md) | 源码求值次序、方法、失败准备与结果交付 |
| [参数与默认值](functions/parameters.md) | 命名供应、元组与属性收集、默认纯度 |
| [参数 elaboration](functions/parameter-elaboration.md) | implicit、post do、tactic lambda 与协议边界 |
| [参数阶段与子句](functions/comptime_fn.md) | 子句依赖、comptime 收集、quote 输入与范式保留 |
| [函数契约与效果](functions/contracts-and-effects.md) | 前后条件、行为上界、错误／效应与验证边界 |
| [Lambda 与捕获](functions/lambdas.md) | 按值捕获、指针观察、最小独立位置 |
| [闭包环境](functions/closure-environments.md) | 调用状态、逆序清理与逃逸 |
| [高阶接口](functions/higher-order-functions.md) | `%c fn`、环境传递与回调契约 |
| [Inline 与 quote fn](functions/inline-and-quote.md) | 确定性接入、上下文替换与实参一次求值 |
| [函数模板与测试模板](functions/templates.md) | 签名复用、正文接入、初始化和清理 |
| [函数级模式](functions/pattern-functions.md) | case 参数、多输入分派与 quoted arm |
| [Trait 与 implementation](traits/index.md) | 接口、关联项、结构精度、归属与动态交付 |
| [实现选择](traits/implementation-resolution.md) | 结构索引、未决候选与一致性 |
| [替换与依赖交付](type/substitution-inference.md) | 开放实现绑定、环境取得与传递 |
| [派生与策略](traits/derivation.md) | 具名/default deriver、产物检查 |
| [动态接口](traits/dynamic-interfaces.md) | 隐藏主体、Self 结果与依赖环境 |
| [代码组织](code-organization/index.md) | 包、umbrella、模块、名称与构建的完整阅读路线 |
| [源码与模块](code-organization/modules-and-sources.md) | 目录扫描、文件映射、mod.fl 与模块属性 |
| [路径与遮蔽](code-organization/scope-and-paths.md) | 统一命名空间、父静态容器、包根与顺序绑定 |
| [导入与再导出](code-organization/imports-and-reexports.md) | 通配、别名、来源身份、循环与歧义 |
| [可见性](code-organization/visibility.md) | 默认包内、private、pub 与公开入口 |
| [内置包](code-organization/builtin-packages.md) | 六个内置包、prelude、属性与 build 上下文 |
| [配置与脚本](code-organization/configuration-and-scripts.md) | comptime 配置、anar 调度、宏准备与生成源码 |

---

## 八个核心维度

### 1. 编译时计算（Comptime）

编译时计算确定值、类型和代码结构。normalization 依据操作契约检查定义；instantiation 代入具体输入，继续求值与检查。

| 文档 | 内容 |
|------|------|
| [编译时计算总览](comptime/index.md) | 心智模型与阅读路线 |
| [Normalization 与 instantiation](comptime/normalization-and-instantiation.md) | ordinary/loose、PTC、defer、query 图与执行器 |
| [Comptime 函数、纯性与范式](comptime/purity-and-normal-form.md) | 编译时函数、子句参数、默认值、pure 与缓存 |
| [Comptime 块与 inline 控制流](comptime/inlining-comptime-code.md) | 分支检查、编译时选择、展开与 runtime 主体 |
| [跨阶段值传递](comptime/quoting-and-splicing.md) | Splice、proxy、quote/lift 边界 |
| [Trait 两相](comptime/the-comptime-phases-of-traits.md) | phase 归属与执行阶段、共享 coherence、Eq/Hash 与 Splice |
| [静态形状的张量加法](comptime/static-shaped-tensors.md) | 形状广播与分阶段执行案例 |
| [属性系统](comptime/attribute.md) | 编译时元数据 |
| [编译流水线](compilation/phases.md) | 阶段输入/输出与实现边界 |

### 2. 类型系统

类型值、操作依据、数据表示与运行时依赖分别解释；从[类型系统总览](type/index.md)进入完整阅读路线。

| 文档 | 内容 |
|------|------|
| [类型值与身份](type/type-identity.md) | 两层、声明身份、工厂与类型关系 |
| [参数化与 Any](type/comptime-gradual-dependent-type.md) | 上下文检查、分支汇合与实例交付 |
| [PTC](type/propositional-constraints/index.md) | 19 类原子、11 个求解模块、组合与诊断 |
| [成员与适配](type/receiver-adaptation.md) | 投影来源、接收者与隐式转换 |
| [数据声明](data-structures/index.md) | struct、enum、union、newtype 与 alias |
| [错误与效应行](type/rows.md) | 唯一标签、scoped labels、统一与路由 |
| [Handler](type/handlers.md) | 错误处理、直接返回、deep 恢复与转发 |
| [依赖与环境](type/runtime-dependent-type.md) | NodeIndex、共享环境、闭包限定 |

### 3. 性能与零开销抽象

从[资源管理总览](resource-management/index.md)进入消耗、复制、引用有效性、清理和纯度的完整阅读路线。静态证据用于消除不必要的检查与抽象成本，优化保持程序的可观察行为。

| 文档 | 内容 |
|------|------|
| [Once / Clone / Copy](resource-management/once-clone-copy.md) | 消耗、显式克隆、表示复制与责任 |
| [移动与初始化](resource-management/moves-and-initialization.md) | 部分移出、重新初始化、自引用与结果交付 |
| [自动清理](resource-management/drop-elaboration.md) | 局部及聚合清理次序、失败路径、Drop 环境 |
| [临时值与丢弃](resource-management/temporaries.md) | 完整表达式、视图保活、通配模式 |
| [指针](resource-management/pointers.md) | 字段指针、顺序别名与访问资格 |
| [可达依赖](resource-management/reachability.md) | 目标身份、世代、内容事实与逃逸 |
| [内存视图](resource-management/memory-views.md) | 字段、区间、快照与分离证据 |
| [内存效果](resource-management/memory-effects.md) | 六种效果、行为上界与出口状态 |
| [契约与验证](resource-management/contracts-and-verification.md) | 默认推导、受检 assumes、safe / verified 接口 |
| [运行时纯度](resource-management/runtime-purity.md) | 六层保证、责任与优化前提 |

### 4. 元编程

模板宏替换 token；构造宏处理结构化输入；CMTT 在显式 runtime 上下文中组合带类型的表达式、模式与 arm 代码。

| 文档 | 内容 |
|------|------|
| [CMTT 总览](cmtt/index.md) | 两阶段、rank-1、三类代码与阅读路线 |
| [上下文与替换](cmtt/contexts-and-substitution.md) | 默认绑定、显式映射、身份与依赖 |
| [模式与 quoted arm](cmtt/patterns-and-arms.md) | 成功上下文与规则接合 |
| [Any 与代码见证](cmtt/any-and-code-evidence.md) | 通配描述、阶段与实际接入依据 |
| [组装有限程序](cmtt/program-construction.md) | 单洞组合与自动机生成示例 |
| [模板宏](macro/template-macro.md) | 文件内捕获、替换与按组展开 |
| [构造宏](macro/construction-macro.md) | AST、PEG、token 与原始文本输入 |
| [属性系统](comptime/attribute.md) | 属性 Object、configuration 与读取 |

### 5. 代数效应

效应请求、处理能力、限定 continuation 和异步工作分别建立控制契约；错误结果保留独立的表示与传播 ABI。

| 文档 | 内容 |
|------|------|
| [代数效应总览](algebraic-effect-and-delimited-continuation/index.md) | 完整阅读路线、操作与结果边界 |
| [操作与处理](algebraic-effect-and-delimited-continuation/algebraic-effect.md) | 操作组、嵌套、转发与结果契约 |
| [处理能力](algebraic-effect-and-delimited-continuation/handling-capability.md) | Trait、环境与 .use 接入 |
| [错误与 catch](algebraic-effect-and-delimited-continuation/error-handling.md) | 前后处理、结果交付与重试 |
| [直接传值](algebraic-effect-and-delimited-continuation/direct-passing.md) | 安装时求值、隐式供应与覆盖 |
| [限定 continuation](algebraic-effect-and-delimited-continuation/multi-prompt-delimited-continuation.md) | 单次、尾恢复、逃逸与清理 |
| [异步效应](algebraic-effect-and-delimited-continuation/async-algebraic-effect.md) | Future、runtime 与结构化退出 |
| [效应 ABI](algebraic-effect-and-delimited-continuation/effect-abi.md) | 路由、池化后端、中止与跨语言边界 |

→ [concurrency/](concurrency/)（含设计备忘 `prompt.fl`）

### 6. 安全等级（safe · `unsafe` · `verified`）

| 文档 | 内容 |
|------|------|
| [安全等级](program-verification/safety.md) | 三个等级的语义、调用规则、安全哲学 |

### 7. 流水线映射

投影属性让源码声明接入目标编译管线；类型接口、intrinsic 操作和后端产物分别承担检查与交付责任。

| 文档 | 内容 |
|------|------|
| [Pipeline projection 总览](pipeline-projection/index.md) | Kernel 声明、显式启动与阅读路线 |
| [投影声明与类型接口](pipeline-projection/declarations-and-interfaces.md) | 先行接口、参数关系、host 与包配置 |
| [Intrinsic 与输出类型](pipeline-projection/intrinsics.md) | 开放输入、as 输出、void 与拒绝例 |
| [目标检查与汇编封装](pipeline-projection/target-validation-and-assembly.md) | Helper 闭包、向量数据路径、目标约束与优化 |
| [宿主后端与产物交付](pipeline-projection/backends-and-artifacts.md) | IIR handle、宿主编译器、诊断与缓存恢复 |

通用的阶段输入、检查与实例化边界见[编译流水线](compilation/phases.md)。

### 8. 模式匹配

模式支持结构解构、位向量与自定义 pattern view；组合范围由模式优先级和分组括号确定。

→ [control-flow/pattern-matching/index.md](control-flow/pattern-matching/index.md)

---

## 编译

Elaboration 与 comptime 共同推进检查和实例化；阶段产物保留各自需要的类型、依赖与高级结构。

| 文档 | 内容 |
|------|------|
| [编译流水线](compilation/phases.md) | 阶段产物、elaboration 与 comptime 交织、IIR 边界 |
| [源码准备、解析与 HIR](compilation/syntax-and-hir.md) | Multi 属性链、表达式／模式边界、take view 专用节点 |
| [Elaboration 与上下文检查](compilation/elaboration.md) | 兄弟参数信息、刚性参数与调用计划 |
| [局部依赖与暂停](compilation/local-dependencies.md) | Goal、等待票据、唤醒、失败与取消 |
| [候选求解与提交](compilation/candidate-solving.md) | 完整索引、隔离试探、PTC 与执行边界 |
| [两阶段计算与冻结](compilation/staging-and-freezing.md) | Interpretation、invocation、残余与实例状态 |
| [运行时依赖类型的表示](compilation/runtime-dependent-types.md) | 带洞类型结构、substitution、identity 与环境 |
| [增量查询与诊断](compilation/incremental-queries.md) | 查询粒度、失效、循环、结果与诊断恢复 |
| [IIR 与代码交付](compilation/iir-and-codegen.md) | 高级操作、类型完成条件、后续转换与目标交付 |

## 程序合成

| 文档 | 内容 |
|------|------|
| [BitVec 代数与程序合成](program-synthesis/bitvec-algebra.md) | 定宽值、切片与回写、转换、基准降码与等价优化 |

## 参考

- [grammar/syntax](grammar/syntax) — 语法定义草稿
- 各目录下的 `prompt.fl` — 设计备忘与示例片段（非正式规范）
- [docs/OLD-2024/](OLD-2024/SUMMARY.md) — 2024 年旧版设计文档（供参考，部分内容已过时）
