# 类型系统：补齐真实源码示例

- 日期：2026-09-13
- global_index：045
- theme：type-system
- subindex：16
- 本轮目标：修复上一轮类型章节缺少真实 Flurry 代码、以文字推演冒充教学示例的问题。

## 作者本轮输入

作者先指出“示例代码也太苍白了”，随后进一步纠正：大部分文件根本没有代码示例，点名 `docs/type/user-defined-types/fixed-width-enum-example.md` 一行代码也没有，不能称为示例；其他文件同样需要检查。

本轮是在已批准的类型章节编写工作中返修教学内容。此前作者批准正式写入的授权继续适用。trait/implementation 的下一轮 prompt 保持原样，本轮不推进该主题。

## 评估与内容规划

上一版将编码表、内部数学记号、执行轨迹及孤立类型表达式放在“常见示例”中，未让读者看到实际程序。对 27 篇类型主题正文的初步检查发现，8 篇的“常见示例”没有 Flurry 代码块，其余若干篇只有声明或脱离上下文的表达式。不能靠增加篇幅或代码块计数解决教学问题。

返修以声明、操作、调用及可解释结果为一组。基础类型展示实际构造和访问；限定类型展示成功、缺失、错误和效应请求的控制流；PTC 各篇以源码契约为输入，再解释约束与求解过程。实现伪代码保留在实现引导中。涉及未确认 API 的地方不用自创库函数或转换规则填空。

已知边界：定宽模式 enum 的表示声明与普通整数的位视图匹配分别展示；不假定任意 u32 可以直接转换成该 enum。运行时依赖示例只使用本地定义的存储，不臆造 SOA 库 API。handler 示例使用已有直接返回语法，不发明 continuation API。

## 已确认与待确认

- 已确认：类型系统正式文档写入已获批准；本轮必须补齐真正的源码示例。
- 已确认：示例遵循现行 grammar 和作者此前确认的语义，不能用当前编译器实现取代语言定义。
- 本轮没有新增待拍板的语言设计提案；既有未定接口不借教学返修扩大承诺。
- 工作区中作者删除的旧拼写 occurrence-typing 文件和 TODO-data-family 文件保持删除，不重建；其他编译器改动不动。

## 过程说明

对作者说明：上一版确实以编码表、推导和文字场景代替了程序；本轮逐篇补全上下文，先修定宽 enum，再检查其他章节的实际用法与结果。

## 已完成的正式文档修改

共返修 `docs/type/` 下 27 篇主题正文。每篇“常见示例”均包含真实 Flurry 源码及调用或编译时检查上下文，合计 62 个 Flurry 代码块、1050 行源码。这些数字仅记录覆盖情况；教学要求是读者能从声明、操作、调用和结果理解规则，不能用行数代替。

| 文档 | 实际补写的教学内容 |
|---|---|
| `index.md` | 坐标 alias/newtype、optional 查询、显式转换与调用；增加按程序任务查找示例的入口 |
| `2-level-type.md` | `pair_of` 返回结构类型，运行时构造该结果；Type 运行时交付的非法案例 |
| `type-identity.md` | alias/newtype 的调用边界，纯工厂重复调用身份，非单射工厂的具体反例 |
| `comptime-gradual-dependent-type.md` | 参数化 identity、Any 输入输出实例、带目标 optional 分支、缺失成员要求的反例 |
| `type-patterns.md` | 从 Array 头检查元素类型并绑定长度；引用外部 Expected 类型 |
| `reflection-and-layout.md` | 类型大小查询与输出、quote 输入的类型观察、指针递归与非法按值布局环 |
| `refined-type.md` | tuple 精化参数调用，requires/ensures 接成下标计算链 |
| `self-reference.md` | Counter 的 Self 构造与另一个参数的 Itself 要求，完整失败调用 |
| `member-projection.md` | 两个自定义 trait 的同名方法、显式实现视角及参数化函数的固定来源 |
| `receiver-adaptation.md` | 指针字段、隐式和显式取址调用、两个直接类型视角的查找顺序 |
| `runtime-dependent-type.md` | 完整 NodeStorage/NodeIndex、数值列与左子节点列访问、两个同内容存储的 identity 边界 |
| `qualified-types/closure-type.md` | 实际捕获偏移量并调用；相同环境需求与不同函数行为 |
| `qualified-types/optional-type.md` | 配置查询区分零与缺失；嵌套 optional 区分不修改、清除、设置 |
| `qualified-types/error-type.md` | 带载荷错误、传播、成功/错误匹配、返回默认值及再次失败的 handler |
| `qualified-types/effect-type.md` | 两次请求共享入口，内层分支向外请求并转换返回值 |
| `rows.md` | 两次同类错误传播、同名效应的嵌套路由、重复 error/effect 限定的类型检查 |
| `handlers.md` | 用 u32 操作结果与 bool 主体结果区分 Y/A/B；内外层输出轨迹；错误不恢复失败点的对照 |
| `user-defined-types/structs.md` | 可观察的字段初始化顺序、默认值重新求值、显式覆盖、不完整构造反例 |
| `user-defined-types/enums.md` | 消息协议声明、三种载荷构造和匹配、实际派发与 Symbol 目标适配 |
| `user-defined-types/unions.md` | 指定成员构造、unsafe 替换共享存储、类型大小查询，以及带判别 enum 的对照程序 |
| `user-defined-types/newtype-and-typealias.md` | 用户/订单编号的接口隔离、显式底层转换、方法不继承及完整失败调用 |
| `user-defined-types/fixed-width-enum-example.md` | ADD/SUB 模式声明、整体大小检查、具体指令字的位视图匹配、寄存器绑定与输出；内部掩码移入编译器引导 |
| `propositional-constraints/index.md` | 真实下标前提调用，缺失前提的具体反例，loose 实例兑现及非线性具体计算 |
| `propositional-constraints/atoms.md` | 类型身份、分类、表示性质及大小观察的源码检查；自定义 predicate 展开 |
| `propositional-constraints/occurrence-typing.md` | 类型匹配的局部绑定、分支正数前提以及原输入事实不能泄漏 |
| `propositional-constraints/theory-solvers.md` | Array 形态/参数协作、整数差分约束、真实字节位模式及各 theory 的对应职责 |
| `propositional-constraints/solver-protocol.md` | 有限类型结果工厂、结果保证接续调用前提、联合冲突与错误 ensures 的拒绝案例 |

导航和主题入口文件不重复复制正文示例。本轮没有改 root `prompt`，没有推进 trait/implementation 设计。示例中的自定义函数、trait 和存储都就地声明，不把这些名字当作标准库 API。

## 语法、风格与语义复核

已对照 `docs/grammar/syntax`、`docs/grammar/index.md` 及此前作者确认结论人工复核新增示例。重点检查：

- 类型参数与函数返回写法、普通类型工厂的 snake_case 命名、类型模式中绑定和单引号引用的区分。
- struct 构造完整指名，enum 位置载荷与命名载荷分别使用对应产生式。
- 值取址使用 `.ref`，指针字段读取显式使用 `.*`，普通整数匹配先取 `'bitvec`。
- optional/error 模式与后缀传播，effect 直接分支的操作返回类型及向外请求位置。
- 布尔谓词使用 and/or/not，字段/参数/匹配臂用逗号，语句分号不跟在结束用的 `}` 后。
- requires/ensures 的数学整数案例不直接套用机器整数溢出等式。
- newtype 不借底层 Copy 能力重复使用已经交出的值；失败代码补齐自身局部变量，避免先因未绑定名称失败。

定宽 enum 的声明与原始 u32 的诊断函数分别展示，没有新增“任意整数直接转换成模式 enum”的许可。union 示例不推断跨成员读取规则。闭包示例由 lambda 推导环境；handler 示例不发明显式 continuation API。没有借补示例引入新的语言设计决定。

机械检查结果：

- 27 篇均具有既定七段骨架，并有真实源码及调用/检查上下文。
- 代码围栏闭合；新增示例未发现外来布尔/路径语法、错误分号、伪占位实现或大写普通函数名。
- 74 个本地链接及其锚点检查通过。
- `git diff --check -- docs/type` 通过；源码扫描同时覆盖未跟踪的新正文。
- 独立数值核对确认 `0x003100b3` / `0x403100b3` 分别匹配 ADD/SUB，rd/rs1/rs2 均为 1/2/3；零值均不匹配。互斥固定位、结构面积输出及字节模式输入的结果也已核对。
- 与本轮开始时的哈希快照比较，本轮改动的已有文件限于上述 27 篇正文；root `prompt` 完全未变。另新增本 history 文件。
- 作者此前删除的旧拼写 occurrence-typing 与 TODO-data-family 文件保持删除。复核时另发现 `docs/type/traits-implementations-extensions.md` 相对本轮基线已不存在；本轮未执行该删除，也未重建该文件。现有正文没有指向它的失效链接。

验证范围是文档结构、人工语法/风格及语义复核、链接和独立数值检查。没有用当前 luna 编译器执行这些 Flurry 示例，不能把文档检查表述为编译/运行测试通过。临时结果记录在 `/tmp/flurry-045-validation.json`。

## 本轮问题列表与写入状态

本轮没有新增需要作者拍板的问题。既有未定接口仍不在本轮承诺范围内。

正式文档写入：是，沿用作者已经批准的类型章节编写授权，执行教学返修。未 stage、commit 或 push。

## 完整过程回复

作者最初指出示例苍白时：

> 你说得对。上一版示例大多只能演示一条规则，没有让读者看到这些机制怎样一起解决问题。我会重做教学部分：以你给过的 NodeIndex/SOA、类型计算和定宽指令编码为主线，补上有实际数据、操作和结果的连贯示例，并把容易误用的边界放进对照案例。

作者进一步指出根本没有源码后：

> 你指出得更准确：我把编码表、推导和文字场景放进了“示例”段，却没有展示对应的 Flurry 程序。这不满足教学要求。我会逐篇检查并补上代码，先从这篇定宽 enum 开始，写出声明、匹配、载荷提取和调用，再核对其他章节是否真正展示了所讲的用法。

覆盖检查后：

> 核对后，确实有 8 篇的“常见示例”完全没有 Flurry 代码，另外一些只放了孤立的类型表达式。我会让每篇都有完整上下文：声明相关类型或函数，实际构造、调用或匹配，再说明结果。定宽 enum 篇会补上 ADD/SUB 的声明，以及从具体指令字提取寄存器编号的代码。

第一批补写后：

> 定宽 enum 篇已经有了实际声明、位模式匹配和三次调用；struct、enum、optional、error、effect、行与 handler 等篇也已补上代码和结果。接下来补类型计算与 PTC：把类型工厂、`requires`、分支事实和求解目标连起来，让读者能看出这些规则在程序里怎样起作用。

全面补写后：

> 27 篇正文现在都补了 Flurry 示例，包括之前只有推理表格的 PTC 各篇。我正在逐段复核，重点检查示例是否自洽、失败案例是否真的因所讲规则而失败，以及有没有误用未确认的 API。当前的文档结构、代码围栏和本地链接检查已通过。

收尾复核：

> 复核时还修正了 newtype 示例重复使用已交出值的问题，并给反例补齐了局部变量，避免它们先因作用域错误失败。文档检查和指令编码的数值核对已通过；示例尚未经过编译器执行验证。我正在把本轮修改与验证范围记入 history。

## 完整最终回复

确实，上一版没有完成“示例”这项基本要求。

已返修类型主题的 27 篇正文。你指出的[定宽 enum](/home/fin/luna/docs/type/user-defined-types/fixed-width-enum-example.md)现在包含 ADD/SUB 声明、大小检查、位匹配、寄存器提取和三次调用的输出。其他章节也补上了实际构造、调用、分支和失败案例；PTC 从源码契约展开推导。

已做语法/风格复核、链接和数值校验；示例尚未经过编译器执行验证。
