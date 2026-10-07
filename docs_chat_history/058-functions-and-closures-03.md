# 函数与闭包：正式文档初版

- 日期：2026-09-27
- global_index：058
- theme：functions-and-closures
- subindex：03
- 本轮目标：按作者批准编写函数与闭包正文，同步相关定义，完成归档并清空 prompt。

## 作者输入与确认

作者本轮消息：“可以开始编写了。”当前 prompt 保留 G1–G6 的六项 A 预填，完整文件已追加到 057，并记录写入批准。替换前及清空前均逐字检查，没有覆盖新增作答。

本轮批准收束：MemoryEffect predicate 裸参数为 quote 简写；tactic lambda 参数表由目标接口提供；参数 elaboration 局限于自己的输入；默认值 P3 限制仅用于 runtime；用户 impl 的主体资格约束泛型匹配；dyn 构造自动保留所需闭包环境。

F1–F6 以及作者先前补充继续有效：实参源码次序、缺省输入依赖次序、默认按值捕获、可证明独立的最小字段、首次捕获与逆序清理、普通 fn 不擦除非空环境。quote fn 非 comptime fn，普通参数求值一次；contextual 使用 implicit quote。

## 内容规划与实现

沿一次调用组织正文：先定义与调用，再参数供应和阶段，再闭包创建、调用与退出，最后代码接入和模板。全部十篇保留七段骨架，Intro 和常见示例面向 C 背景读者；具体 IR 契约、算法步骤、失败模式和成本放在实现引导。

| 文件 | 行数 | Flurry 代码块 |
|---|---:|---:|
| [calls-and-returns.md](../docs/functions/calls-and-returns.md) | 207 | 5 |
| [closure-environments.md](../docs/functions/closure-environments.md) | 159 | 4 |
| [comptime_fn.md](../docs/functions/comptime_fn.md) | 174 | 6 |
| [higher-order-functions.md](../docs/functions/higher-order-functions.md) | 149 | 5 |
| [index.md](../docs/functions/index.md) | 104 | 2 |
| [inline-and-quote.md](../docs/functions/inline-and-quote.md) | 161 | 5 |
| [lambdas.md](../docs/functions/lambdas.md) | 167 | 6 |
| [parameter-elaboration.md](../docs/functions/parameter-elaboration.md) | 155 | 5 |
| [parameters.md](../docs/functions/parameters.md) | 201 | 7 |
| [templates.md](../docs/functions/templates.md) | 192 | 5 |

总计 1669 行、50 个 Flurry 代码块。拒绝例标明拒绝原因，不作为已编译通过的程序。例子覆盖实际输出、移动和逃逸错误，并使用完整的局部声明；没有以占位省略体替代主要教学程序。

### 核心交付

- 调用：具名实参换序、callee 先求值、方法指针访问、具名结果与参数准备中断。
- 参数：依赖默认值、P3 稳定状态读取、元组收集、匿名结构体属性收集及直接供应。
- Elaboration：post do、目标决定 tactic lambda 参数表、占位符边界、显式 implicit 输入、局部协议及 continuation 的有限承诺。
- 阶段：普通函数的 comptime 参数、外层参数化定义与 runtime 实例、Type 结果命名、trait bound 简写、MemoryEffect 参数简写。
- 捕获：数值副本与指针观察、非 Copy 创建时移出、独立字段、调用后责任和逆序 Drop。
- 高阶：普通 fn 的环境边界、C: Type.vars.Closure、环境保留转交、返回自有捕获和悬空反例。
- Staging：quote fn 普通实参一次求值、未使用实参仍执行、implicit quote、单洞和阶段边界。
- 模板：简单签名、quote lambda body 接入、独立实例、template test 唯一锚定子句 body、互斥函数级 case。没有编造开放规则冲突的额外优先级或未经确认的公共 trait API。

### 跨章节同步

除函数目录十篇外，同步十个文件：README、grammar/syntax、traits/declarations、implementation-resolution、dynamic-interfaces、derivation、type/qualified-types/closure-type、resource-management/memory-effects、comptime/purity-and-normal-form、quoting-and-splicing。

语法补足锚定 lambda 形参、必需默认表达式、四类命名供应、derive fn 参数、case 参数以及本章用到的 quote/template、contextual 类型、splice 和环境绑定片段。类型与 trait 部分保留裸 dyn 两 usize 的保证，完整 %C dyn 的环境交付另计；用户 impl 主体资格在泛型匹配后仍检查。

runtime 默认 P3 与 comptime 子句默认政策分别陈述。内存效果与可达状态由程序验证层处理，不作为 PTC 类型命题。没有修改编译器实现，也没有创建提交。

## 可行性评估与界限

参数来源、执行阶段和语法角色分别建模，保持实参一次求值及局部协议可组合性。捕获结构、真实环境值、主体行为及资源状态分别保存，避免以类型相同推导同一对象或可重复消耗。确定性展开不变成任意展开必终止的承诺。

本轮只将已经确认的边界写入正文。完整 CMTT 代码构造 API、open code 规则集合的重叠政策和参数 elaboration trait 的具体公共项不因编写函数章被额外发明；函数级示例使用互斥且覆盖的模式。正式正文不包含审阅状态或 history 指针。

## 问题列表与确认状态

- 本轮无新增拍板问题；用户已明确授权写入。
- 已确认并落实：F1–F6、G1–G6、作者补充的参数角色、特殊类型实现范围及 dyn 表示。
- 仍由相应专门主题定义：完整 CMTT 与开放规则 API，不影响这里已确认的调用、捕获和模板接入边界。

## 校验

- 十篇逐一检查七段结构；共 50 个 Flurry 代码块，全部包含实际代码。
- 对照 grammar/style 检查后缀指针、Tuple、参数与列表分隔、谓词和命名；上下文及模板采用本主题确认的局部语法。
- 检查本轮 20 个文档的本地链接、代码围栏与示例定界；未发现断链。
- git diff --check 通过。未把当前编译器当作语义权威，未声称 Flurry 示例经过编译器验证。
- 跨语言比较查阅 Rust Reference、C++ 工作草案、Zig 官方文档与 C11 N1570；支持性链接就近放在正文。
- prompt 原文已在 057 完整保存，归档核验后清空；本记录保存完整最终回复。

## 本轮最终回复

函数与闭包初版已完成：[阅读入口](/home/fin/luna/docs/functions/index.md)。共 **10 篇、50 个 Flurry 代码块**，覆盖调用、参数、捕获、环境、高阶接口、quote fn 和 template。

已同步 grammar、trait 实现限制、dyn 表示及相关类型说明，并检查章节结构、示例风格和链接；未做编译器验证。

本轮已归档，`prompt` 已清空。
