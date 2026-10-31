# 代数效应 ABI（第二版）：后端无关的主体代码、池化 prompt 栈、基于效应的异步

- 日期：2026-10-03
- global_index：067
- theme：effect-abi
- subindex：02
- 本轮目标：作者指出 066（effect-abi-01）由另一模型写成，要求给出一版独立的 ABI 设想；nessa 位于 `~/nessa2`。本轮只讨论，不修改 `docs/`。

## 作者本轮输入

> effect abi 01是gpt astra写的，你出一版新的，让我看看你的想法，另外nessa可以在~/nessa2中找到。

## 本轮阅读范围

- 066 全文。
- 作者草稿（`/home/fin/vilya/examples/`）：`2026-9-28-algebraic-effects.fl`、`2026-9-29-continuation-pool.fl`、`2026-1-30-algebraic-effect-abi-3.0.fl`、`2025-6-10-thread-local-algebraic-effect-handler-abi.fl`、`2026-9-19-inline-effect.fl`、`2026-9-18-handling-capability-by-trait-system.fl`、`2025-5-19-asynchronous-algebraic-effect.fl`、`2026-8-25-handler-capability.fl`、`2026-8-25-effect-enum.fl`。
- luna 现行文档：`docs/type/handlers.md`、`docs/type/qualified-types/effect-type.md`、`docs/functions/parameter-elaboration.md`（catch）、`docs/traits/protocols.md`（处理能力）、`docs/resource-management/runtime-purity.md`（完成方式含“挂起”）、`docs/grammar/syntax`、`docs/grammar/index.md`、`docs/algebraic-effect-and-delimited-continuation/prompt.fl`。
- nessa2：`docs/algebraic-effect-and-multi-prompt-delimited-continuation/` 下 in-place、evidence passing、multi-prompt、dynamic call、capability；`docs/concurrency-and-async-algebraic-effect/` 下 async、task tree、structured concurrency；`docs/dev/architecture.md` §5.3、§5.4、§6；`engine/stack_pool/src/lib.rs`。
- 外部对照：libmprompt（每个 prompt 独占一条在虚拟内存中原地增长的栈，yield/resume 常数时间，多次恢复需复制，README 明确提示与线性资源的冲突）。

nessa 与 libmprompt 只用于理解设计空间，不作为 Flurry 已有规则。

---

## 与 066 的主要分歧

1. **主体代码不需要按后端实例化。** 066 认为按某种挂起约定生成的代码不能换后端，因此建议“按控制域选定后端并实例化”。我的判断是：只要恢复是单次的、栈不搬移，perform 对主体来说就是一次经 evidence 的间接调用，栈切换发生在 handler 入口桩内部，主体无从区分。后端可以在每次安装 handler 时选择，主体代码只编译一份。
2. **地址稳定是准入条件，不是可选契约。** Flurry 允许指向局部对象的裸指针，也有 FFI；搬移栈的后端在 Flurry 中没有安全实现方式。
3. **多层捕获可以是 O(1)。** 每个 prompt 独占一条栈时，捕获与恢复只是拆接栈链，与深度无关。不是常数的只有放弃时的清理与动态查找。
4. **066 漏掉了两件直接影响 ABI 的事。** 其一，直接分支里的 `error`／`return` 必须放弃主体帧，因为分支语义上位于边界之外；其二，非尾位置恢复会让父栈随 yield 次数线性增长，尾恢复必须是语言保证。
5. **066 没有使用作者自己的草稿。** abi-3.0 的三分法、9-29 的显式 runtime 写法、thread-local 路由草稿都直接关系本题；grammar 也已经有 `async? effect`、`async pattern`、`resume`、`.await`、`async { }`。

---

## 一、先拆轴

9-28 草稿列出的七类，实际是几条独立轴的组合。拆开以后，哪些由编译器决定、哪些可以交给库，界线就清楚了。

| 轴 | 取值 | 谁决定 | 何时决定 |
|---|---|---|---|
| R 路由 | 静态 evidence ／ 编译期 evidence（inline effect）／ 动态查找 | 编译器 | 编译时 |
| C 控制 | 不恢复（错误）／ 直接分支 ／ 带非局部退出的直接分支 ／ 捕获·单次 ／ 捕获·多次 | effect 声明 + handler 分支 | 编译时 |
| S 存储（仅捕获） | prompt 独占池化栈 ／ 换入同一地址区间（多次恢复） | 库 | 安装 handler 时 |
| T 调度 | 无（同步）／ runtime 库 | 库 | 安装 runtime handler 时 |

草稿七类在这张表上的位置：

| 草稿条目 | R | C | 备注 |
|---|---|---|---|
| never-resume（错误） | 静态 | 不恢复 | 带标签返回即可 |
| handler inlinable | 编译期 | 任意 | 展开后变成局部跳转 |
| evidence，无捕获 | 静态 | 直接分支 | |
| dynamic，无捕获 | 动态 | 直接分支 | |
| evidence，有捕获 | 静态 | 捕获 | |
| dynamic，有捕获 | 动态 | 捕获 | |
| async | 静态 | 捕获（await 处）| 再加 T 轴的调度库 |

所以 async 不是第七种 ABI，而是“捕获 + 调度库”。这正好对应你“异步底层完全基于代数效应”的目标。

---

## 二、核心论断

### 2.1 单次恢复 + 栈不搬移 ⇒ 主体代码与后端无关

下面是内部记号，不是 Flurry 语法：

~~~text
perform op(x)             ⇒  y = ev_op.entry(ev_op.env, x)    -- 对主体而言只是间接调用

直接分支的入口             ⇒  在当前栈上执行分支，返回 y
可捕获分支的入口（池化）   ⇒  保存被调用者保存寄存器、sp、返回地址
                              → 切到父栈 → 执行分支(x, K)
                              …… 以后 resume(K, y) 切回，入口桩“返回” y
~~~

主体只需要满足两项与后端无关的要求：

- 在可能被放弃的调用点之后有**中止通道**（见 2.5 与问题 2）；
- 若 continuation 可以越过捕获它的 prompt，主体不得跨可挂起调用按值缓存外层 evidence（见 2.6 与问题 6）。

是否为主体领取一条新栈，由安装 handler 的代码自己决定，这也是局部决定。因此函数指针、trait object、独立编译都不受后端影响，也不出现函数着色。066 第 1 题的三个选项在这一前提下只剩“选择粒度”的差别；只有当你需要无栈（状态机）后端时，按后端实例化才变得必要（问题 7）。

### 2.2 Flurry 的栈不能搬移

OCaml 5 的 fiber、Go 的 goroutine 都从小栈起步，满了就整体复制到更大的区域。它们能这样做，是因为运行时掌握所有指向栈的指针。Flurry 有指向局部对象的 `*T`、`.ref`，还有 C 帧，复制后这些指针全部失效。

剩下的可行形态只有两种：

- 每条栈预留固定的虚拟地址区间，物理页按需提交（nessa 的 slot、libmprompt 的 gstack）；
- 多次恢复时把栈段换入**同一**地址区间。

所以“地址稳定”应当作为任何后端的准入条件写进控制后端协议，而不是后端可选的性质。

### 2.3 prompt 独占栈时，捕获与恢复与深度无关

~~~text
栈 1: [main … install P_outer]  ─►  栈 2: [P_outer 的主体 … install P_inner]  ─►  栈 3: [P_inner 的主体 … perform]
~~~

perform 的目标是 P_outer 时，K 就是“栈 2 → 栈 3”这条子链：捕获是解开一个链接，恢复是重新接上，都与中间有多少层 prompt 无关。不是常数的部分只有：

- 放弃 K 时的清理，代价与持有清理义务的帧数成正比；
- 动态查找，代价与 handler 链长度成正比。

### 2.4 尾恢复必须是保证

deep handler 在栈切换实现下的执行顺序是：分支在父栈上运行；分支调用 resume 后，如果 resume 不在尾位置，分支帧留在父栈上等待 B；主体再次 perform 时，下一个分支在这个帧**之上**运行。

后果：一个 yield N 次的生成器，若分支以非尾位置恢复，父栈增长 O(N)，最终溢出。因此“恢复是分支最后一个动作时，先弹出分支帧再切换”必须是语言保证，与 `musttail` 同级，不能留给优化器。grammar 中已有的 `resume expr? (while expr)?` 语句形态，很适合作为这一保证的载体（问题 4）。

### 2.5 直接分支的非局部退出等于放弃主体

现行 handlers.md 规定分支在当前边界外执行。in-place 实现让分支物理上运行在 perform 点的栈顶，这时如果分支发出错误、`return` 或 `break`，必须先丢弃 perform 点到边界之间的主体帧，并且不能让主体内部的错误处理截获。

下面的例子在两种实现下输出不同：

~~~flurry
enum LimitErr {
    TooLarge(u32),
}

effect read_number() -> u32;

fn checked() -> #[read_number] !LimitErr u32 {
    let value = read_number()#;
    if value <= 8 {
        value
    } else {
        error LimitErr.TooLarge(value)
    }
}

fn body() -> #[read_number] u32 {
    let checked_value = checked()#;
    checked_value! {
        .TooLarge(_) => 0,
    }
}

fn outer() -> !LimitErr u32 {
    body()# {
        read_number() => error LimitErr.TooLarge(99),
    }
}

test {
    let result = outer()! {
        .TooLarge(n) => n,
    }
    println("{}", result);
}
~~~

- 按现行“分支在边界外”的规则：分支的错误越过 body，在 outer 的错误行中交付，输出 `99`。
- 按 nessa in-place 文档的做法（错误在 perform 点所在上下文传播）：错误从 `checked()` 返回，被 body 的 `.TooLarge(_)` 截获，输出 `0`。

第一种结果需要的机制，与“放弃一个被捕获的 K”完全相同：逐帧执行清理，直到边界。而且 perform 点无法静态知道当前 handler 的分支会不会非局部退出，因为分支的错误不在主体的行里。所以要么每个 perform 点都带中止通道，要么使用表驱动展开（问题 2、3）。

### 2.6 `resume k : Y -> Comp(ε,B)` 已经隐含的 ABI 约束

handlers.md 写的是恢复返回 `Comp(ε,B)`：外层效应 ε 在**恢复点**取得入口。

- 若 K 不越过捕获它的 prompt，恢复点只可能在该 handler 的分支或状态内，恢复点的 ε 入口与捕获时相同，主体可以缓存外层 evidence。
- 若 K 可以越过 prompt（9-28 prompt.fl 中 `queue.push_back(k)` 后分支结束的写法），恢复点的 ε 入口可能不同，主体就不能跨可挂起调用按值缓存外层 evidence，而要经 prompt 帧中的槽位读取，由恢复操作更新。

abi-3.0 写“动态的：动态绑定 handler，挂起后可在其他上下文中恢复”。我把它读作上面第二种语义；这需要你确认（问题 6）。

### 2.7 跨挂起点存活值：一种分析，两个用途

- **跨线程迁移**：挂起的计算若要换线程恢复，跨挂起点存活的值必须满足 Send。
- **多次恢复**：同一 K 恢复多次时，跨挂起点存活的值会被复制，必须满足 Clone，Once 资源不能出现。

Rust 能对 Future 计算 Send，是因为状态机本身是一个类型，跨 await 存活的值就是它的字段。有栈实现没有“帧类型”，Flurry 只能把它算成函数摘要：每个函数记录跨可挂起调用存活的值的性质，沿可挂起调用向上合成；行多态的闭包则需要在函数类型上携带这份摘要，或者按保守情形处理。两个用途共用同一份分析（问题 9、10）。

---

## 三、协议分层（候选，名称均为占位）

| 层 | 归属 | 内容 |
|---|---|---|
| 路由 | 编译器 + handler 能力 trait | evidence 构造与传递；错误侧已有 HandleError，效应侧对应 9-18 草稿中的 HandleEffect |
| 控制契约 | 编译器 | perform 视为不透明调用；中止通道；尾恢复为尾切换；外层入口槽位（若问题 6 选 A） |
| 控制后端 | 库，trait 实现 | 进入 prompt、捕获、恢复、放弃；切换时保存哪些上下文；切换本身可用内联汇编实现 |
| 栈存储 | 库 | 池、守护页、物理页提交与回收策略 |
| 调度 | 库，runtime 作为 handler | 就绪队列、Future、唤醒、取消、作用域等待 |

切换时必须一起保存、恢复的上下文成员：

- 被调用者保存寄存器、栈指针、恢复地址；
- Windows 下 TIB 中的栈上下界（否则栈探测与 SEH 失效）；
- 若采用线程局部路由（问题 13 的 B），保存 handler 链头；
- 若采用外层入口重绑定（问题 6 的 A），保存对应槽位。

---

## 四、池化后端的算法骨架（同步，不需要运行时）

前置：handler H 至少有一个可捕获分支。以下为内部步骤，不是 API。

1. **安装**：从线程本地空闲表取得栈 s（命中时没有系统调用）；在 s 底部放置 trampoline 帧，记录 parent 为当前栈；切到 s 运行主体，evidence 指向 H 的入口桩。
2. **perform 被捕获**：入口桩在当前栈保存上下文；把“s 到当前栈”的子链作为 K；切到 parent，在 parent 上执行分支(x, K)。
3. **尾恢复**：弹出分支帧；把 K 的栈链重新接到当前栈之上；切入，入口桩返回 y。
4. **非尾恢复**：保留分支帧，其余同 3；主体下一次 perform 时，下一个分支在该帧之上运行。
5. **主体返回 a**：trampoline 切回 parent，执行正常完成路径 a → B；s 归还空闲表。
6. **放弃 K**：以中止信号切入；主体沿中止通道逐帧清理到 trampoline；trampoline 切回 parent 并归还 s。

不变量：

- 每条池栈在任一时刻至多属于一个 K 或一个运行中的计算；
- K 被消费后不能再次切入；
- s 归还前，没有任何 K、evidence 或已交付的指针仍指向 s 内的帧（由问题 5 的生存期规则保证）。

代价：

- 每次 yield 两次切换，与栈深度无关；
- 每次安装一次空闲表弹出；
- 每条存活的栈至少占用一页物理内存；`madvise` 回收放在冷路径批量执行。nessa 现在用全局 Mutex 保护整个池，热路径应改为线程本地空闲表。

在此之上的异步：

- runtime 在每个 task 根部安装一个可捕获的挂起类效应 handler，并处理 spawn；
- await 未就绪时 perform 挂起，runtime 把 K 登记到 Future 的等待者；完成时把 K 放入就绪队列，运行循环恢复它；
- 取消就是放弃 K，与第 6 步共用中止通道；
- 结构化并发的作用域就是 async 分支所在的 handler 安装，退出时等待或放弃子任务；
- 完成通知通过 runtime 自己的数据结构传递，不需要 nessa 那种 safe-point 中断。

---

## 五、需要你拍板的问题

### 控制与清理

1. **可捕获性在哪里声明？**
   - A. 只在 effect 声明中写 catch 参数（abi-3.0、nessa 方式），该 effect 的所有处理都捕获。perform 点静态知道可能挂起，但所有 handler 都要付切换代价。
   - B. 只在分支中写 catch（`algebraic-effect-and-delimited-continuation/prompt.fl` 方式）。任何 effect 都可能被捕获，每个 perform 点都要按“可能挂起、可能放弃”编译，runtime-purity 摘要中的“挂起”完成方式无法精确。
   - C. **（推荐）** 声明许可、分支选择：effect 声明标明可捕获，分支可以选择捕获或直接返回。perform 点的挂起信息静态可知，直接分支仍走调用/返回。
   - Other: …

2. **放弃（以及取消）的清理机制**
   - A. **（推荐）** 隐藏中止通道：以中止信号恢复，perform 点转入与错误传播相同的返回路径，逐帧执行 drop elaboration，到 prompt 边界停止；用户 handler 不能匹配它。不需要展开器运行时，代价是每个可挂起调用之后多一次判断。前提是 Flurry 的错误按带标签返回降码，这一点也需要你确认。
   - B. 表驱动展开（类似 C++ 零成本异常）：正常路径没有额外指令，但需要展开器运行时和每帧展开表，C 帧也必须带表。
   - C. 静态拒绝放弃仍持有清理义务的 continuation：生成器提前结束、任务取消等常见用途会被大量拒绝。

3. **直接分支中的 `error`／`return`／`break`**（2.5 的例子）
   - A. **（推荐）** 视为放弃主体：主体帧按问题 2 的机制清理，错误在边界外交付，主体内部的处理不截获，例子输出 `99`。与现行 handlers.md 一致。
   - B. 禁止直接分支非局部退出；需要时改写为可捕获分支并显式放弃。实现最简单，但 `read_number() => error …` 一类写法全部被拒绝。
   - C. 错误在 perform 点所在上下文传播（nessa 做法），例子输出 `0`。与现行 handlers.md 冲突，选它需要修改该文档。

4. **`resume` 语句（grammar：`resume expr? (while expr)?`）的含义**
   - A. **（推荐）** 在分支中交付 Y 并结束本分支；在可捕获分支中保证尾切换，父栈不增长；非尾恢复通过 continuation 值上的显式操作完成。
   - B. 只用于直接分支（提前交付 Y，类似 nessa 的 `resume 10 if …`）；可捕获分支只能显式调用 continuation，尾切换另行规定。
   - C. Other: …

5. **continuation 的生存期**（细化 066 第 2 题）
   - A. **（推荐）** 不超过捕获它的 prompt 的动态范围：可以存入该 handler 自己的状态（例如调度器队列），handler 安装结束前必须恢复或放弃。prompt.fl 中 `queue.push_back(k)` 后分支结束、handler 表达式随即完成的写法，在这一规则下被拒绝，除非 queue 由该 handler 自己持有。
   - B. 用可达性类型约束：K 的类型携带其依赖（挂起的栈、handler 环境），是否可逃逸由现有 `&expr T` 规则判定。可以越过 prompt，但必须配合问题 6 的 A。
   - C. 不可离开分支：最严格，调度器无法在库中实现。

6. **恢复点的外层效应入口**
   - A. ε 在恢复点取得入口；主体不跨可挂起调用缓存外层 evidence，改经 prompt 帧槽位读取。问题 5 选 B 时必须选这一项。
   - B. ε 固定为捕获时的入口。只在问题 5 选 A 或 C 时成立，因为那时两者相同。
   - C. Other: …
   - 附带确认：abi-3.0 中“动态的……挂起后可在其他上下文中恢复”是否就是 A 的语义？

### 后端与存储

7. **是否需要无栈（状态机）后端？**
   - A. **（推荐，第一版）** 不需要，只有有栈后端，主体代码与后端无关。代价：每个存活任务至少一页物理内存，一百万个并发任务约 4 GB。
   - B. 需要，作为按函数选择的选项。按后端实例化（066 的方案）在这时才是必须的，也会引入函数着色。
   - C. 以后再议，但协议不排除它。

8. **后端选择粒度**
   - A. **（推荐）** 每次安装 handler 时选择，经协议供应（类似 implicit 供应），缺省使用标准库的池化后端。
   - B. 整个程序或整个包配置一个后端。
   - C. Other: …

9. **多次恢复**
   - A. **（推荐，第一版）** 不提供。
   - B. 提供，换入同一地址区间实现，每次切换代价与栈段大小成正比；要求跨挂起点存活的值都可 Clone（2.7 的分析）。
   - C. 作为 unsafe 操作提供，不做检查（nessa、libmprompt 做法）。

### 并发

10. **挂起的计算能否跨线程迁移？**
    - A. **（推荐，第一版）** 不能：任务固定在线程上，只窃取尚未开始的任务。
    - B. 能：编译器计算 2.7 的摘要并要求 Send；函数类型需携带摘要。
    - C. 能，由用户 unsafe 断言。

11. **抢占**
    - A. **（推荐）** 只在类型化的 perform 点挂起（协作式），与“异步完全基于代数效应”一致。
    - B. 编译器在循环回边或调用点插入轮询（nessa 方式）。这是一种不出现在效应行里的挂起，与效应类型以及 P5“无挂起”的判定冲突。
    - C. 基于信号的异步抢占。冲突同 B，还会在持锁、线程局部访问处产生新问题。

12. **async 的接入形态**（9-29 草稿写作 `http.get(…).async(runtime)` 与 `future.await(runtime)`）
    - A. **（推荐）** `.await` 像 `.ref` 映射 Reference 一样映射到一个 trait 协议；runtime 作为 handler，用 `.use(runtime)` 或 `# { }` 安装；`async` 分支（grammar 已有 `async pattern`）是“在 spawn 效应下运行分支体”的糖。这样不需要把 runtime 作为实参传入。
    - B. runtime 作为显式实参（9-29 写法）。`.await` 需改为可带参数，grammar 要修改。
    - C. runtime 由 implicit 参数供应。

### 边界

13. **C ABI 函数与效应**
    - A. **（推荐）** `^attrs.abi(.c)` 函数的效应行必须为空；需要效应的回调由 Flurry 侧包装。这样动态查找没有必需的使用场景。
    - B. 允许，入口从线程局部 handler 链取得（2025-6-10 草稿方向）；后端切换时须保存该链头，且与问题 10 的迁移冲突。
    - C. 动态查找作为一般回退（nessa 的 Any 退化）。需要先确认 Flurry 是否存在对应的 runtime Any 场景。

14. **挂起栈上的 C 帧**
    - A. **（推荐）** 允许挂起与恢复，禁止放弃；静态无法排除时在运行时 trap。
    - B. 禁止跨 C 帧捕获，运行时检测后 trap。
    - C. 允许放弃并跳过 C 帧，相当于 longjmp 越过 C 帧，C 侧资源泄漏。

15. **`inline effect`（9-19 草稿）**
    - A. 像 quote fn：必须完成实例化与内联，否则编译失败；行中含 inline effect 的函数不能擦除为函数指针或 trait object。
    - B. 像 inline：只表达意向，无法内联时退回普通 evidence。
    - C. 取消这个修饰，交给优化器。

### 文档与草稿之间的不一致（只提问，不选边）

I1. **handler 绑定模式。** `docs/traits/protocols.md` 示例写 `let handler handles LimitErr = ClampTo { limit: 8 }`，但 `docs/grammar/syntax` 的 pattern 产生式中没有 `id handles expr`。
   - A. 在 syntax 中补 `handler_decl_pattern -> id handles expr`（9-18 草稿中的写法）。
   - B. 改写示例，去掉 handles 绑定。
   - C. Other: …

I2. **catch 的位置。** grammar 把 `catch_param` 放在 `parameter` 中（effect 声明、函数参数可用），pattern 中没有 catch；parameter-elaboration.md 的例子把它放在 trait 方法参数里；abi-3.0 放在 effect 声明里；`algebraic-effect-and-delimited-continuation/prompt.fl` 写成分支模式 `suspend(catch k) =>`。这与问题 1 绑定。
   - A. 只允许参数位置（声明与 handler trait 方法），分支通过形参名绑定。
   - B. 在 pattern 中增加 `catch pattern`。
   - C. 两处都允许，分别承担问题 1 C 中“许可”与“使用”。

I3. **`.await` 的形态。** grammar 是无参后缀关键字 `expr.await`；9-29 草稿写 `future.await(runtime)`。见问题 12。

I4. **`inline effect`。** grammar 的产生式是 `algebraic_effect -> async? effect …`，没有 inline。见问题 15。

I5. **continuation 类型名与参数。** 文档写 `builtin.Continuation`，abi-3.0 写 `AnyContinuation`，nessa 写 `Continuation`。
   - A. 保持 `builtin.Continuation`，不带类型参数。
   - B. 改为带 Y、B（及外层行）参数的形式。
   - C. Other: …

### 开放问题

O1. 9-29 草稿的错误消去中有一条 `catch _ => return,`。它匹配什么？
   - A. 取消／中止信号，即问题 2 的中止通道对用户可见的入口。
   - B. 其余所有错误的兜底。
   - C. Other: …

---

## 六、已确认 ／ 待定

已确认（来自现行文档与先前轮次）：

- 处理能力由 trait 协议配合 elaboration 提供；函数签名中没有 handles 段。
- deep handler：分支在当前边界外执行，恢复时重新安装当前 handler；Y、A、B 分开；同名 occurrence 保序。
- 恢复以单次、不可逃逸为基线；放弃需要已定义的清理依据。
- catch 参数当前固定使用 `builtin.Continuation`，操作接近 intrinsic，检查有限。
- 作者目标：ABI 与并发运行时都以协议提供；同步场景能单独使用池化 continuation；异步完全基于代数效应。

待定：问题 1–15、I1–I5、O1，全部未决。本轮提出的“主体与后端无关”“prompt 独占栈 O(1)”“尾恢复保证”“直接分支非局部退出即放弃”“跨挂起点存活值分析”都是候选论断，尚未被作者确认。

## 七、文档写入

否。本轮只写入本 history 文件。

## 检查说明

- 本轮唯一的 Flurry 示例（2.5）已对照 `docs/grammar/syntax` 与 `docs/grammar/index.md` 自检：effect 声明以分号结束；类型前缀 `#[…] !E T`；后缀 `#`、`! { }`；`error expr`；arm 用逗号分隔；以 `}` 结尾的语句后不加分号；中缀 `<=` 两侧有空格；错误枚举名以 Err 结尾、变体沿用 protocols.md 的 `TooLarge` 写法；打印使用 `println`。为避免 `#!` 连写的词法歧义，示例把传播与消去拆成两步。
- text 围栏均为内部执行记号，不是 Flurry 语法，也不是库 API。
- “控制后端”“HandleEffect”等名称为占位，不是已有 API。
- 未使用 luna 编译器源码作为依据。

---

## 作者答复（prompt，2026-10-03）

~~~text
1 C
2 A
3 error qualified type是如此常用，所以实际上它们已经是独立的系统了，现在已经有了较规范的标准，只是按照传统，我们将它放到统一的代数效应主题中。
4 A
5 B
6 A
7 A
8 A，实际上我们几乎必须在标准库中提供改策略，并且大家也会默认使用
9 A，多个task往复重启，实际上每次都是不同的continuation,同样的也不能复制continuation
10 A
11 A
12 A, 自然的
13 A，pipeline backend会自行进行合规检查的，不只是检查是否有效应行
14 A
15 B

grammar是文档统一工作开始前的版本了，protocols.md中的是最新的版本，handler_bind -> pattern handles expr。
注意，如果不是async effect通常指的是handler的async执行，所以handler会在新栈（新task）中执行，这也是async effect的处理pattern要加async关键字、暴露该语义的原因。
同步handler就地执行。
catch的解构就当作普通参数就行了，比如 yield(data, k) => ...
I3，显然我们现在倾向于显式指定runtime实例。
I4，更新语法
I5，A

这个其实是error handling的一个语法糖。
fetch_data()! {
    -- 在error elimination块中，第一个catch arm会在所有arm前执行
    -- 最后一个catch arm会在所有arm后执行
    -- 如果只有一个catch arm，那就视作最后一个
    -- 有了post catch arm后，elimination arm就不默认返回了。
    JsonParseErr { message, ... } => eprintln("Json parse err: {}", message),
    NetErr.Timeout(_, _) => eprintln(...),
    ...
    -- 这里的实际上是e的完整返回值，一个普通的error qualifed type的实例
    catch e => {
        -- 如果是parse err直接返回，否则可以尝试重试
        if e is error JsonParseErr { ... } as parse_err do {
            -- 我们假设caller本身也有该行，一般来说这是优秀的设计
            return error parse_err;
        }

        if retried < max_retry {
            retried += 1;
            continue retry_loop;
        } else {
            return error NetErr.RetryExhausted(max_retry);
        }
    }
}

另外，我还有一种direct passing as implicit的abi想法，
这其实是一种借用了代数效应系统的implicit arguments（flurry中已有的隐参系统跟scala等语言中的角色和机制差异很大，因此如果你想用scala那样的隐参，这是一种方式）。
考虑一组函数，它们共享很多参数，但是并不完全相同，此时我们可以将相同的部分抽象出来：
effect enum Args {
    -- 显然effect enum的variant不能复用enum variant的语法
    -- 要求1：不能有参数
    -- 要求2：必须有属性标准，在effect enum的variant上或者裸的effect上
    ^attrs.effects.direct_passing
    some_runtime() -> *Runtime,
    ^attrs.effects.direct_passing
    diagnostic_context() -> *DiagCtxt,
    ...
}

-- 于是，编译器会在handler安装时，直接执行handler，并把需要Args的函数加上对应的隐式参数，把handler的执行结果传递下去
fn foo(name: String) -> #Args void {
    ...
}

-- 对比把它们都包装一个大context,这样做的好处是更灵活、覆盖起来更方便，且样板代码更少，且能复用很多代数效应的特性，并且本身也不与前者的风格冲突
-- 与fn template的对比也是更灵活，但是对样板代码的处理手段没有前者厉害。
-- 比如bar只需要一个参数的话：
fn bar(name: String) -> #Args.diagnostic_context void {
    ...
}
~~~
