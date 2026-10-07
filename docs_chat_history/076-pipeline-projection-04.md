# Pipeline projection：正式文档交付

- 日期：2026-10-07
- global_index：076
- theme：pipeline-projection
- subindex：04
- 本轮目标：按 075 的作者答复完成本主题文档与语法同步，随后进入编译主题的审阅。

## 作者输入与授权

作者回复了 prompt，明确说“可以编写文档了”，并要求下一主题讨论编译，完成后即可开始开发；程序验证高级内容暂缓。作者提供了 /home/fin/vilya/examples/2026-10-7-elaboration-of-elaboration.md 作为编译主题材料。

075 的答复全文已原样追加到 [075](075-pipeline-projection-03.md)。P7 保留 A；P8 改为显式查询 build.package_metadata("cuda").features，并明确 build 中的内容默认指 host pipeline。这一修改覆盖此前关于隐式切换设备 build 上下文的建议。

## 已确认并落实

1. 投影声明依据签名、属性及必要的编译时输入，先向外围提供类型接口；完整代码生成兑现它。
2. CUDA kernel 不因为采用 fn 表面形式就取得普通宿主函数调用接口，启动、加载和完成协议属于库。
3. build 默认指 host pipeline；设备目标配置通过包元数据查询。Host pipeline 的目标不等于运行编译器的机器。
4. Helper 随实际依赖进入目标检查，无需重复添加投影属性。间接调用、递归和运行时服务由目标能力约束。
5. Intrinsic 开放操作数接口，需要 unsafe；紧接调用的 as 指定输出类型，缺省 void；外围类型注解不补写 as。
6. Void、结果未使用和 unsafe 均不证明无副作用，未知操作按保守行为保留。
7. IIR handle 供宿主后端访问 IR；void 回调通过任务交付产物和诊断，缓存须恢复实际产物。
8. 延后的 data family 不进入正式章节；PTC 与程序验证继续分层；范式头不展开。

## 文档交付

新增总览与四篇主体：

- [总览](../docs/pipeline-projection/index.md)：一个 kernel 的声明与启动代码，区分接口与代码生成。
- [投影声明与类型接口](../docs/pipeline-projection/declarations-and-interfaces.md)：Kernel 与参数关系、教学 LaunchConfig、包配置及普通调用拒绝例。
- [Intrinsic 与输出类型](../docs/pipeline-projection/intrinsics.md)：标量、屏障、连续操作、聚合结果、缺少 as 与后端拒绝例。
- [目标检查与汇编封装](../docs/pipeline-projection/target-validation-and-assembly.md)：FMA、向量 load/add/store、helper 闭包与保守优化。
- [宿主后端与产物交付](../docs/pipeline-projection/backends-and-artifacts.md)：回调、IR handle、宿主执行、失败、缓存和发布状态机。

同步 docs/README.md、docs/grammar/syntax、docs/grammar/index.md、docs/code-organization/builtin-packages.md 和 docs/compilation/phases.md。新产生式是既有 take view、application 与 as 的组合速查，不新增关键字。

## 示例边界与自检

按 grammar/syntax 及 grammar/index.md 检查属性、函数修饰、子句、指针、调用、take view、元组、error 传播、inline if、lambda 和分隔符。类型函数遵守 PascalCase，普通教学函数使用 snake_case。CUDA 接口已有 gridDim/blockDim 拼写明确标为库字段；教学 LaunchConfig 自己使用 snake_case。

Inst、WordInst 和本地配置类型明确标为教学定义。Vector 是由调用者显式提供的目标库类型，不杜撰标准库 SIMD 名。宿主回调沿用作者提供的诊断分支及 meta 接口，修复草稿缺失 fn 与 metacompilation 拼写；不增加产物注册 API。

已有文档与新草稿关于冻结结果的交界不在本主题私自裁决；进入 077 的 C1。没有用当前 luna 编译器判定新语言设计已经实现。

## 检查与协作边界

根代理唯一写入；两个只读代理分别完整阅读 elaboration 草稿的前后两部分，编译主题的分析与完整问题保存在 [077](077-compilation-01.md)。这两项阅读可与本主题撰写独立进行。

检查范围包括新章节七段结构、代码块及本地链接、旧 prompt 全文归档、下一轮 prompt 的完整备份，以及本轮修改是否超出授权文件。语义核对以作者答复、相关 proposal 和正式文档为依据；跨语言对照采用文中链接的官方资料。结构检查不等于 Flurry 编译或后端运行测试。

独立审阅及其修正结果将在本文件末尾追加；不以作者尚未审阅的新编译提案作为 pipeline 文档的前提。

## 下一主题

编译主题先讨论，不修改其未确认的正式语义。重点收束合法残余计算与临时状态、同一调用的推断可见性，以及候选试探期间的编译时执行。程序验证高级求解延期，编译器开发尚未启动。

## 最终检查与独立审阅（2026-10-07）

独立审阅读取最终文件和来源，复跑检查并报告全部适用项通过：13/13；C3 不适用，因为没有并发写入者。修复包括 history 中两处链接及两个打印示例缺少的 std.prelude 导入。之后独立读回并复跑，未遗留缺陷。

- 结构核对命令：`python3 /tmp/luna-docs-076-check.py`，退出 0，issues=[]。
- 最终产物：五篇 pipeline 文档，21 个 Flurry 代码块；各篇七段结构、代码围栏、本地链接及新章节锚点通过。
- 原始答复按字节完整存在于 075，当前 prompt 按字节完整存在于 077。
- `git diff --check -- docs/README.md docs/grammar/syntax docs/grammar/index.md docs/compilation/phases.md prompt` 无输出，退出 0。
- 开始时 tracked 文件的指纹对比显示本轮变化仅限授权路径；预先存在的编译器修改保持原样。既有 untracked 文件未由该指纹扫描覆盖；本轮写入清单另由工具操作与独立文件检查核对。
- 没有运行 Flurry 编译、后端执行或性能基准，不以结构检查代替语义实现验证。

审阅证据分级：C1（依赖图）、C2（任务边界）、C7（冲突及修复归属）、C8（上限）依据执行过程中追加的记录，属于过程陈述；C4–C6、C9–C14 通过文件、来源、仓库状态或命令复核。独立审阅同时检查候选与既定语义的区别，没有把预填答复视为作者批准。

实际协作规模为根代理、两个只读阅读任务及一个独立审阅任务；峰值并发 3，派发 2 批，深度 1，worker turns 4，无失败重试，一次正式审阅中修正并复核缺口。详细执行记录与检查输出分别在 /tmp/luna-docs-076-graph.json、/tmp/luna-docs-076-check-result.json；本段保留可持久阅读的结果摘要。


## 对作者的完整交付回复

[Pipeline projection 文档](/home/fin/luna/docs/pipeline-projection/index.md) 已完成：总览＋四篇主体，包含 21 个代码块，并同步 syntax、导航和 build 说明。

[新 prompt](/home/fin/luna/prompt) 已写入 elaboration 分析、章节规划和预填答复，集中讨论三点：

- 合法残余计算如何冻结，避免缓存临时推断状态。
- 后面的实参能否帮助前面的实参推断。
- 候选回退如何处理不可重放的 comptime 执行。

我的核心判断是：**推断规则决定可用信息，调度只决定何时重试。** 草稿的分层方向值得采用，下一步需要把这些边界定清楚。

原答复已归档，独立审阅与修正已完成，结构和链接检查通过；未将示例作为已编译程序验证。程序验证高级内容留到后续。
