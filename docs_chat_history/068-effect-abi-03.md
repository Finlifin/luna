# 代数效应主题收尾轮：答复消化、遗留缺口与写入前的最后确认

- 日期：2026-10-03
- global_index：068
- theme：effect-abi
- subindex：03
- 本轮目标：消化作者对 067 的答复（全文已追加到 `067-effect-abi-02.md` 末尾），列出已确认结论，并就开始编写代数效应主题前仍缺的定义做最后一轮问答。本轮不修改 `docs/`。

## 作者本轮输入摘要

- 对 067 的 15 题逐条作答：1 C、2 A、4 A、5 B、6 A、7 A、8 A、9 A、10 A、11 A、12 A、13 A、14 A、15 B。第 3 题没有选项，答复为：error qualified type 已经是有规范标准的独立系统，只是按传统放在代数效应主题里。
- 附注：
  - 第 8 题：标准库几乎必须提供池化策略，用户也会默认使用它。
  - 第 9 题：多个 task 反复挂起、恢复时，每次都是不同的 continuation，continuation 同样不能复制。
  - 第 13 题：pipeline backend 自行做合规检查，检查内容不只是效应行是否为空。
- I1：grammar 是文档统一工作开始前的旧版，protocols.md 是最新写法，`handler_bind -> pattern handles expr`。
- async effect 的含义是 handler 异步执行：handler 在新栈（新 task）中运行，所以处理它的分支模式要加 `async` 关键字来暴露这一语义。同步 handler 就地执行。
- I2：catch 位置的值在分支中按普通参数解构，例如 `yield(data, k) => …`。
- I3：倾向显式指定 runtime 实例。
- I4：更新语法，加入 `inline effect`。
- I5：A，保持 `builtin.Continuation`，不带类型参数。
- O1：`catch` arm 是错误处理的语法糖。第一个 catch arm 在所有 arm 之前执行，最后一个在所有 arm 之后执行；只有一个时视作最后一个；有了后置 catch arm，消去 arm 就不再默认返回；`catch e` 绑定完整的 error qualified 值。作者给出了带重试循环的示例。
- 新提案“direct passing as implicit”：effect（或 effect enum 的 variant）带 `^attrs.effects.direct_passing` 属性，且不能有参数。安装 handler 时编译器直接执行该 handler，把结果作为隐式参数传给行中含该效应的函数。行中可以用 `#Args.diagnostic_context` 只要求单个 variant。作者认为它比打包成一个大 context 更灵活、更便于覆盖、样板代码更少，比 fn template 也更灵活。

## 已确认（本轮由作者答复确立）

| 编号 | 结论 |
|---|---|
| C1 | 可捕获性由 effect 声明许可（catch 参数写在声明中），分支自行选择捕获或直接返回 |
| C2 | 放弃与取消使用隐藏中止通道：恢复时交付中止信号，沿与错误传播相同的返回路径逐帧执行 drop elaboration，到 prompt 边界停止；用户 handler 不能匹配它 |
| C3 | error qualified type 是已有规范的独立系统，在本主题中按传统并列 |
| C4 | `resume expr` 在分支中交付 Y 并结束本分支；在捕获分支中保证尾切换；非尾恢复通过 continuation 值上的显式操作完成 |
| C5 | continuation 能否逃逸由可达性类型约束，可以越过捕获它的 prompt |
| C6 | 外层效应 ε 在恢复点取得入口；主体不跨可挂起调用按值缓存外层 evidence |
| C7 | 只有有栈后端；主体代码与后端无关 |
| C8 | 后端在每次安装 handler 时选择，经协议供应；标准库提供池化后端，默认使用它 |
| C9 | 不提供多次恢复；每次挂起都产生新的 continuation，continuation 不可复制 |
| C10 | 挂起的计算不跨线程迁移 |
| C11 | 协作式，只在类型化的 perform 点挂起 |
| C12 | `.await` 映射到 trait 协议；`async` 分支表示 handler 在新 task（新栈）中执行；同步 handler 就地执行 |
| C13 | C ABI 函数的效应行必须为空；pipeline backend 另做更全面的合规检查 |
| C14 | 挂起栈上的 C 帧：允许挂起与恢复，禁止放弃，无法静态排除时运行时 trap |
| C15 | `inline effect` 只表达内联意向，无法内联时退回普通 evidence |
| C16 | 处理器绑定写法以 protocols.md 为准：`handler_bind -> pattern handles expr`；grammar 需更新 |
| C17 | catch 位置在分支中按普通参数解构：`yield(data, k) => …` |
| C18 | continuation 类型保持 `builtin.Continuation`，不带类型参数 |
| C19 | 错误消去中的 catch arm 是语法糖；前置、后置的顺序规则与 `catch e` 绑定完整值，如作者所述 |
| C20 | 作者倾向显式指定 runtime 实例 |

## 评估

### 1. 答复之间有四处需要对齐

**(a) “同步 handler 就地执行”与捕获分支。** 捕获分支不能真正就地执行：若分支帧压在被捕获栈的顶上，K 就无法与分支帧分离，只能复制栈段，这与 C7、C8 的池化目标冲突。我认为这句话指的是直接分支；捕获分支需要切回安装者的栈，含捕获分支的 handler 的主体运行在池化栈上。需要确认（问题 1）。

**(b) 第 3 题仍未定。** C3 说明了错误系统的地位，但没有回答：就地执行的同步分支中出现 `error`、`return`、`break`、`continue` 时，主体帧怎么办。067 的例子在两种答案下分别输出 `99` 与 `0`。C2 的中止通道已经确认，所以选项 A 可以实现（问题 2）。

**(c) C18 与 C5 之间的类型缺口。** continuation 不带类型参数，又允许逃逸。存进调度器队列以后，恢复点用什么检查交付值的类型 Y、整个结果 B 以及外层行 ε？在分支内部可以从操作签名推出，离开分支以后就没有来源了（问题 5）。

**(d) C12 与 C20。** `.await` 走协议、runtime 又要显式指定，两者可以共存，但需要定下 runtime 实例出现在哪里。你的 direct passing 示例里恰好有 `some_runtime() -> *Runtime`，它可以把两者接起来（问题 8）。

### 2. direct passing：可行，并有直接的先例

Koka 的 `val` 操作就是这种语义：handler 安装时求值一次，主体中的 perform 只是读这个值。你的版本还加上了覆盖、单 variant 行选择，以及与 Flurry 隐参系统的连接。需要先定的边界：

- **这是语义承诺，还是优化？** 普通 handler 每次 perform 都执行一次分支，direct passing 只执行一次。分支有副作用、或者每次返回不同值时，两者可以区分。若它是语义承诺，属性就改变了 handler 的含义，必须写进正式定义（问题 11）。
- **与 C6 冲突。** C6 要求外层效应在恢复点取得入口；direct passing 却把安装时的值当参数直接传下去。一个越过 prompt 的 continuation 在别处恢复后，应当看到安装时的值还是恢复点的值？两种答案的实现形态不同（问题 12）。
- **值的可达性。** 传下去的值（例如 `*Runtime`）必须活得比所有使用它的计算更久；在 C5 下，这包括逃逸出去的 continuation。
- **安装时的失败。** 分支在主体开始前运行。分支出错或请求外层效应时，都发生在主体开始之前，并在外层环境中处理。
- **`#Args.diagnostic_context` 是通用的行写法。** 8-25 草稿已经说明 effect enum 的每个 variant 是独立效应，所以按 variant 选择不应只给 direct passing 使用（问题 14）。

### 3. 写进现行文档时需要改动的条文

`docs/type/handlers.md` 与 `docs/algebraic-effect-and-delimited-continuation/index.md` 都写着“恢复能力以单次、不可逃逸为基线”。C5 改成了由可达性类型约束逃逸，这两处在写入时必须改写（问题 19 一并确认）。

---

## 需要你拍板的问题

### 执行位置与控制

1. **捕获分支在哪条栈上执行？**
   - A. **（推荐）** 直接分支就地执行；捕获分支切回安装者的栈执行；含捕获分支的 handler，其主体运行在池化栈上。这是不复制栈的唯一形态。
   - B. 捕获分支也就地执行，捕获时把分支帧以下的栈段复制出去。与池化目标冲突。
   - Other: …

2. **同步直接分支中的 `error`／`return`／`break`／`continue`**（067 第 3 题重问）
   - A. **（推荐）** 视为放弃主体：主体帧经 C2 的中止通道清理，控制在边界外交付；主体内部的错误处理不截获。067 例子输出 `99`，与现行“分支在边界外”的规则一致。
   - B. 禁止直接分支进行非局部退出。
   - C. 错误在 perform 点所在上下文传播。067 例子输出 `0`，需要修改 handlers.md 的边界规则。

3. **分支中“直接”与“捕获”怎样区分？**（effect 声明形如 `effect yield(data: u32, catch k: builtin.Continuation);`）
   - A. **（推荐）** 按实际绑定：`yield(data) =>` 省略 catch 位置，是直接分支；`yield(data, k) =>` 是捕获分支；`yield(data, _) =>` 先捕获再立即放弃（例如生成器只取第一个值）。
   - B. 参数个数必须与声明一致；写 `_` 表示直接分支，不捕获。
   - C. Other: …

4. **捕获后既没有恢复、也没有存起来的 continuation**
   - A. **（推荐）** 作用域结束时由 drop elaboration 隐式放弃，走 C2 的中止通道。
   - B. 必须显式消费，否则编译错误：要么恢复，要么用显式放弃操作。
   - C. Other: …

5. **continuation 逃逸以后，Y、B、ε 怎样检查？**（C18 与 C5 的缺口）
   - A. 编译器在 `builtin.Continuation` 上记录用户不能书写的隐藏索引 Y、B、ε，随值一起流动；所有存放它的位置都要能承载这些索引，例如同一个队列里的 continuation 必须索引一致。
   - B. 在分支内部恢复时由编译器检查；离开分支以后的恢复是 unsafe 操作，由调用者负责类型一致。
   - C. 重新考虑 I5，改为带参数的形式。
   - Other: …

6. **continuation 显式操作的表面形式**（C4 的非尾恢复，以及问题 4 选 B 时的显式放弃）
   - A. 调用形式 `k(y)`，放弃另有操作。
   - B. `builtin.Continuation` 上的方法；方法名待定，不在本轮编造。
   - C. `builtin` 下的内建函数，参数为 continuation 与值。
   - Other: …

### 异步

7. **async 效应与分支能否混配？**
   - A. **（推荐）** async effect 只能由 `async` 分支处理，非 async effect 不能用 `async` 分支；混配为静态错误。
   - B. async effect 可以用同步分支：就地执行，perform 仍返回一个已完成的 Future（便于 mock）；非 async effect 不能用 `async` 分支。
   - C. 两个方向都允许。

8. **runtime 实例在哪里显式出现？**（C12 与 C20 的对齐）
   - A. 在安装处用 `.use(runtime)` 写出实例；`async` 分支与 `.await` 经效应行找到它。
   - B. 按 9-29 草稿写成 `.await(runtime)` 与 `.async(runtime)`，grammar 需要改为 `async_await -> expr.await(expr)`。
   - C. **（推荐）** runtime 指针通过一个 direct passing 效应传递（如你示例中的 `some_runtime() -> *Runtime`）。安装 handler 时显式给出实例；`.await` 的协议实现和 `async` 分支都从这个效应取得 runtime；真正的挂起仍由 runtime 在 task 根部安装的捕获效应完成。
   - Other: …

9. **结构化并发的作用域**
   - A. **（推荐）** 含 `async` 分支的 handler 安装就是作用域：正常结束时等待它派生的全部 task；因错误或 `return` 提前退出时，以 C2 的中止通道放弃其中仍挂起的 task。
   - B. 作用域由 runtime 实例决定，与 handler 安装无关。
   - C. Other: …

10. **没有 await 就被丢弃的 Future**
    - A. 丢弃即取消对应 task。
    - B. 不取消，在作用域结束时一并等待。
    - C. 必须消费，否则编译错误。

### direct passing

11. **`^attrs.effects.direct_passing` 的性质**
    - A. **（推荐）** 语义承诺：分支在安装时、主体开始前，在外层环境中恰好执行一次；之后每次 perform 都读这个值。分支的错误与效应都发生在主体开始前。
    - B. 优化许可：语义仍是每次 perform 执行一次分支；只有分支达到 P4 时才允许编译器只执行一次。
    - C. Other: …

12. **direct passing 的值与 C6（外层入口在恢复点取得）的关系**
    - A. **（推荐）** 词法固定：使用安装时的值，不随恢复点重绑定；可达性规则要求该值活得比携带它的 continuation 更久。ABI 保持为普通隐式参数。
    - B. 遵守 C6：从 prompt 槽位读取，恢复时重绑定。这样它就不再是普通参数。
    - C. Other: …

13. **调用处能否显式供应？**
    - A. **（推荐）** 能，沿用隐参的命名供应 `.diagnostic_context = ctx`，等价于只在这一次调用外安装一个 direct passing handler。
    - B. 不能，只能通过安装 handler 供应。

14. **按 variant 选择的行写法 `#Args.diagnostic_context`**
    - A. **（推荐）** 适用于所有 effect enum：`#E` 是全部 variant，`#E.v` 是单个 variant。
    - B. 只适用于 direct passing 的 variant。

### 语法与错误系统

15. **effect 相关的语法增量**，以下为 DRAFT — NOT CANONICAL：

    ~~~text
    algebraic_effect -> async? inline? effect id(parameter*) (-> result)? clauses? (block | (= expr))?
    effect_enum -> effect enum id clauses? { effect_variant* }
    effect_variant -> attribute* id(parameter*) (-> result)?
    handler_bind -> pattern handles expr
    catch_arm -> catch pattern => (block | statement)
    ~~~

    - 15a. `async` 与 `inline` 能否同时出现？A. 不能，二选一；B. 能，顺序如上；C. Other。
    - 15b. effect enum 整体能否带 `async`／`inline`？A. 不能，只在 variant 上用属性表达；B. 能，作用于全部 variant。
    - 15c. `handler_bind` 中的 `expr`：A. 沿用 9-18 草稿，用 `and` 连接多个错误／效应标签；B. 用行写法 `[A, B]`；C. Other。
    - 15d. `catch_arm` 只出现在错误消去 `expr! { … }` 中吗？A. 是；B. 也出现在 `match` 中。

16. **effect 声明的主体 `(block | (= expr))` 是什么？**（grammar 中已有，现行文档没有解释）
    - A. 缺省 handler：在没有安装处理器的环境中使用。
    - B. 删除主体形式，只保留以分号结束的签名。
    - C. Other: …
    - 附带：省略 `-> result` 时，操作结果是 `()` 还是必须写出？

17. **catch arm 的细节**（C19 的补充）
    - 17a. 成功值是否经过 catch arm？A. **（推荐）** 不经过，catch arm 只在错误时执行（否则作者示例在成功时也会重试）；B. 经过。
    - 17b. 有后置 catch arm 时，整个消去表达式的值：A. 等于后置 catch arm 的值，普通 arm 的值被丢弃；B. 普通 arm 的值仍是结果，后置 catch arm 只能用 `return`／`continue`／`break` 等改变控制流；C. Other。
    - 17c. 没有普通 arm 匹配的错误：A. 后置 catch arm 执行后按 17b 交付；B. 后置 catch arm 执行后继续向外传播，类似 finally；C. Other。

18. **错误类型的命名不一致**（错误系统按传统在本主题内，写入时会碰到）
    - `docs/type/qualified-types/error-type.md` 写 `LimitError` 与 `too_large`；`docs/traits/protocols.md` 与 9-18 草稿写 `LimitErr` 与 `TooLarge`（草稿注释：错误类型名以 Err 结尾，variant 用大驼峰）；`docs/grammar/index.md` 的命名表规定枚举变体用 `snake_case`。
    - A. 采用 9-18 规范：错误枚举名以 Err 结尾、变体大驼峰；同步修改 error-type.md 与命名表。
    - B. 保持命名表；修改 protocols.md。
    - C. Other: …

### 写入范围

19. **本主题的文件结构与改动范围**
    - 现有空文件：`algebraic-effect.md`、`async-algebraic-effect.md`、`handling-capability.md`、`multi-prompt-delimited-continuation.md`；`index.md` 已有内容。
    - A. **（推荐）** 保留这四篇；新增 `direct-passing.md` 与 `effect-abi.md`（后者集中写运行时与编译器引导：池化后端、中止通道、尾切换、外层入口槽位）；错误系统只在 index 中链接 `type/qualified-types/error-type.md`，catch arm 写进 error-type.md。类型规则留在 `type/handlers.md` 与 `effect-type.md`，本主题链接过去，不重复。同时把 handlers.md 与 index.md 中“不可逃逸为基线”改写为 C5 的可达性规则，并按问题 15 更新 `docs/grammar/syntax`。
    - B. ABI 不单独成篇，分散写进各篇的第 6、7 段。
    - C. Other: …

## 仍待定

问题 1–19 全部未决。direct passing 的命名与属性路径 `attrs.effects.direct_passing` 按作者原文记录，不另作改动。

## 文档写入

否。作者说“写入文档”后，按问题 19 的结论动手。

## 检查说明

- 本轮唯一新写的 Flurry 片段是问题 3 中的 effect 声明：catch 参数用 `catch k: builtin.Continuation`，符合 `catch_param -> catch parameter`；省略结果；以分号结束。分支写法沿用作者在 C17 中给出的 `yield(data, k) =>`。
- 问题 15 的产生式均标为 DRAFT — NOT CANONICAL，风格对齐 `docs/grammar/syntax`。
- 作者的 direct passing 与 catch arm 示例只作摘要引用，原文保存在 067 末尾的作者答复中，没有改写。
- 问题 18 列出了现行文档之间的命名冲突，未擅自选边。


## 授权代拟答复（2026-10-07）

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
