# 028 — comptime — 07

- 日期：2026-09-05
- global_index：028
- theme：comptime
- subindex：07
- 本轮目标：按作者授权规划并编写 comptime 初版正式文档。

## 作者本轮输入

> 好的，可以开始编写了，记得做好内容规划

作者明确授权写入正式文档。本轮没有新的 prompt 答复；上一轮审阅全文已存于 026，027 中的建议没有因本次写作授权逐项自动确认为语言规则。

## 内容规划

| 文件 | 本章拥有的内容 | 避免承担的内容 |
|---|---|---|
| docs/comptime/index.md | 入口心智模型、阅读路线、comptime/runtime 与两个编译过程的区别 | 不在 Intro 堆特性，不重复各章完整算法 |
| docs/comptime/normalization-and-instantiation.md | ordinary/loose、PTC 操作契约、defer、assumes、开放定义；query 图及 interpretation/execution 算法 | 不给出完整 PTC 逻辑或模块可见性理论 |
| docs/comptime/purity-and-normal-form.md | comptime 函数、pure 承诺、范式头、子句参数默认值；执行与缓存边界 | 不创造副作用 capability 或 implicit 注入顺序 |
| docs/comptime/inlining-comptime-code.md | comptime block、inline 分支/循环、命题提取与运行时代码生成 | 不将 inline 写成全体代码执行于编译期 |
| docs/comptime/quoting-and-splicing.md | 普通值跨界、Splice runtime 方法、proxy；quote/lift 最小边界 | 不设计 CMTT 语法、引用序列化或自动派生优先级 |
| docs/comptime/the-comptime-phases-of-traits.md | trait 两相、按阶段使用、共享 coherence、有限 Splice 派生 | 不造重载解析或完整 trait 框架 |
| docs/comptime/static-shaped-tensors.md | 用户给出的三组调用、形状广播、分阶段执行、教学算法与实现不变量 | 不发布零阶 coherence 等尚未闭合的完整库 implementation |
| docs/compilation/phases.md | 流水线与阶段输入/输出契约，指回主章 | 不把一次 pass 固定成用户语义 |

各主题正文采用 Intro、语法、正式定义、常见示例、跨语言差异、运行时引导、编译器引导七段。总览的阅读路线放在 Intro 之外。既有 attribute.md 属独立属性主题，本轮保持原文。

相关入口整理：更新 docs/README.md 的 comptime 导航；将 docs/type/boostrap.md 的重复 comptime 叙述整理为简短类型入口，按作者已经明确纠正的无用户公理、implicit、Splice 普通 runtime 方法等结论消除相反表述；按实际需要修正基础章中的跨阶段引用与实现引导。独立类型、宏、内存管理与 OLD-2024 不展开。

## 确认边界与编辑选择

- 已确认：此前作者原文及最新审阅中的 comptime/runtime、normalization/instantiation、ordinary/loose、操作 requirement/结果命题、可执行 PTC assumes、无用户逻辑公理、pure 信任、两种执行载体、Splice 的 runtime 方法语义、trait 两相、命名限制和静态作用域基础。
- 待确认：027 的零阶实现排除、关联类型/const 的 defer 位置例外、implementation block 命题对成员的确切传播范围、自定义/自动 Splice 优先级等。本轮不制定这些规则。
- 教学例范围：采用作者已提供的 Tensor 调用与正维度形状数学算法；完整库实现的内存、索引与 coherence 契约不伪装成已解决。数学伪代码明确不是 Flurry 表面语法。
- grammar：只采用全局已有或作者明确确认的局部产生式；不为呈现教学例而补造全局 predicate/列表 rest/关联约束语法。
- 与其它语言的差异仅依据官方文档和标准草案，引用就近落在各章对比条目。C 的比较明确标注 C11，避免泛称所有 C 版本。

## 外部核对资料

Rust Reference 的 constant evaluation、generic parameters、trait bounds、traits；Zig 官方语言参考的 comptime/inline for；C++ 工作草案的 constant expressions、constexpr、if constexpr、constraints、parameter packs；WG14 N1570 的常量表达式/条件预处理/静态断言。最终引用以各文档中的直接链接为准。

## 执行与验证

已完成，实际交付与验证如下。


## 实际交付

- `docs/README.md`
- `docs/basics/literals.md`
- `docs/basics/primitives.md`
- `docs/compilation/phases.md`
- `docs/comptime/index.md`
- `docs/comptime/inlining-comptime-code.md`
- `docs/comptime/normalization-and-instantiation.md`
- `docs/comptime/purity-and-normal-form.md`
- `docs/comptime/quoting-and-splicing.md`
- `docs/comptime/static-shaped-tensors.md`
- `docs/comptime/the-comptime-phases-of-traits.md`
- `docs/type/boostrap.md`

共 12 个正式文档文件，其中 7 个 comptime 主题章全部具有约定的七段结构。保留 attribute.md、prompt、全局 grammar、编译器源码及 OLD-2024 原状。

1. 总览入口提供分阶段心智模型和顺序阅读表，避免在 Intro 堆列特性。
2. 两阶段章将 PTC requirement/结果命题及可执行 assumes 放在用户规则中，把 query 图、状态、执行载体、缓存和复杂度放在编译器引导中。
3. 纯性章区分 comptime 函数、普通函数的 comptime 参数、pure 信任与 PTC 逻辑，不引入宿主能力体系。
4. Inline 章区分分支检查、选择、循环展开与主体中的 runtime 操作。
5. Splice 明确为普通 runtime 方法实例；基础字面量的运行时引导相应去除“一律物化只读常量”的推论。
6. Trait 章以不同名称的教学方法展示两个阶段，不恢复此前错误的重载解析模型。
7. Tensor 章保留作者三组调用，以数学形状扫描和 runtime 投影伪代码说明实现；限制案例算法的输入为正维度，未声称完整 Tensor implementation 已闭合。
8. type/boostrap.md 改为类型与 comptime 的阅读入口；删去的相反论述均涉及作者已明确纠正的用户公理、纯性、implicit 默认值和无条件零开销 splice。没有据此改写独立类型主题。

## 语法与语义自检

- 19 个 Flurry 代码块逐一按 grammar/syntax、grammar/index.md 及作者已经确认的局部形式复核。
- 本轮正式局部产生式采用作者已确认的 defer、assumes、inline for 和 comptime predicate；全局 grammar 的未接线项没有擅自补入。正文没有使用作者 Tensor 实现中尚未接合的列表 rest 或关联类型参数约束语法。
- 教学函数均就地定义；Tensor 的 random/zeros 来自作者示例，Sample 是明确说明的教学 trait；打印使用 println。
- 核对值表达式后缀、类型表达式/路径、注释、中缀空白、布尔关键字、名称、逗号/分号与大括号分号省略；未混用 Rust/C 的代码形式。
- 新写正式章中无协作状态、chat history 指针、待定草案标记、废止概念对照或具体编译器源码路径。
- 自动派生优先级、完整零阶 implementation coherence、implementation block 事实外传及尚未确认的 defer 位置例外均未制定新规则。
- 这不是完整 PTC 求解理论、CMTT 语法或 Tensor 内存契约的规范；这些独立主题的细节没有以模糊承诺填入。

## 验证结果

- 7 章的七段标题顺序全部通过检查。
- 本轮修改文件中的所有本地链接目标及章节锚点通过检查。
- Markdown 代码围栏、Flurry 示例括号配对和针对性语法/风格静态检查通过。
- 本轮 12 个文件的 git diff --check 通过。全 docs 检查曾遇到未修改 attribute.md 的既有行尾空白，后将检查范围限定为本轮实际文件；未为消除既有告警改动属性内容。
- 对新正文中的形状扫描算法作独立数学核对：阶数 0–3、维度 1–3 共 1600 对输入，其中 940 对相容。核对相容性、结果阶数、递归子形状、严格降阶、作者三例及交换操作数后的结果。
- 没有运行 luna 编译器来宣布示例可编译；现有实现不是此语言文档的语义权威。数学检查不等于 PTC 求解器证明或完整 Tensor runtime 测试。
- 通过编写前后的文件摘要确认实际正式文档变更恰为上述 12 个路径，未改其他 docs 文件。

## 对作者的完整回复

初版已写入，从 [comptime 总览](/home/fin/luna/docs/comptime/index.md) 开始阅读。

按七章组织：总览、两阶段检查、纯性与范式、inline 控制流、跨阶段值传递、trait 两相、张量案例。各章采用约定的七段结构，并同步整理了导航和重复总述。

已检查链接、示例语法风格和形状算法；未将现有编译器作为语义验证依据。尚未拍板的 Tensor 实现细节没有固化进正文。

内容规划与验证记录见 [028 记录](/home/fin/luna/docs_chat_history/028-comptime-07.md)。
