# 函数与闭包：重组正文与补全用例

- 日期：2026-09-28
- global_index：065
- theme：functions-and-closures
- subindex：04
- 本轮目标：按作者要求重写函数与闭包章节，补足特性说明、组合用例、拒绝情形与实现引导。

## 作者输入

> 函数章节内容太单薄了，很多特性不仅没讲清楚，用例更是没有，回去重写！

这是明确的正式文档重写授权。本轮不再发起设计问卷，不要求作者重新批准已经确认的语义。

## 评估与内容规划

原十篇正文虽然有代码块，但不少特性只有一句边界说明或一个极简例子。主要问题是：

1. 参数分类与实际调用没有形成完整路线，连续默认依赖、命名供应冲突、嵌套 lambda 边界及编译时收集说明不足。
2. 闭包的创建、调用、转交和清理分别出现，却缺少把这些时刻连起来的程序；高阶组合及剩余捕获状态几乎没有展开。
3. 函数契约分散在资源章节，函数读者无法在本主题中完整看到前提、结果、效果、错误与验证的交接。
4. 模板和函数级模式挤在一篇，签名复用、正文接入、测试初始化和 quoted arm 的不同作用没有形成独立教学顺序。
5. quote fn 主要围绕简单加法，缺少上下文自动接入、真实别名和局部清理的完整例子。

重写按“接口 → 输入供应 → 阶段 → 契约 → 捕获 → 环境状态 → 高阶交付 → 代码接入 → 模板 → 模式分派”组织。保留原有有效例子，改写总览，重组各篇定义和实现引导，新增函数契约与函数级模式两篇。没有把增加行数作为唯一完成标准。

## 已确认依据

- F1–F6：callee／接收者先求值、显式实参按源码次序各一次、前序默认依赖、默认按值捕获、最小独立位置、首次捕获与逆序清理、普通 fn 的环境边界。
- G1–G6 及作者补充：参数角色与阶段分别记录；MemoryEffect predicate 裸参数简写 quote；tactic lambda 由目标接口决定完整参数表；参数协议局部化；runtime 默认值 P3 与合法入口下有限完成；特殊主体的用户 impl 限制；环境交付。
- quote fn 不属于 comptime fn，普通实参求值一次，contextual 参数使用 implicit quote；模板支持非 comptime fn；quoted arm 通过 quote fn 与函数级模式表达。
- 函数 ABI 使用属性；capability 由 trait 协议与 elaboration 实现。
- normal form 不展开：Identity<T> 与 T 在 normalized IR 中不因函数体建立关系。alias 只消解到范式项。
- safe 与 verified 的检查载体、程序规格与 PTC 的职责继续分别保持。

没有采用当前编译器作为语义权威。历史 proposal 用于理解用途；其中未确认的公开协议项、open code 属性 API 和更强的 continuation 操作没有被直接升格为正式承诺。

## 正文交付

| 文件 | 行数 | Flurry 代码块 | 补足的主要内容 |
|---|---:|---:|---|
| functions/index.md | 144 | 2 | 重写阅读路线、四个接口维度及端到端报价用例 |
| functions/calls-and-returns.md | 345 | 10 | 表达式主体、ABI 边界、接收者后续变化、准备失败清理和返回责任 |
| functions/parameters.md | 306 | 12 | 分页连续默认值、异型收集、属性包、直接供应与重复／错误命名 |
| functions/parameter-elaboration.md | 254 | 11 | 非末尾 lambda 路由、implicit 阶段转发、嵌套 tactic lambda、越界拒绝及 catch 接口 |
| functions/comptime_fn.md | 349 | 14 | 参数阶段地图、依赖数组、编译时 optional／收集、两种 Identity、多个 trait bound 和 quote 规格 |
| functions/contracts-and-effects.md | 257 | 7 | 新篇：前后条件、outcomes、受检除法、错误传播、效应请求、pure/spec/verified |
| functions/lambdas.md | 264 | 10 | 三个时刻、捕获路径、显式克隆、嵌套快照、类型简写与显式结果 |
| functions/closure-environments.md | 247 | 8 | 调用状态表、可重复修改、重复消耗拒绝、部分捕获消费与剩余清理 |
| functions/higher-order-functions.md | 264 | 9 | runtime 选择函数、完整环境转交、组合后返回、同型不同值与延后执行 |
| functions/inline-and-quote.md | 275 | 9 | inline 对照、接入点计数、模板上下文自动供应、真实别名和局部 Drop |
| functions/templates.md | 297 | 9 | 签名契约复用、实例与调用的区别、提前失败、私有上下文拒绝、测试独立初始化和清理 |
| functions/pattern-functions.md | 224 | 5 | 新篇：命令分派、多参数组合、分支绑定、quote fn 模式及 quoted arm 分工 |

合计十二篇、3226 行、106 个 Flurry 代码块。计数包含少量用于说明接口或拒绝原因的代码块，不把这些声明或反例宣称为独立可运行测试。

多数新增例子提供完整局部定义和 test，随后逐项说明输出、责任或拒绝原因；接续前文定义的代码明确说明依赖。两份 Identity 例子明确为独立定义，普通执行返回 u32，范式例子只按原类型转交输入，不再进行无依据的数值初始化。

同时更新 `docs/README.md` 的函数阅读入口。全局 syntax 未新增产生式：本轮使用现有表面形式，局部语法引用对齐全局名称。

## 实现引导与抽象边界

- 调用准备记录输入来源、阶段、责任和失败后继，区分准备失败与主体退出。
- 参数供应用状态表表示唯一来源，源码顺序和声明依赖分别维护。
- 捕获分析保存有序路径、独立性与资源关系；调用更新环境字段状态；清理只处理剩余责任。
- 高阶组合保留两份完整环境，分别组合调用前提、结果依赖和出口状态。
- quote 接入保持一次求值、对象 identity 和逻辑退出清理；模板将共同主体与用户正文交给同一份控制流检查。
- 函数级规则按所属接口与 package 收集，成功绑定和主体一起接入；没有编造新的文件顺序优先级。
- 具体参数 elaboration trait 的公共 API 和 continuation 内建操作不由示例补造；catch 用真实接口声明说明绑定角色。

## 检查

1. 十二篇均保持七段骨架，新增实现引导不指向编译器具体源码文件或函数。
2. 对照 grammar/syntax 与 grammar/index.md 核对新示例：投影、指针取址／解引用、参数前缀、属性、模式、模板、命名及语句分隔。
3. 全部 106 个代码块通过结构检查：括号与围栏配对，无大括号后错误分号，无行尾空白。
4. 本主题所有本地链接与锚点检查通过；README 新入口对应实际文件。
5. `git diff --check` 通过。搜索确认本主题没有引回函数签名 handles、extern 或淘汰的 contextual 属性拼写。
6. 核对 Rust 官方 Reference、C++ 标准草案及 Zig 官方文档的相关对照；函数级模式篇附 Rust match 与 Zig switch 的来源。
7. 本轮文件摘要显示，既有文件只变更函数目录十篇和 README，另新增两篇函数正文；编译器未被修改，未创建提交。
8. 输出与资源次序是依确认语义进行的源码推演，未运行当前编译器来宣称示例通过。
9. prompt 始终为空。本轮没有 prompt 答复要搬运，也没有新的问卷。

## 问题与确认状态

- 已确认：本轮写入的语义均承接上述已确认规则与最近更正。
- 新增拍板问题：无。
- 没有为补用例而自行定义公开 elaboration trait、quote arm 关键字、任意 continuation API 或函数签名 capability 段。

## 对作者的完整回复

已重写[函数与闭包章节](/home/fin/luna/docs/functions/index.md)，扩展为十二篇，补齐参数组合、捕获与清理、高阶组合、quote fn 和模板的完整用例及拒绝情形。

新增[函数契约与效果](/home/fin/luna/docs/functions/contracts-and-effects.md)和[函数级模式与分散定义](/home/fin/luna/docs/functions/pattern-functions.md)，同步更新阅读路线。

结构、链接和示例语法自检已完成；示例未用编译器验证。本轮已归档，prompt 保持为空。
