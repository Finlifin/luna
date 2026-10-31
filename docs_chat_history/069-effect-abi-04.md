# 代数效应：代拟 068 答复与完整主题写入

- 日期：2026-10-07
- global_index：069
- theme：effect-abi
- subindex：04
- 本轮目标：按作者授权代拟 068 的答复，完成代数效应文档，补齐 pattern 优先级、词法列表和相关 syntax。
- 同一工作回合的后续记录：070 保存 CMTT 交付；071 保存 pipeline projection 首轮审阅。三个记录不是三次作者答复。

## 作者输入摘要与授权

作者指出 067、068 由 Opus agent 编写，其中把 error 与 effect 的独立 ABI 混淆，并在已有属性语义下重新询问是否构成承诺。作者要求代为回复 068，然后编写代数效应整个主题；并要求以现有解析规则为基础补充 pattern 优先级，新增括号分组，以现有 lexer 为基础补齐关键字与预留关键字，并加入后续主题已确定的关键字。作者还授权编写 CMTT，再进入 pipeline projection。

因此，本轮可以完成局部设计取舍和正式写入，不另设确认门槛。代拟答复明确标注授权来源，不冒充作者逐字作答。原 067、068 问题和作者原回复均保留；068 末尾已追加以下完整答复。

## 对 068 的完整授权代拟答复

作者本轮要求：“67、68是opus agent编写的，它问了些蠢问题，比如分不清error和effect完全不同的abi，并且在加了属性的情况下还在问是不是语言承诺，你帮我回复68，然后我们开始编写代数效应整个主题。”以下由助手按这项授权代拟；不是冒充作者先前逐字答复。已明确的规则直接承接，局部选择由本轮授权落实。

1. 池化后端可以令捕获分支在所选边界外的执行栈上运行。同步表示不创建异步子 task，并不要求所有同步分支占用同一物理栈。删除“不复制栈的唯一形态”这一过强断言。
2. 本题混合了独立的 ABI。error qualified 是普通成功／错误结果的表示；effect 的入口按操作结果 Y 交付。若 Y 是 !E T，handler 可以返回错误值，调用者再按 ! 或 match 处理。若 Y 是 u32，不能把 error E 当作一个 u32 返回。067 的例子没有先满足入口结果契约，不能据此在 99 与 0 中选控制语义。明确指向外层的 return/break/continue 另按其控制目标和退出清理处理；隐藏中止信号不成为用户错误行的一项，也不要求使用错误结果 ABI。复用的是清理控制图，不是错误标签、数据布局或匹配入口。
3. 捕获许可仍由声明的 catch 参数给出。直接分支不取得 continuation，捕获分支在该位置用普通参数模式接收；省略该专用供应位置为直接处理，显式绑定或显式通配该位置为捕获。普通操作参数不能因此随意省略。通配对新交付的 continuation 按普通丢弃及清理规则执行，不是“无捕获”的另一种拼写。
4. 采用自动清理：尚未转交或恢复的 continuation 离开清理范围时走放弃协议；其合法性仍检查清理依赖和 C 帧限制。已移入调度队列的值由队列所持责任管理，不能在原分支再放弃一次。
5. 保持 builtin.Continuation 不带参数，并保持 intrinsic 附近、检查有限的定位。编译器能在直接语法位置核对操作结果；逃逸存储和显式恢复的库封装必须维护 Y、B、外层入口及可达依赖的关联。原始显式恢复属于 unsafe 契约，不引入未经确认的隐藏依赖类型系统，也不以 Any 视为检查成功。
6. 显式操作经 builtin.Continuation 的内建能力及后端协议接入；本章规定消费、恢复、放弃的契约，不新增公开方法名。resume 语句保留已确认的尾恢复保证。示例若需要显式操作，使用本例声明的适配接口并说明其契约。
7. 首版要求 async 声明和 async 分支对应。同步 mock 可以在 async 分支内立即算出结果；不改变 Future 交付和子工作归属。
8. runtime 由使用者显式选择并在安装处供应，.use(runtime) 是文档的接入写法；await 由协议获得该已安装实例。direct passing 可以供应运行时指针，但不是所有 runtime 唯一强制的查找途径。9-29 的显式方法调用属于库接口设计，不因选用协议而自动增加一套语言内建 .async(runtime) 语法。
9. 含 async 分支的安装范围承担结构化并发责任：正常退出等待所属子工作；提前退出取消并完成相应清理后才能释放被借用环境。runtime 可以比这个范围存活更久；调度实例的寿命不等于单次任务组的寿命。
10. 丢弃 Future 不隐式取消子工作；所属作用域仍负责等待或取消。取消使用其明确协议，避免普通结果凭据的丢弃改变工作是否发生。
11. 直接承接作者定义：^attrs.effects.direct_passing 规定无参数操作在安装时求值一次，之后转交所得值。这已经是行为定义，不再讨论“只是优化”。普通操作与此不能因结果类型相同而互换 ABI。
12. 按两个层次区分：每个安装实例的 direct passing 结果固定；continuation 恢复时，外层操作入口按 067 已确认规则重新供应，因此未来请求读到所选新实例已持有的值。已经取得并保存为普通局部值的结果不被追溯修改。实现可在恢复边界刷新外层隐参槽，非挂起路径继续直接传值；不每次重新执行原 handler。内层重新安装的处理实例保留自己的值与身份。
13. 不额外定义操作名自动成为函数命名参数名的语法糖。先通过显式 handler 覆盖值；普通 implicit 参数若声明存在，继续使用已有命名供应规则。effect 行不能凭空生成可用 .name = value 供应的源码形参。
14. #E 与 #E.v 分别引用操作组与单个操作，适用于所有 effect enum。组不是一个新的不可拆分标签。
15. 语法按下述范围补齐：async 与 inline 是独立修饰，采用 async? inline? effect；effect enum 的各项采用操作签名，可独立 async/inline，并可带属性；不增整组修饰的额外默认规则。handler_bind -> pattern handles expr，多个能力按作者示例用 and 连接。catch arm 只属于错误消去，普通 match 的错误模式仍是 error pattern。
16. effect 采用签名声明，省略结果为 void；不根据历史 grammar 的函数体占位发明“没有 handler 就自动用缺省主体”的机制。本轮删除该未定义主体入口。
17. catch arm 只在输入为错误时执行。前置 catch、普通匹配分支、后置 catch 顺序组合；一个 catch 视为后置。具有后置 catch 时，普通分支完成后继续后置处理，其普通值不提前完成消去；后置路径交付整个错误分支的结果，或执行其显式控制动作。无普通分支匹配也进入后置 catch；它不是执行后必定继续传播的 finally。catch 绑定原始完整错误限定值，向各步骤提供观察／消费时仍须满足资源规则，不隐式复制错误载荷。
18. 采用作者已给出的错误命名规范：错误类型以 Err 结尾，错误枚举变体 PascalCase；普通枚举变体仍 snake_case。同步纠正错误类型章及风格表，不要求作者再次回答同一命名约定。
19. 采用独立总览、基本效应、处理能力、continuation、async、direct passing、ABI 七篇；错误处理语法另设本主题阅读页并链接错误类型规则。同步更新 handler 的逃逸、恢复点上下文等规则、grammar 和阅读索引。本轮已有正式写入授权。

另需纠正 067 的若干推论：外层入口在恢复点重绑定不能仅从 resume 的类型箭头推导，而是作者随后明确选择的语义；有栈 ABI 的兼容后端可共享主体代码，但不因此保证任意异构 ABI 自动兼容；栈链链接可为常数开销，附带的上下文交接、资源清理与验证成本必须分别计算；未开始任务的迁移也需要合法的捕获转交，不能从“尚未运行”推导任意资源可跨线程。

## 写入结构与覆盖

| 文件 | 行数 | Flurry 代码块 |
|---|---:|---:|
| [algebraic-effect.md](../docs/algebraic-effect-and-delimited-continuation/algebraic-effect.md) | 190 | 5 |
| [async-algebraic-effect.md](../docs/algebraic-effect-and-delimited-continuation/async-algebraic-effect.md) | 160 | 3 |
| [direct-passing.md](../docs/algebraic-effect-and-delimited-continuation/direct-passing.md) | 170 | 4 |
| [effect-abi.md](../docs/algebraic-effect-and-delimited-continuation/effect-abi.md) | 158 | 2 |
| [error-handling.md](../docs/algebraic-effect-and-delimited-continuation/error-handling.md) | 143 | 3 |
| [handling-capability.md](../docs/algebraic-effect-and-delimited-continuation/handling-capability.md) | 157 | 3 |
| [index.md](../docs/algebraic-effect-and-delimited-continuation/index.md) | 122 | 2 |
| [multi-prompt-delimited-continuation.md](../docs/algebraic-effect-and-delimited-continuation/multi-prompt-delimited-continuation.md) | 173 | 3 |

各篇按 Intro、语法、正式定义、常见示例、跨语言差异、运行时引导、编译器引导组织。阅读路线从请求／响应进入处理能力，再分别解释错误结果、安装值、continuation、async 与 ABI。

示例包含：同一主体的不同 handler、分组操作、同名请求向外供应、resume 提前交付 Y、Y 本身是错误限定结果、状态化 trait 接入、前后 catch 与重试、安装一次而请求多次、配置覆盖、三次挂起的三份恢复权与同一 Drop 对象的存活／放弃清理、非尾恢复的 B 结果、显式 runtime 下并发请求及 C ABI 包装。

异步例子明确把 runtime 视为所选库已创建的实例；没有编造 Runtime.new 等 API。非尾恢复例子显式声明适配函数及契约，没有把它当成已确认的标准库方法。

实现引导区分路由、控制、存储与调度；给出池化栈链的捕获／恢复／归还、隐藏中止和清理、C 帧屏障、结果发布与取消之间唯一恢复权的交接。实际遇到非法跨 C 帧放弃才 trap；静态不能排除时保留检查，不是因证明未知而立即 trap。

## 语法与既有章节同步

1. pattern 优先级以本轮指定的 parser 为基础：名称投影、应用、成功包装、or、as、guard／and-is，由紧到松。前缀与范围端点按实际读取范围说明；括号明确划定子模式，区分 (p)、(p,) 与 ()。
2. 新增有输出说明的括号示例；优先级与 arm 的尝试顺序分开。
3. lexer 词表核对得到 90 项关键字及预留拼写：现有集合排除作者已经撤销的 extern 和作为标点的 _，加入 template、defer、var、Itself。形式文档只列现行拼写，不列淘汰概念。
4. 词法列表区分当前有构造含义的关键字与预留词，不替预留词发明构造。属性路径成员按其合法位置解释。
5. grammar 补齐操作组、逐项 async／inline／属性、catch arm、绑定位置的 handles、contextual pattern、quote／splice 映射、分组模式和模板 arm 接入。函数签名没有 handles 段，外部 ABI 沿用属性。
6. handler 类型章改为可达依赖约束逃逸，并明确恢复点供应外层入口；错误类型章补 catch 衔接。错误命名按作者既有规范使用 Err／PascalCase，普通枚举变体继续 snake_case。

正式文档没有引用编译器具体文件或私有函数作为实现指令；本轮对 lexer/parser 的检查仅服务作者明确指定的语法基线。

## 已确认与授权落实的区分

- 作者既有确认：独立错误 ABI、direct passing 安装时求值、deep handler、单次不可复制 continuation、可达性逃逸、恢复点外层环境、可替换兼容有栈后端、默认池化、线程内恢复、协作挂起、C 边界限制、inline 意向可回退。
- 本轮代拟落实：直接／捕获分支按 catch 位置区分，未转交的恢复权自动清理，async 声明与分支对应，显式 runtime 安装，安装范围的结构化退出与 Future 丢弃行为，direct passing 的固定安装值与恢复后新请求分层，前后 catch 的最终结果及语法收束。它们已在作者“代为回复并写入”的授权下进入文档。
- 不冒充现成公共 API：continuation 显式方法名、具体 runtime 构造函数、控制后端 trait 名和固定存储布局。本章给出协议行为，不无依据固定这些库拼写。

## 校验与工作区边界

已对照 grammar/index.md、grammar/syntax 自检新增示例的投影、参数修饰、命名、括号、逗号和分号；明确区分完整示例、使用端片段与拒绝例。新增主题页面通过七段结构、Markdown 围栏、代码括号和本地链接检查。针对本轮文件的 git diff --check 通过。

现有编译器不是本轮设计的语义权威，也未用尚未支持完整语法的编译器宣称示例编译通过。本轮没有修改编译器、proposal、用户原有实现改动，没有提交或推送。

## 本轮问题

代数效应没有新增需要作者重复确认的问题。下一主题的问题及预填答案完整保存在 071 与 prompt。
