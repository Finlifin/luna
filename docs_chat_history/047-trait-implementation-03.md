# Trait / implementation：恢复审阅、依赖交付与接收者适配

- 日期：2026-09-26
- global_index：047
- theme：trait-implementation
- subindex：03
- 本轮目标：消化作者对 046 的新增答复，为 runtime dependency substitution 和更积极的 receiver adaptation 提供可审阅方案。

## 作者输入摘要

作者说“好了，我们回归文档编写。已在 prompt 中回复”。先前暂停记录和 AGENTS 文档循环的要求由本轮明确恢复。

prompt 的新增内容：要求设计易用且 elaboration 友好的 substitution 显式绑定、推导、传递机制，尤其覆盖 impl 的 runtime dependency；说明无孤儿约束的 deriver 是早期探索，现考虑 receiver adaptation 改善用法；明确指定策略改为 `derive Serialize<.json>.use(by_repr) for Student`；指出 handling capability 已给出作者版本；要求优化过于保守的 auto dereference/reference。

旧 R1–R4、R8 的助手预填没有新增作者确认。R6 的“是的”连同历史解释一起保留，不单独扩大为批准整套局部能力世界。作者 prompt 全文追加到 046，以下审阅保留已确认/提案边界。

## 完整审阅正文（与 prompt 本轮追加一致）

# 本轮审阅：依赖替换的交付与接收者适配

日期：2026-09-26；history：047；theme：trait-implementation；第 03 轮。

**DRAFT — NOT CANONICAL。** 恢复文档协作和逐轮记录。本轮先收束你在 R5、R6 和末尾提出的新要求；下面由我预填方案供修改。未改动的旧预填不视作批准，不重复搬回整套旧问卷。

## 1. 对本次回复的理解

你要求 substitution 的显式绑定、推导和传递形成统一、易用且适合 elaboration 的机制，重点是 implementation 的 runtime dependency。你也要求扩大 auto dereference/reference 的能力，而不是维持当前正文的保守边界。

指定 deriver 的写法已经明确为：

~~~flurry
derive Serialize<.json>.use(by_repr) for Student;
~~~

这里按你的决定复用 `handler_application -> expr.use(expr)`，其结果仍作为 derive 的接口表达式；不为 deriver 选择另造 projection 或专属应用节点。上面只是调用片段，Serialize、by_repr、Student 的声明沿用你的例子。

你指出“绕开孤儿规则”的 deriver 文本属于早期探索，现在可以借 receiver adaptation 改善使用体验。我的理解是：优先让用户通过明确视角使用本地适配类型的实现，而不是默认授予派生器全局注入外部实现的特权。你在 R6 的“是的”紧接着解释了历史来源，我不把它扩大解释为批准我先前提出的整套“局部能力世界”。下面给出更小的方案供审阅。

handling capability 以你给出的 `/home/fin/vilya/examples/2026-9-18-handling-capability-by-trait-system.fl` 为讨论起点，不继续推进旧 ClampTo 草案。该文件中的行和 handler 规则仍按具体声明区分，不把整个草稿一并视为正式规范。

## 2. 我建议的共同表示：一份替换，两种交付

实现匹配产物可以用以下**内部分析记号**表示，不是新增源码类型或编译器 API：

~~~text
实现证据 = (声明身份, 完整接口实例, 参数替换 σ, 要求的证据, 环境取得计划)

σ 的条目可以是：
    comptime 参数 -> 已确定的值或仍待求解的符号项
    runtime 依赖参数 -> 确定的绑定 identity

环境取得计划：
    依赖 identity -> 当前操作如何取得对应运行时对象
~~~

**知道依赖是谁，与当前能拿到它，是两个检查。** Tensor 的类型匹配可以确定 allocator 的 identity，但跨函数返回或 dyn 封装后仍要交付对象。反过来，作用域里恰好有一个同类型 allocator，并不能证明它就是目标依赖。

这不要求每个值增加环境字段。已知的运行时依赖类型规则允许共享外层环境、隐藏参数或必要的封闭携带者。这里补的是：由哪份实现证据要求这些依赖，以及如何使所有调用使用同一份替换。

普通函数、方法、闭包调用和自动清理可以共享这个表示。它们的触发规则不同：方法在调用点需要环境，Drop 在每条实际清理边上需要环境；不能到清理时才临时重新选择一个依赖对象。

### S1. 推导依赖时，允许哪些来源？

以 Tensor 为例，建议从目标和完整 trait 实参解出 T、A、allocator；必要时从已经交付的实现证据取得环境。SomeType 的自由 space 若没有这些来源，就需要显式绑定。

- A. 从显式绑定、匹配输入和已携带证据统一收集约束；无来源则留下具名缺口，必须在能力交付前补齐（建议）。各来源必须一致，不按“哪个优先”覆盖不同 identity。
- B. 除上述来源外，允许从当前作用域寻找唯一同类型对象。短代码更少，但增加一个局部声明可能改变选择和缓存依赖。
- C. implementation 的所有 runtime 依赖都必须每次显式传入。检查简单，但重复 notation 多，自动清理还需另设入口。

**助手预填：A。** 所谓推导，是恢复已有关系；所谓绑定，是提供此前缺失的关系。显式传参也不能把目标中已固定的 allocator 换成另一个对象。无来源的依赖不通过名字相同或当前内容相等来补齐。

### S2. 显式补齐开放 implementation，是否复用范式应用？

我建议先统一“选接口，再补参数”的表达式组合。候选表面片段如下：

~~~flurry
-- DRAFT：新增的是开放实现视角的应用语义，不是新 AST 节点。
some.as(MyTrait)<.space = space>.operation();
~~~

MyTrait、operation 是占位接口与方法，不是标准库 API。这里的 space 归属于已选实现的开放参数，不是给 MyTrait 擅自加参数，也不是给 operation 加普通实参。现行语法能组成这棵表达式树，但**现行语义尚未规定实现视角可这样应用**。

对于 `quote space` 一类依赖槽，绑定保留 runtime 对象的 identity 与合法取得关系，不在 comptime 读取其内容，也不因写在尖括号里就复制该对象。这个应用必须复用依赖参数的阶段契约，不能降成接收任意 runtime 实参的普通 comptime 求值。

若同一查询仍有多份实现候选，命名参数只能在隔离的候选环境中匹配；不能因为先看到某个 impl 的参数名，就提交它。该设计还意味着这类开放参数名成为可被用户引用的接口，重命名不能再被当作完全私有改动。

- A. 采用上述组合；参数名对应所选实现公开的依赖槽，应用只封闭既有开放参数（建议）。普通非开放值仍不能随意接受尖括号参数。
- B. 首版要求把依赖放进 target 类型参数，由普通类型应用和构造绑定。无需开放实现视角的新语义，但 SomeType 的原写法需要改成依赖类型或本地携带类型。
- C. 使用具名普通工厂来构造携带能力的值；工厂参数是公开 API，impl 参数保持私有。用户入口更稳定，但需给每类开放实现提供工厂。

**助手预填：A 用于直接表达开放实现，库可以用 C 包装常用入口。** A 与 C 这里比较的是基础语言是否提供 A；只采用 C 时不隐含存在 A。先确认这层语义，再写完整的定义和调用例，避免用一段看似可运行的代码掩盖尚未确定的能力承载方式。

### S3. 被绑定的能力如何保存，尤其是 Drop？

我建议区分短期调用视角与需要跨边界保存的能力：

~~~text
短期调用：
    选择 impl -> 封闭依赖 -> 使用一次视角调用

跨边界携带：
    值本体 + 必要实现证据 + 环境取得关系 -> 一起交付

具有自动清理责任的值：
    建立清理责任时固定所需证据 -> 移动时传递 -> 每条退出边兑现
~~~

这不是规定三个源码类型。具体何时有物理环境，仍由布局和逃逸需求决定。

风险最明显的是：为同一个待清理对象，先建立 Drop 使用 space1 的视角，再建立使用 space2 的视角。如果视角建立能随意改写清理责任，那么看似普通的成员适配就改变了资源行为。

- A. 普通调用视角可短期存在；需要持久使用时显式携带。Drop 的依赖随清理责任建立而固定，普通 `.as(...)` 不改写它（建议）。首次绑定清理责任必须接入构造/交付，不能留到销毁时搜索。
- B. 首次使用某种 trait 就在原对象上永久登记绑定。调用简短，但分支、别名、首次使用顺序都会影响后续行为，需要新的对象状态协议。
- C. 开放实现仅支持显式调用，不参与自动 Drop；自动清理依赖只能来自 target。限制明确，但不能统一所有开放实现。

**助手预填：A。** 如果 SomeType 的唯一合法 Drop 依赖 space，那么创建这种资源时就要完成相应绑定；不能先把资源交付到会发生清理的程序路径，再期待后续方法调用补齐。具体构造与绑定之间的失败路径、责任转移归资源主题细化；本轮不会把 S2 的一次方法调用写成已经解决自动 Drop 的完整例子。

函数只使用已有依赖能力时，可以沿明确的参数契约传递隐藏环境；函数若需要调用者新提供 space，就必须让这一要求出现在可检查的接口中。局部 body 中发现缺口，不等于可以自动向所有调用者增添一个未声明的依赖。

分支汇合也必须保留来源：space1 和 space2 不同，不能因为两边本体都叫 SomeType 就抹平能力环境。若选择封闭表示，需要实际保留分支对应的证据；若接口不容纳这种表示，应在边界要求显式处理。这里不默认为所有分支合成新 union 或 dyn 类型。

## 3. 接收者适配：扩大可达路径，同时明确顺序

当前正文 [receiver-adaptation.md](docs/type/receiver-adaptation.md) 规定 Deref/Reference 只用于显式操作；[member-projection.md](docs/type/member-projection.md) 要求定义端固定成员来源。你的新要求意味着前者要扩展，但不必放弃后者。

我建议：**先形成有限的有序候选路径，记录路径要求，最后只执行选中的路径。** 自动解引用方法可能带副作用、需要环境或产生临时值，查找本身不能运行它，更不能把失败候选的适配代码也执行一遍。

### A1. 自动解引用与自动取址开放到什么程度？

- A. 方法接收者允许多步内建解引用及唯一确定的 Deref 路径；选定方法要求指针接收者时，再做一次必要的取址或 Reference 适配（建议）。Reference 由目标接收者需求触发，不反过来无限产生新查找主体。
- B. 允许任意组合 Deref、Reference 与 cast，寻找可用方法。覆盖更多程序，但需要处理大量路径、环、优先级和不同副作用路径的歧义。
- C. 只开放多层内建指针处理，重载 Deref/Reference 仍显式。实现最小，但不能充分满足智能指针等包装的使用体验。

**助手预填：A。** 内建取址用于合法可取址对象；自定义 Reference 若存在多份同优先级可行转换，不猜一个。所有路径上的 trait 查询都使用同一 implementation 选择机制，保留真实效果、依赖和资源要求。

例如在缺少原主体同名方法的前提下：

~~~text
原接收者类型 Box<Widget>
Deref 的确定结果为 *Widget
Widget 方法要求 *self

候选动作：调用已选 Deref -> 得到 *Widget -> 调用已选方法
不是：执行 Deref -> 复制整个 Widget -> 再取新 Widget 的地址
~~~

这是内部路径例，Box 不被声明为 Flurry 已有标准库类型。Deref 的精确协议以该主题最终定义为准。

### A2. 视角顺序、适配深度和 impl 精度怎样组合？

这三个次序不能简单相加成一个分数。你确认的结构精度解决的是同一主体查询的实现选择；它没有说“另一种接收者上存在更具体 impl，就能跨路径抢走方法”。

建议按以下次序查找：

1. 显式接口视角先限定成员来源；类型视角则保留现有从后往前的顺序，例如 D、C、B、A。
2. 对当前类型视角，从原接收者形态开始，再逐层考虑解引用后的形态。
3. 每个形态内部遵守已有能力/成员来源规则，trait 查询按 normalized IR 结构精度选择实现。
4. 所选方法的接收者需求触发末端取址。不同 trait 的同名成员不会因为一个少做一次取址就自动消歧。
5. 优先位置未决时保留义务；只有已证明不可用才越过它。选定来源后，参数或资源契约失败应诊断该调用，不能用另一来源把错误藏掉。

- A. 采用上述嵌套顺序（建议）。显式视角表达的偏好高于其内部解引用长度；同一位置的实现精度独立计算。
- B. 在所有视角之间优先选择最少适配步骤。可能使显式排在最后的视角输给较早视角，须修改现行视角优先含义。
- C. 只要多个路径都能调用就报歧义，要求用户显式展开。规则直接，但削弱自动适配的便利性。

**助手预填：A。** 定义端固定来源及动作路径中的符号契约；实例化负责兑现，不能见到具体类型新增的方法就重新开始名称竞争。对于尚未决定的推断变量，记录候选查询，不伪装成已经选定的调用。

此方案仍需与旧 R2 一起确认：我建议更高优先候选 Unknown 阻止最终提交，而不是把 Unknown 当作不存在。这里给出统一规则，不另开一套接收者专用的未知处理。

### A3. 是否同时让目标类型引导内建指针读取？

用户提供的 handler 例子在 `*self` 中返回 `self.limit`；当前规则下字段投影得到 `*u32`。如果想减少此类 `.*`，仅扩大方法查找不够，还需有目标类型引导的值适配。

以下是候选规则的完整教学例，前后共享同一字段投影语义：

~~~flurry
struct Counter {
    value: u32,

    fn read(*self) -> u32 {
        -- DRAFT：返回目标 u32 触发合法的 Copy 读取。
        self.value
    }
}

test {
    let counter = Counter { value: 7 }
    let pointer: *u32 = counter.ref.value;
    let value: u32 = counter.ref.value;
    println("{} {}", pointer.*, value);
}
~~~

候选语义下输出 `7 7`。原始字段投影仍是指针；第二个绑定根据显式目标插入读取。没有目标的 `let field = counter.ref.value;` 仍保留字段指针。

- A. 对已有明确目标的检查位置，允许有限次内建指针读取；每步须满足有效性，读出值须具有 Copy 能力；不自动搬出非 Copy 对象（建议）。先从初始化、实参和返回值位置接入。
- B. 仅扩大方法接收者适配，值读取仍要求显式 `.*`。风险更小，但 handler 等例子继续补解引用。
- C. 连任意用户 Deref、Reference、cast 都参与一般表达式目标推断。覆盖更广，但涉及转换搜索和更多隐式效果，应独立定义。

**助手预填：A。** 目标引导的自动读取不变成一般隐式转换，也不把 Copy 当成可从无效内存读取的许可。对非 Copy 值，显式读取或移动仍按资源规则检查。

需要强调：这仍不能使 ClampTo 中返回 u32 的方法兑现任意 `T` 结果。自动解引用最多得到 u32，不能得到任意类型。该草稿的 `impl HandleError<LimitErr, T> ... where T` 若要继续返回 limit，应收窄为 u32；保留任意 T 则须使用能够交付 T 的恢复逻辑或不正常返回的逻辑。此处不擅自改作者草稿，也不把它当作“适配会解决”的例子。

## 4. Receiver adaptation 怎样改善孤儿规则的使用体验

我的建议是沿用你已经给出的本地 newtype 与显式类型视角路线：实现仍属于本地适配类型，调用者可以通过视角使用它，避免每次手写包装和拆包。

~~~text
外部类型 S
本地 newtype LocalS，具有已允许的双向 cast 关系
本地实现 Trait for LocalS
接收者带 S.as(LocalS) 视角

成员调用 -> LocalS 上的实现 + 必需适配动作
不因此产生 Trait for S 的全局实现
~~~

使用便利性的改进，并不单独证明任意嵌套本地身份足以解决跨包 coherence。旧 R3 的 `Pair<A, T>` / `Pair<U, B>` 反例仍存在；本轮先不重复询问整套归属规则。

### D1. 显式 deriver 的产物是否回到普通 impl 规则？

- A. 产物遵守共同的 impl 归属与冲突规则；外部类型需要适配时使用本地类型和 receiver adaptation（建议）。`.use(by_repr)` 选择生成策略，不为产物赋予豁免。
- B. 保留独立的局部能力产物，可不注册到普通实现世界。需要另定作用域、传递和冲突规则；接收者适配不能代替这些定义。
- C. 显式派生可以向普通世界注册外部实现，由下游冲突检查兜底。应明确接受组合两个独立包后失败的情形。

**助手预填：A。** 这是根据你对早期探索的说明作出的收敛建议，不将原 R6 的助手 B 悄悄升级为已确认。默认策略注册的准入与生成产物的准入继续分开；本题只确定产物，不据此推导完整 default deriver 优先级。

## 5. Elaboration 的可实现骨架

上述方案若采用，可以按以下步骤实现，而不要求在一次搜索中解完所有问题：

1. **建立查询。** 记录成员名或完整 trait 查询、所需 phase、接收者类型及视角、现有参数约束、实现世界。运行阶段与 trait phase 仍分开保存，旧 R4 尚未全部确认。
2. **按序展开路径。** 为当前接收者形态检索候选；没有可用来源时才向下一个允许的解引用形态推进。记录动作描述，不执行 runtime 代码。
3. **隔离试探。** 每个候选用独立的推断状态，产生 σ、要求、开放依赖和动作。未获胜候选的推断赋值不得泄漏到公共状态。
4. **处理未决。** 路径中 Deref 的关联结果未知，就等待相应义务；不能编造下一个主体。相同查询状态重复出现报告循环；持续生成不同类型的展开也须受资源预算限制，耗尽报告未决或超限，不能伪称无成员。
5. **确定来源。** 路径顺序与实现结构精度分别计算。候选的显式参数绑定只补充其约束；更高优先候选未排除前不提交较低者。
6. **封闭依赖。** 对所选证据的每个 runtime identity 确定取得方式。已知 identity 却当前无法取得，与根本没有绑定来源，给出不同诊断。
7. **检查适配与调用。** 接收者求值一次；执行选定路径的转换、解引用、末端取址和调用。沿途效果、临时值清理、依赖有效性以及仿射操作进入实际控制流检查。重复调用所需的环境是否还能使用，也按契约检查。
8. **跨边界交付。** 返回、捕获、移动和动态封装时保留必要依赖；自动清理使用已经固定的责任与证据。Any/any 不能充当遗漏环境的补丁。

缓存至少区分三个结果：候选集合、符号化选择/替换、使用点的环境交付检查。结构索引可以广泛复用；带 runtime identity 的语义结果不能仅按依赖类型名合并；代码模板可在证明成立时共享，但共享机器代码不表示两份依赖身份相等。内存状态变化后的合法性检查也不能靠旧的“实现存在”缓存跳过。

验收例按程序行为组织：

| 情形 | 预期 |
|---|---|
| Tensor 目标已固定 allocator1，同时显式传 allocator2 | 绑定不一致，不覆盖原依赖 |
| 两个 allocator 内容相同 | 不以内容相等合并 identity |
| SomeType 缺 space，局部恰有同名变量 | 仍报告缺少明确绑定 |
| 所需依赖只能留在已退出的环境 | 在交付边界拒绝 |
| 一个值移动到新作用域后清理 | 使用随责任传递的同一 Drop 证据 |
| Deref 存在副作用，多个候选被检索 | 只执行最终路径上的动作，次数不因搜索增加 |
| Deref 自环或不断产生新包装类型 | 诊断循环或展开超限，不静默选择另一路径 |
| Pointer 字段有 Copy 值目标 | 按 A3 决定是否插入读取；无目标仍保留指针 |
| 高优先来源已经选定，但资源契约失败 | 报告原调用错误，不换来源继续 |
| 两个 trait 提供同名成员 | 不用解引用成本替代来源消歧 |

## 6. 内容规划与本轮状态

建议正式章节按以下依赖顺序推进：

1. **Trait 实例与实现证据**：声明、参数、关联项、所需 phase；让读者看清一次调用究竟取得什么。
2. **实现检索与选择**：结构索引、精度、适用性和未决；旧 R1–R3 的剩余决定在这里合拢。
3. **Substitution 与运行时环境**：本轮 S1–S3；完整使用 Tensor、有缺口的实现、跨函数交付三个例子。
4. **成员与接收者适配**：本轮 A1–A3；更新类型主题中的相关页面，并接合定义端来源固定。
5. **Derive 与策略**：已明确的 `.use(...)` 组合，加 D1 的产物规则，再定义默认策略。
6. **动态接口与协议入口**：dyn 的环境与 Self 结果另用完整案例收束；handling、pattern、变参等只在 trait 篇解释共同机制，各自语义回到相应主题。

不需要先把所有上层协议写完才能确定依赖交付骨架，但也不借这一骨架预先发明它们的完整 ABI。

**已确认：** 恢复文档流程；新的 deriver 选择写法；需要更易用的 substitution 与更积极的接收者适配；handling capability 采用作者版本继续讨论。

**候选：** S1–S3、A1–A3、D1 的全部预填。旧 R1–R4、R8 以及早先未答基础问题中未被作者明确陈述覆盖的部分仍待定。

**待同步正文：** trait phase 新标记与执行阶段的区别；接收者适配规则；deriver 选择形式。前两项尚有具体语义待审，不按旧正文反驳新设计，也不提前把候选写为规范。

**语法与风格自检：** `.use(...)`、`.as(...)`、投影、范式应用沿用现有组合；S2 明确标注新增应用语义；A3 明确标注新增适配语义。例子使用 `.ref`、`.*`、就地定义的 Counter 与已知 println，分隔符按 grammar 核对。内部算法与未定协议使用 text 记号，没有伪造可编译的 Flurry 实现。未运行当前编译器作为语言语义验证。

**本轮落盘：** 原 prompt 全文追加到 046；本审阅追加到 prompt，并完整保存到 047。正式 docs 暂不写入这些待确认规则。


## 过程回复

> 我先读取 `prompt`，核对它对应的审阅检查点和已有记录，再恢复文档编写与本轮记录。

> 这次回复的重点是 runtime dependency 的替换机制，以及更积极的接收者适配；指定 deriver 的 `.use(...)` 写法也已明确。我会把这两条机制连起来推演，尤其检查自动 `Drop` 的依赖从哪里取得，以及适配后能否稳定地选定实现。原来未修改的预填选项仍保留为待确认。

> 我找到一个需要分清的边界：从 `Tensor` 的类型恢复 allocator、把同一 allocator 传过函数边界，以及给实现中没有来源的 `space` 绑定对象，是三种不同操作。它们可以共用替换表示，但不能都靠搜索当前作用域解决。
>
> 接收者适配可以扩大。我准备提出一套候选规则：有序查找视角和解引用路径，选定后才执行适配；实现的结构精度仍只在对应主体的候选中比较。这样能减少显式操作，也能保留可解释的选择顺序。

> 新的审阅已整理成 7 组预填，先讨论依赖来源和绑定，再讨论适配顺序。另有一个独立问题已标出：handler 草稿从 `self.limit` 最多能得到 `u32`，即使扩大自动解引用，也不能兑现任意 `T` 的返回类型。
>
> 我正在核对原回复归档与新增审阅的一致性；这一轮的候选规则仍留在 `prompt` 中供你修改。

## 检查与写入范围

读取现行 grammar/style、成员投影、接收者适配、运行时依赖类型、两相正文、044/046 记录，以及作者的 handling capability、implicit substitution、derivation 草稿。未以现有编译器代码裁决语义，未运行编译器测试。未启用子代理。

只修改 prompt、046，新增 047；正式 docs、proposal 草稿和现有用户编译器改动未修改。写入前以原始字节校验 prompt，防止覆盖作者同时编辑。

## 完整最终回复

已恢复文档协作，并把下一轮预填审阅追加到 [prompt](/home/fin/luna/prompt:796)。这轮集中在两件事：

- **依赖替换**：区分“确定依赖是谁”和“运行时如何取得它”，给出显式绑定、环境传递及自动 Drop 的候选规则。
- **接收者适配**：扩大自动解引用、取址，并提出由明确目标类型引导 Copy 读取；同时保留成员来源与实现选择的确定性。

指定 deriver 已按你的 `.use(by_repr)` 写法推进。需要特别注意：自动解引用可以把 `*u32` 读成 `u32`，但不能让 handler 返回任意 `T`。

共 7 组预填，建议先审 S1–S3。原回复已归档到 046，本轮完整记录在 [047](/home/fin/luna/docs_chat_history/047-trait-implementation-03.md)；待确认规则尚未写入正式正文。


## 作者审阅与写入授权——2026-09-26

作者原话：

> 已审阅。剩下两项语义我认为非常容易推导出一个合理的结果，不用赘述了。可以编写本主题文档了，然后我们进入资源管理主题。另外，你忘记在回合后清空prompt文件了.

本轮读取的 047 审阅段未包含逐项新增改写；以上直接消息作为审阅批准及授权收束剩余边界的依据，不虚构额外的逐题回复。作者同时要求归档后清除已审阅 prompt 内容，下一主题只保留新一轮预填。

### 作者答复载体全文（prompt）

下面逐字保存处理前的完整 prompt，包括更早轮次与助手预填；其保留不表示作者分别重新作答。

字节数：73649；SHA-256：`d1d8ef82a33febe43efd403f4e6a4525c422d0f806c415db82780a5ae1e35794`。

``````text
# Trait / implementation：接口身份、实现选择与派生

日期：2026-09-13；history：044；theme：trait-implementation；第 01 轮。

**DRAFT — NOT CANONICAL。** 类型章节已经按授权编写；本稿开始新主题。下面仍由我预填建议，你可以直接修改整段答复或个别规则。候选没有因预填而获得确认。本轮不据此编写 trait 正文。

## 一、先建立全貌

我认为这一主题的核心是“一个操作凭什么由这一份实现提供”。声明一个方法容易；困难在于参数未知时选择什么、不同包能添加什么、生成实现何时成为事实，以及动态调用怎样保留已经检查过的关联关系。

类型章节已经把几个接口留清楚了：PTC 可以证明某 trait 实例成立，但不会替 coherence 发明规则；成员来源在定义端固定，但具体实现可在实例化兑现；newtype 隔离接口，type view 通过显式视角接合；运行时依赖需要交付环境，但不预定 dyn 一定怎样存。

因此，我建议先把 trait 的身份与实现选择收束，再进入派生和动态接口。这一轮同时触及你此前留下的 Serialize 例子，防止基础篇只讨论抽象 coherence，却无法解释真实用法。

### 已确认的地基，不重新投票

| 事项 | 采用边界 |
|---|---|
| 两相 | comptime/runtime 方法分相，缺一相不能由另一相补齐；共享 implementation 候选与 coherence |
| 同名定义 | 不以普通函数重载区分本应唯一的同名定义；implementation 匹配是另一问题 |
| 成员来源 | 定义端选定来源；实例化兑现该来源，不能悄悄改成另一个同名成员 |
| PTC | 满足性来自有效要求或实现证据；无候选不自动为否定；Eq/Hash 缓存作用不等于逻辑公理 |
| loose | 开放定义线性可见，整体实例化时接受最终 coherence 检查 |
| 类型身份 | 普通数据声明名义区分；alias 不增加身份，newtype 不继承底层接口 |
| 适配 | type view 保持原类型、逆序考虑候选；Deref/Reference 不自动进入查找链 |
| 派生意图 | 可以定义并选择不同策略；你给出了 Serialize 的 debug/json/toml 等策略 |
| default deriver | 你明确写过孤儿约束及本地类型之前的量化参数限制；如何精确映射到全部实现规则见第 4 题 |
| 指针 | 现在只有指针；是否需要 unsafe 留到程序验证，不在本轮重问 |

依据：[两相正文](/home/fin/luna/docs/comptime/the-comptime-phases-of-traits.md)、[类型总览](/home/fin/luna/docs/type/index.md)、[037 中的作者派生例](/home/fin/luna/docs_chat_history/037-type-system-09.md)。proposal_trace 只用于理解意图，不作为规则来源。

## 二、我的内容规划

| 章节单元 | 需要给读者的答案 |
|---|---|
| Trait 的身份与完整实例 | 声明了什么；主体、参数、关联项分别处在什么位置 |
| implementation 与 inherent 成员 | 提供什么；成员如何属于接口或类型 |
| 适用性、约束与实例化 | 未知参数时能否检查，何时产生具体实现 |
| Coherence、重叠与包边界 | 哪些实现能共存，第三方能添加什么 |
| 关联项、默认项与 supertrait | 缺什么必须提供，默认代码怎样检查 |
| 两相与生成实现 | 复用已定稿阶段契约，补本主题实际例子 |
| 成员选择与显式资格 | 与类型篇接合，解释参数化 trait 的方法访问 |
| Derive 与策略 | 策略的身份、可见性、选择、产物和失败 |
| dyn 与其他动态接口 | 可以封装什么、关联项怎样交付、环境和资源如何保留 |
| 委托与封闭分派 | using、tagged polymorphism 是否采用及各自展开契约 |
| 实现算法与验收 | 候选索引、条件交集、关联项规范化、证据、诊断与缓存 |

每个完整单元仍按七段写；不把默认派生器写成语言对所有类型结构的隐藏特例。下面先确定能支撑这些章节的规则。

## 三、首轮预填审阅

### 1. Trait 实例的身份与 implementation 主体，先采用哪一层对象？

全局语法允许 impl 后跟表达式，但“表达式”没有单独决定最终必须得到类型，还是也能对某个普通值建立实现。PTC 内部写主体 x 也不替语言裁决这一点。

我的候选基线：trait 声明有身份；完整 trait 应用按声明及语义参数区分；实现主体首先是类型值，可以是参数化或带 runtime dependency 的类型。普通 runtime 对象通过它的类型取得能力，不单独因当前内容生成一个 impl 身份。

- A. 采用这个类型主体基线（建议）。参数可以携带已定义的值或依赖信息，但不增加逐对象 impl 系统。
- B. 同时允许普通值作为实现主体。需要定义值 identity、变更与作用域如何影响候选和关联项，不能只复用类型缓存键。
- C. Trait 采用结构身份。同一签名集可视作相同接口；必须重写声明来源与跨包实现关系。

**预填答复：A。** Trait 的声明身份与完整实参构成接口实例；主体是类型值，动态依赖类型也在其中。某个普通对象是否具备能力由其类型与当前证据判断，不根据当前值内容另造实现。

### 2. implementation 匹配，是否也遵守“已暴露的形态 + 可证明关系”？

类型篇已经禁止任意逆解普通工厂。这里需要防止 implementation 匹配器绕开那条边界。

候选算法：先匹配公开的头与参数结构；产生约束，验证 requires 与已定义类型关系；关联项结果随选定来源取得。普通工厂结果只能按结果身份与契约比较，不把函数名当单射构造子。定义端参数保持刚性，调用产生的推断变量才可赋值。

- A. 采用上述边界（建议）。复用 PTC，遇到未知保留义务；不存在“impl 专用的任意反向求值”。
- B. implementation 的头仅允许有限的声明应用模式。更容易保证搜索，但拒绝一部分普通类型表达式。
- C. 为 implementation 头单独开放可逆计算协议。需要明确注册规则、唯一性和终止边界，不能默认所有 pure 函数可逆。

**预填答复：A。** 匹配依赖已经成立的形态和关系，不能逆解任意工厂；头无法观察或要求未证时保持未决。允许推断的元变量与定义端参数分开。

### 3. 同一主体与 trait 实例存在两个适用 impl 时，首版是否一律冲突？

具体例子是同一个参数化接口：一个泛化实现适用于所有满足 P 的 T，另一个实现也覆盖某个满足 P 的具体类型。两者文本不同、方法体相同或一份来自 derive，都没有自动消除重叠。

- A. 无隐式特化，两个同时适用的独立实现冲突（建议）。要求交集不可满足才能证明互斥；定义端无法证明时按已有 ordinary/loose 边界处理，最终交付不能悬而未决。
- B. 允许严格更具体者胜出。须定义“更具体”的偏序、多个极小候选和跨包稳定性，PTC 蕴含未知也需处理。
- C. 按声明或导入顺序优先。容易实现，但依赖顺序会改变语义，且与固定来源需要额外接合。

**预填答复：A。** 首版不隐式特化，独立 impl 重叠就是冲突。默认成员复用同一实现的代码，不视作第二份实现；derive 产物参与相同检查。后续若需要特化，再定义独立且可检验的机制。

我的判断：A 能尽快得到可靠实现；代价是组合库需要更明确地设计接口参数。B 不是不能做，但绝不能只写一句“具体实现优先”。

### 4. 孤儿规则怎样从你的 default deriver 注释扩展到一般 impl？

需要对照的两段原文：

- [traits/index.md](/home/fin/luna/docs/traits/index.md) 只写“trait 的包或主体类型的包”。
- 你在 [037](/home/fin/luna/docs_chat_history/037-type-system-09.md) 写 default deriver 的约束时，把 Trait 与全部类型参数纳入本地定义判断，并限制本地类型之前出现的量化类型。

两者范围不同；不能因旧文已经是肯定句就忽略参数，也不能把派生器注释无条件推广到所有 impl。

我的整套候选：手写 trait impl 与默认派生产物采用共同包规则；检查序列先主体，再 trait 的显式类型实参；本地 trait 可声明实现，否则需要该序列出现本地名义类型。本地类型之前不允许未覆盖的自由类型参数。typealias 不制造本地身份，newtype 可以；“外部包装中含本地类型”本身不自动使包装成为本地。inherent impl 则归主体定义包。这里的序列与“未覆盖”定义都是待你裁决的建议，不是直接移植 Rust。

- A. 采用上述统一方案（建议）。精确判定实现归属；需要据此写清包归属、序列与覆盖定义。
- B. 一般 impl 只允许 trait 或主体在本地；你的宽规则仅服务 default deriver。规则有两套，需解释为何派生产物享有不同准入。
- C. 你写的是一般 trait 参数序列，主体并不按上述顺序参与。请在预填中改写完整检查序列和包归属对象。
- D. 采用其他包级准入，不通过“第一个本地类型”约束。请给出能判定上述三种情况的规则。

**预填答复：A，作为整套候选。** 检查主体及类型实参，本地名义身份有明确归属；alias 透明，newtype 独立。手写与派生产物最终遵守同一准入。外部包装的本地性不递归传播，除非以后明确指定例外；inherent impl 属于类型定义包。

参考仅用于对照真实机制：[Rust orphan / overlap](https://doc.rust-lang.org/reference/items/implementations.html)。Flurry 是否采用以上各项由本题决定。

### 5. 实现候选的可见范围，是否与普通成员名的导入分开？

“一个 impl 对 coherence 是否存在”和“当前能否直接拼出一个成员名”不必是同一个问题。若导入一个模块能让原先合法的调用改选实现，来源稳定性必须有额外约束。

- A. 在合法依赖图及其静态作用域形成的实现世界中统一检查；普通 use 不隐藏已在该世界中的冲突（建议）。名称可见性单独处理，loose 生成实现仍遵守已确认的线性可见性及最终检查。
- B. 只有显式导入的实现进入世界。需要实现导入语法及世界身份，库间传递证据时也要处理不同世界。
- C. 包内统一、包外按调用者词法导入。实现世界随调用点变化，需要严格说明定义端证据如何保持。

**预填答复：A。** 先以合法依赖与作用域确定候选世界，再做名称访问检查。use 不作为规避已存在 coherence 冲突的机制。这里不扩展包管理和链接器规则；具体包边界由相应专题接合。

### 6. 实现成员是否必须兑现原接口契约，而不能提高调用要求？

这直接决定“证明 Implements 后到底能使用什么”。候选基线是：参数/结果和接收者按接口匹配；实现不能要求调用者提供接口没有要求的更强前提，不能交付更弱结果保证，也不能偷偷增加调用环境必须处理的 effect/error。已有方向性适配若可证明可以使用，但不因“看起来更安全”新增子类型规则。

- A. 采用契约替换基线（建议）。通过 PTC 验证适用的蕴含与表示/路由证据，不要求签名文本完全相同。
- B. 要求规范化后的签名及要求逐项相同。容易检查，但拒绝部分可证明安全的实现。
- C. 实现可以加要求，调用时再看具体 impl。泛型代码只依赖 trait 已不足够，需要改变接口承诺方式。

**预填答复：A。** 调用者满足 trait 契约即可使用实现；实现不能强化外部要求或削弱交付保证。可接受差异必须由已有关系和实际适配支持，不凭此开放任意参数逆变或结果协变。

### 7. 关联项、默认成员与 supertrait，采用怎样的检查责任？

现有 grammar 已允许 assoc 默认值、带主体方法以及 trait 后的 :- 表达式；它没有替我们决定默认代码能假设什么。

候选基线：默认成员在接口已声明的要求下检查；实现替换关联项后复核依赖。无默认的必需项必须提供。supertrait 是实现当前 trait 的必要能力，不能只因为子接口有同名方法就自动生成另一份实现。关联项求值循环不自动作为等式证明。

- A. 采用上述基线（建议）。默认代码可以复用，但保持接口自洽；关联项循环按类型计算与求解能力诊断。
- B. 默认主体只在具体实现处检查。更接近模板替换，需要说明定义端不能依赖哪些保证。
- C. supertrait 同时自动派生父接口实现。需定义产物、重叠及覆盖顺序，不能仅由 :- 语法推出。

**预填答复：A。** 默认成员只能假设声明的能力，必需项要完整；关联项替换后验证所有依赖要求。supertrait 表达必需证据，不自动制造新的实现；递归查询重复出现不是成功依据。

### 8. Derive 策略的产物，是否完全进入普通 implementation 检查？

你的 Serialize 例子已经确立“同一个 trait 可以有多种具名派生策略”。现在需要把策略和最后生成的实现分开，避免策略名成为绕过 coherence 的第二层重载键。

- A. 策略是生成实现的编译时定义；结果完整进入普通检查（建议）。策略身份不同不允许同一主体、同一 trait 实例出现重叠实现；产物验证后才成为可依赖的事实。
- B. 策略名成为实现身份的一部分。一个 trait 实例可以并存多种策略，调用者必须能选择策略；这会扩大成员与动态接口的身份。
- C. Derive 直接注册不可观察的编译器特殊入口。实现成本较低，但与自定义策略及 PTC 保证难以统一。

**预填答复：A。** 策略影响生成什么，不改变生成物的普通语义身份。不同格式参数可形成不同 Serialize 实例；同一实例的两种策略仍会冲突，除非作者明确选择只生成其中一份。

我建议在后续派生专轮再细化 default 策略如何选择、策略的导入，以及失败诊断，不在这里凭属性名推导全部优先级。

### 9. serialize<.json>() 是否正式承担“选择参数化 trait 实例”的语法糖？

这是之前明确留到本主题的问题，不能默默润色掉。你的原例定义普通方法 serialize，却用尖括号选择格式；全局语法中的尖括号应用同时承担保留范式。

已有形式：

~~~text
trait_def -> trait id (:- expr)? clauses? { (assoc_decl | definition | statement)* }
impl_trait_def -> impl expr for expr clauses? { (assoc_decl | definition | statement)* }
normal_form_head_application -> expr<argument*>
~~~

候选 B 的新增局部形式（DRAFT）：

~~~text
trait_selected_method_call -> expr.id<argument*>(argument*)
~~~

它需要规定尖括号参数归属于哪个 trait，而不是默认归于方法本身。

- A. 使用已有显式视角（建议）。格式选择写成 student.as(Serialize<.json>).serialize()；普通 serialize 方法仍是普通调用。
- B. 保留原例简写并正式定义脱糖。仅在可唯一确定参数化 trait 来源时，把尖括号映射到 trait 参数；歧义要求显式视角。
- C. 修改方法本身，使格式属于方法的参数。需要同时解释为何 impl 仍以 Serialize<format> 区分，避免两个格式参数冲突。

**预填答复：A，优先保持现有机制。** 格式属于 trait 实例，显式视角负责选择，成员随后普通调用。若你更看重原例的简洁性，可选 B；届时应把适用位置和唯一来源条件写成局部脱糖契约，不把普通方法偷换成范式函数。

A 的调用片段已按现行 .as(expr)、范式应用和普通调用语法检查；B 明确是待确认增量。

### 10. dyn 的公开契约先要求交付接口，还是固定为两个指针？

需要裁决两端：

- [traits/index.md](/home/fin/luna/docs/traits/index.md) 写“数据指针 + vtable 指针”，并断言无法内联。
- 类型基础允许依赖环境在外层共享，也允许必要时封闭；此前闭包材料又对某些动态依赖实现的 trait object 作了限制，但没有完整说明适用域。

我的建议是先定义可封装条件：调用所需方法、关联项、表示、环境和资源责任必须可交付；comptime-only 操作不能直接成为 runtime 分派入口。具体可调用成员集合仍需动态接口专轮逐项定义。

- A. 不在语言层固定为两个机器字，按接口可交付性规定条件（建议）。常规实现可使用数据加入口表，动态依赖按实际环境处理。
- B. 公开固定为数据指针加 vtable 指针。额外环境必须藏入指定承载对象，并明确哪些类型因此不可封装。
- C. 首版拒绝全部含 runtime dependency 的动态封装。范围清楚，但会限制已经讨论的环境使用场景。

**预填答复：A。** 先定义接口交付，不把一份具体布局当全部 dyn 的语义。动态调用也不能在规范中绝对宣称无法内联；优化若有有效证明可以消除间接调用。确切封装形式、关联项暴露及资源操作到 dyn 专轮收束。

### 11. using 与 tagged polymorphism，本轮是否只把它们列为独立展开主题？

现有 traits/index.md 把 using 写成成员提升、把 tagged polymorphism 写成基于 enum 的分派；这些文字还没有完整回答重名、缺成员、接收者传播及错误/效应汇合。直接把它们当“自动转发所有内容”会掩盖接口边界。

- A. 保留为需要单独定义展开契约的主题（建议）。先完成 trait/impl/derive 基础，随后各用实际例子确定产物与冲突规则；不从旧概述自动继承全部细则。
- B. 本轮将两者统一为同一种 derive 策略。需要说明字段委托和变体分派怎样进入同一接口。
- C. 其中一个或两个不再采用。请在预填中点名，不由我依据新旧程度删除。

**预填答复：A。** 保留方向，后续按具体展开规则收束；类型身份、成员来源和行关系沿用类型篇，不另造隐式继承。没有明确定义前，不把旧例中的默认行为写成新规范。

## 四、这一轮之后如何推进

我的建议顺序是：你回复本稿后，先把实现选择与 coherence 推到能画出候选判定流程的程度；再围绕 Serialize 完整走一次“策略选择—生成—契约检查—调用资格”；最后处理 dyn、using 和封闭分派。

资源失效与指针 unsafe 仍留原专题。类型篇中 enum 混合编码、union 活跃成员和清理、源码恢复形式也没有借本轮变成 trait 规则。

## 五、确认状态与文档边界

- 已确认：上面的地基表，以及本轮已获授权完成的类型章节。
- 作者意图已知但细则未闭合：可选择的派生策略、default deriver 孤儿限制、参数化序列化调用的用法。
- 待确认：第 1–11 题所有新增候选，包括本稿的全部预填答复。
- 正式文档：类型章节已写入；trait 页面本轮仅清除已明确淘汰的机制和同步已确认的示例语法，没有据本稿写入新规则。
- 语法自检：候选与现行形式分开；没有把旧 Serialize 大例中的省略号、字符串运行时插值、未定反射 API 或派生函数语法直接复制进正式文档。


两相新语法：
-- 现在默认的项都是runtime phase的
-- 而有attrs.comptime_phase attribute的则是comptime phase的
trait ConcatPatternView {
    assoc ConcatWith: Type;
    fn concat_pattern_view_prefix(*self, comptime prefix: ConcatWith) -> ?Self;
    fn concat_pattern_view_postfix(*self, comptime postfix: ConcatWith) -> ?Self;

    ^comptime_phase
    comptime fn concat_pattern_view_prefix_comptime(self, prefix: ConcatWith) -> ?Self;
    ^comptime_phase
    comptime fn concat_pattern_view_postfix_comptime(self, postfix: ConcatWith) -> ?Self;
}

impl ConcatPatternView for str {
    fn concat_pattern_view_prefix(*self, comptime prefix: str) -> ?Self {
        if self.starts_with(prefix) {
            self[prefix.len()..]
        } else {
            null
        }
    }
}

{-
-- runtime上下文
let ("Bearer " ++ token)? = request.headers.get("Authorization") else {
    return error HttpErr.Unauthorized("Invalid Authorization header");
}
...
-}
如果想为类型实现comptime phase, 请在derive或impl关键字前加comptime



impl索引优化:
-- 5、类型的多精度语法分解与implementation索引 (Grammaric Decomposition)
-- 由于任意类型的implementation可能出现在包的任意静态作用域中，
-- 当我们寻找implementation的所有candidate时，可能每次都需要穷举所有的implementation
-- 即使改implementation于当前类型无关
-- *这里我们先不考虑孤儿规则，实际上孤儿规则在flurry中需要重新阐述*
impl MyTrait for Vec<T> where T:- MyTrait {
    fn my_method(*self) { ... }
}

impl MyTrait for T where T {
    -- default implementation for all types
    fn my_method(*self) { ... }
}

impl MyTrait for i32 {
    fn my_method(*self) { ... }
}

impl MyTrait for Vec<Tuple<T, i32>> where T:- MyTrait {
    fn my_method(*self) { ... }
}

impl MyTrait for SomethingOther { ... }

test {
    let v: Vec<Tuple<i32, i32>> = ...;
    v.my_method();
    -- 这时，按照标准模型，编译器需要穷举所有的implementation，寻找candidate
    -- 对任意的select查询，我们可能都需要穷举所有的implementation来寻找candidate！
    -- 这显然是不可接受的，即使有一定的缓存机制
    -- 因此，编译器需要一种更精准的implementation索引机制
}

-- 考虑类型的多精度语法分解，任意normalized类型，可以根据语法结构，分解为多颗对原树保留不同精度的抽象树
-- 对于Tuple<Vec<Tuple<i32, i32>>, i32>, 分解为：
-- degree、representation，注意，degree是语法分解的量化精度，即子树高度-1，
-- 当然，最后一级除外，最后一级是类似本身，没有navie type可以继续抽象为Any了，而代入navie type本身，子树高度不变
-- 0、Any
-- 1、Tuple<Any, Any>
-- 2、Tuple<Vec<Any>, Any>
-- 3、Tuple<Vec<Tuple<Any, Any>>, i32>
-- 4、Tuple<Vec<Tuple<i32, i32>>, i32>

-- 那么，此时我们可以按implementation target type最高精度子树来索引implementation
-- impls.insert(Tuple<Any, Any>, the_impl);
impl Tuple<T, T> where T { ... }

-- 注意，语法分解时，如果同级存在Any，并且无法找到navie type带入该Any, 那么就不能再继续分解下去了
-- 所以是不存在Tuple<Vec<Any>, Any>这种分解的，
-- 也就是说，所有的Any都必须在同一级，不能跨级
-- impls.insert(Tuple<Any, Any>, the_impl);
impl Tuple<Vec<T>, T> where T { ... }

-- impls.insert(Vec<Tuple<i32, i32>>, the_impl);
impl Vec<Tuple<i32, i32>> { ... }

-- degree该额外提供了对implementation具体程度的描述
-- 即使在一些歧义的情况下，也能帮我们找到*最详细*的implementation

-- 当我们查询Vec<Tuple<i32, i32>>所有的implementation candidates时
-- 我们先分解Vec<Tuple<i32, i32>>为
-- 0、Any
-- 1、Vec<Any>
-- 2、Vec<Tuple<Any, Any>>
-- 3、Vec<Tuple<i32, i32>>
-- 然后依次
-- impls.get(Vec<Tuple<i32, i32>>)
-- impls.get(Vec<Tuple<Any, Any>>)
-- impls.get(Vec<Any>)
-- impls.get(Any)
-- 并且缓存该candidate查询结果
-- 然后再从candidate中找合理的implementation

-- 这样的索引机制，还额外带来了一个好处
-- 那就是，可以对不完整的类型进行查询
struct Vec where T {
    ...
}

impl Vec<T> where T {
    fn new() -> Vec<T> { ... }
    fn push(*self, T) { ... }
}

test {
    let v: Vec<i32> = Vec.new();
    -- 我们拓展Vec的contextual解析过程
    -- 在我们知道需要给Vec分配一个推理变量之后，
    -- 可以知道 Vec.new() 其实是 Vec<?T>.new()
    -- 即使Vec<?T>是不完整的，也可以进行Vec<Any>查询，由此找到new的定义
    v.push(1);
}
另外，我认为 impl T/impl A for T/derive T/derive ... for T, 也就是单一个T作为target type时，应该用另一套索引机制，
否则太多内容挤到Any下了。
并且，flurry trait resolution的精确度排序选择系统，有且只有normalized后的类型ir分解索引规则，逻辑refine等不会考虑。同精度下冲突impl将不会被编译器接受。



孤儿原则有效的本质，是其保证了只有一个package能决定forall A: Trait, T: Type. impl A for T，即使新代码冲突了也能在同一package内解决。目前，对任意A、T，其normalized后的"表达式"，必须包含至少一个在本地package中定义的语言项，因为这类值具有定义所有权，也就是说，所有package都能构造i: Integer = 10、s: Symbol = .json这类普通值，而package a中构造一个类型T和package b中构造一个类型T, 两个T是完全不同的。



关联项：
考虑到flurry中基于comptime实现的的依赖类型有它的特点，目前允许任意类型的关联项。对于elaboration压力，我们介绍了大多数上层语言主题后一起讨论。


substitution闭包：
显然当前的系统中，允许量化任意，这给整体的elaboration带来了新的挑战。
impl Drop for Tensor<T, .A = A, .allocator = allocator>
where T, A, quote allocator: A, requires A:- std.mem.Allocator
{
    fn drop(*self) 
    where outcomes kill<self> and read<allocator> and write<allocator> 
    {
        ...
    }
}

impl Clone for Tensor<T, .A = A, .allocator = allocator>
where T, A, quote allocator: A, requires A:- std.mem.Allocator
{
    fn clone(*self) -> Tensor<T, .A = A, .allocator = allocator>
    where read<self> and read<allocator> and write<allocator>
    {
        ...
    }
}


test {
    var cuda_allocator = cuda.CudaAllocator.new();
    
    let xs = Tensor<f32>.zeros(1024, 1024, .allocator = cuda_allocator);
    let ys = Tensor<f32>.ones(.like = xs);
    let zs = Tensor<f32>.zeros(1024, 1024);

    let xs_trait_object = xs.dyn(Clone);
    let zs_trait_object = zs.dyn(Clone);
}


impl Drop for SomeType
where quote space: SomeSpace
{
    fn drop(*self) {
        println("{}", space);
    }
}

test {
    let space: SomeSpace = ...;
    let some: SomeType = ...;

    -- drop(some.ref); ??? 
    -- some: %{ space: SomeSpace = ??? } SomeType
    -- (%{space: SomeSpace = ???} SomeType).as(Drop)

    -- 所以得有什么地方，需要给SomeType.as(Drop)绑定space的substiution
    -- 拒绝scala-like隐式参数，其对编译缓存破坏性大
    -- 也许这样？
    -- some.as(Drop) = space 
}


那些与语法糖交互的trait：

运算符重载traits：
设计 +-*/% ++ == and or not等运算符的重载trait。
Deref for expr.*
Reference for expr.ref
Index（优先找） && SpaceApply for expr[...args]，见/home/fin/vilya/examples/2026-9-7-tensor-algebra.fl
如果你有别的想法可以补充


pattern重载traits -> pattern-matching主题：
见/home/fin/vilya/examples/2026-9-7-pattern-traits.fl，显然没列举完，并且trait的样子是草案


变参与拓参重载trait -> functions主题
见/home/fin/vilya/examples/2026-9-8-variadic-argument-collecting-rules.fl


隐式参数传递的定义trait：
见/home/fin/vilya/examples/2026-9-12-implitcit-substitution.fl


字面量拓展trait for Integer、Float、str、char：
由于extension系统已经死去，原基于此的literal extension转向基于trait实现，请看简单示例即能知晓。
pub struct Pixel {
    value: u32,

    pub fn new(value: u32) -> Self {
        Self { value: value }
    }

    derive Clone, Copy, Debug for Self;
}

impl LiteralInto<.px, Pixel> for Integer {
    fn literal_into(comptime int: Integer) -> Pixel {
        Pixel.new(int.as(u32))
    }
}

注意到，对于表达式`32px`，编译器执行的查询大概是forall T: Type. LiteralInto<.px, ?T> for Integer这样的，几乎固定generalize一个待elaborate变量



Splice trait -> comptime主题，不再赘述
Once, Clone, Copy -> resource management主题
逃逸reachability type等函数契约的unsafe impl、unsafe trait -> 程序验证主题

derive and deriver，见/home/fin/vilya/examples/2026-8-13-derivation.fl

然后，expr.id<argument*>(argument*)显然是一个call(normal_form_apply(project(...), ...), ...)啊，不要发明新语法节点类型来增加系统的冗余性。从用户体验上看，大家都希望能直接serialize<.json>()。

委托与封闭多态已冻结，准备移除。

类型的error/effect handling capability定义我也准备用trait系统进行抽象，不再给语法系统提供负担，并且能提供能清楚的resolution路径。你可以给出草案。


---

# 本轮继续审阅：从新增材料校准 Flurry 的共同机制

日期：2026-09-15；history：046；theme：trait-implementation；第 02 轮。

**DRAFT — NOT CANONICAL。** 你明确说明尚未回答原第 1–11 题。上方原问题和你的补充原文均保留；本节是新的阅读入口。下面区分你已经明确的新决定、我的演绎，以及仍需你修改的预填方案。没有根据旧预填编写正式文档。

追加前的完整 prompt 已归档到 [044](docs_chat_history/044-trait-implementation-01.md) 的“作者补充（非审阅条目答复）”。五份引用草稿只用于理解意图，不能因为日期较新就成为语义权威。

**我的理解首先需要作一个调整。** 这次材料把 Flurry 的设计重心展示得更清楚了：大量不同的表面用法，共用表达式组合、依赖参数、trait 实现选择和可追踪的替换环境。后续审阅应检查这些共同机制能否解释具体程序，避免按每个语法位置另写一套例外。

## 1. 我现在怎样理解这种风格

**表达式的组合本身承担很多工作。** 你对序列化调用的纠正是准确的：投影、范式应用、调用已经能组成所需语法树。上轮我把“这段组合怎样取得参数化接口”误写成了“是否增加专属语法构造”。student.serialize<.json>() 的树形不需要再投票。后续在成员与应用的通用语义中解释其输入输出，不为这个方法增加特殊节点。

**语法糖通过明确的能力协议取得行为。** Index / SpaceApply、模式视图、变参收集、ImplicitPassing、LiteralInto 和 handling capability 可以放进同一张图：表面操作确定所需能力，resolution 取得实现与参数替换，选定方法按自己的阶段和契约执行。各协议仍拥有自己的观察、求值和控制流契约；“走 trait”不使这些行为自动相同。

**结构精度承担优先级，逻辑要求承担适用性。** 这是你已经明确的新规则。类型头更精细可以取得优先级；更强的逻辑要求、重复变量形成的等式限制，不额外提高精度。PTC 仍用于判断要求、证明互斥和解释未决，不能转为“谁的条件逻辑上更强，谁就获胜”的排序器。

**实现实例还需要交付依赖。** Tensor 的 allocator 已进入 target 参数；匹配 target 可以恢复对应替换，调用时仍须取得那个运行时对象。SomeType 的 space 没有出现在 target 或 trait 参数里，单凭匹配没有来源。这两种情况需要分别解释。实现选择至少记住“哪个声明、哪些参数替换、还需要什么环境”，不能只缓存一个方法地址。

**自动传参也可以有明确来源。** 我把你拒绝 Scala 式隐式参数的陈述与 ImplicitPassing 草稿合读为：允许由参数声明和所选协议限定的自动传参，依赖来源必须能追踪。草稿中的调用位置参数依赖调用描述；它没有给 SomeType 的自由 space 提供按名字或类型搜寻局部对象的许可。这是我的理解，具体 passing 协议仍属于 functions 主题。

关联项允许任意类型，意味着它能携带类型计算需要的其他值和函数信息。你已要求将整体 elaboration 压力留到上层主题介绍之后，本轮只检查影响当前语义的接合，不重新开启一次全系统可判定性审问。

## 2. 原审阅条目的状态校准

| 原条目 | 现在的处理 |
|---|---|
| 1：trait 身份与实现主体 | 仍待回复；任意类型关联项不等于普通值已获准作为 impl target |
| 2：implementation 匹配 | 补入 normalized IR 分解索引；仍保持任意纯工厂不可凭名字逆解的既有边界 |
| 3：重叠选择 | 撤回旧预填“首版不特化”；按你明确的结构精度排序，同精度冲突拒绝 |
| 4：孤儿规则 | 旧的主体/参数顺序方案未获批准；改审本地定义身份原则及其实际保证 |
| 5：实现可见范围 | 未回复；索引不自动决定包世界和 use 规则 |
| 6：实现兑现接口契约 | 未回复；先前的契约替换方案仍为候选 |
| 7：关联项、默认成员与 supertrait | 任意类型关联项已明确；其余检查责任仍待审阅 |
| 8：derive | 具名策略、默认策略与生成物分开审阅；引用草稿的孤儿约束尚未收束 |
| 9：序列化调用 | 采用已有表达式组合及你要求的直接写法；撤回专属 AST 增量问题 |
| 10：dyn | 纳入 Tensor 环境与 Clone 的 Self 结果；不预套固定布局或其他语言的禁用规则 |
| 11：委托与封闭多态 | 按“冻结、准备移除”退出推进清单，不再要求你重选是否保留 |

trait phase 的新标记、LiteralInto 方向、独立 extension 机制的退出，都按本次明确陈述更新认知。与现行正文的差异记录为待同步事项，本轮只讨论。

## 3. 索引方案可以形成怎样的共同骨架

下面是**内部算法候选**，不是新的 Flurry 源码，也不新增编译阶段：

1. 在合法实现世界中建立候选索引。全类型 target 单独组织；trait 声明身份、所需成员等已知信息帮助缩小检索范围。
2. normalized 类型项产生结构键。键中的通配槽表示索引抹去的信息，不等同于源语言 Any 的全部语义。
3. 检索返回可能适用的声明，保留真实头、变量关联、完整 trait 实参和要求。同桶不等于类型相等或实现冲突。
4. 每个候选独立产生替换与证明义务；候选之间隔离对推断变量的试探。
5. 按结构精度选择；要求只决定适用、不适用或未决。最终选择必须处理仍可能胜出的候选。
6. 交付声明、确定替换、必要证明和环境取得方式。dyn 等边界继续保留这些信息支持的调用关系。
7. 候选缓存与最终选择缓存分别记录依赖。实现可见范围、推断结果和必要事实变化后，不复用失效答案。

phase 是查询与兑现的维度，但分桶不自行推翻既有共同 coherence 契约。索引减少无关候选；完全未知的查询或大量全类型实现，仍可能访问大量候选。

## 4. 继续审阅：新增材料真正留下的缺口

所有“助手预填”均待你修改，不能视为作者答复。

### R1. query 分解与 impl 索引键，是否采用同一套深度截断？

原第 295 行列出不同深度含 Any 的形状，第 303–305 行又禁止这种形状。可能是笔误，也可能有意区分 query 与 impl key，不能由我暗自选定。

还有一个实际漏检案例。以下全部是**类型项分析记号**，◇ 表示索引通配槽：

~~~text
query:         Tuple<Vec<Tuple<i32, i32>>, i32>
impl head:     Tuple<Vec<T>, i32>
可行替换:      T = Tuple<i32, i32>
impl 索引键:   Tuple<Vec<◇>, i32>
~~~

这个键不在原 query 的五项链里。只逐项精确 get，会漏掉适用实现。

- A. 统一深度截断，已到达的较浅具体叶子保留（建议）。原 query 第 2 级改为 Tuple<Vec<◇>, i32>；impl 中的未知参数限制继续展开的深度。
- B. query 与 impl key 故意采用不同抽象策略。保留原 query 链，增加兼容键检索或额外键以避免漏检；算法和完整性证明更复杂。
- C. 不限于单条深度链，允许多个局部精度键。索引更细，但需另定精度排序和键数量上界。

**助手预填：A。** 对任一合法替换，只要真实 impl 头适用于具体查询，索引就必须取到它。原文表格的修订等本题确认。

已明确排序规则还带来一个结果：Tuple<T, T> 与 Tuple<U, V> 的粗结构精度相同；重复变量约束不会给前者加分。两者同时适用就属于同精度冲突。反过来，同桶中真实头互斥的两个实现不冲突。

### R2. 更高精度候选未决时，什么时候允许提交最终选择？

低精度 Vec 实现已经适用，更高精度的“元素为 Tuple”的实现还要求 P。若 P 为 Unknown，立即提交低精度实现，后来 P 得证就会改变应获胜者。

Vec.new() 在元素类型未知时也可以开始查找，但“找到能推进检查的成员”和“已排除更精确成员”需要不同证据。

- A. 更高精度未决阻止最终提交；保留候选与约束，按既有 ordinary/loose 边界继续检查（建议）。证明不适用后才越过；必要交付点仍不能含未决选择。
- B. 只在当前已证明适用者中选择并冻结，后续事实不改选。同一具体类型可能因定义处事实不同而使用不同实现，需要接受这一后果。
- C. 不保留条件性选择；当前不能确定最高精度及唯一替换就立即拒绝。规则直接，但限制渐进推断与 loose。

**助手预填：A。** 暂定查询可以推进信息，不能作为最终实现证据。Unknown 不按 false 处理，也不借此放宽 ordinary 的必检义务。

字面量的结果元变量也要分别试探候选替换；不能让第一份候选永久写入结果类型，再据此排除其他候选。完整 trait 实例不同与同一实例的重叠，是不同问题。

### R3. 本地定义身份要保证宽准入，还是保证独立包不会在下游等精度相撞？

类型声明身份与普通数值不同，这个依据成立。但“表达式含任一本地身份”还不能推出唯一包负责完整查询。

以下是无需循环依赖的**包与头模式分析记号**：

~~~text
base：定义 Pair 与 trait Q

包 a：只依赖 base；定义 A；提供 Q for Pair<A, T>
包 b：只依赖 base；定义 B；提供 Q for Pair<U, B>

下游同时依赖 a、b，查询 Q for Pair<A, B>
~~~

两份实现都含本地定义，两包各自看不到对方，下游却以同精度重叠。身份区分了 A 与 B，没有唯一分配整个查询的归属。

- A. 本地身份就是宽准入条件，允许下游合流时发现冲突。保留开放扩展，不再承诺冲突总能在同一个上游包内解决。
- B. 保留嵌套本地身份的准入意图，再增加归属/覆盖约束，使上例至少一方不合法（建议）。顺序与覆盖对象在孤儿专轮明确。
- C. 为完整查询及必要精度层指定唯一归属包，泛化实现须证明其覆盖范围已获授权。归属更强，也限制全类型默认实现和嵌套扩展。

**助手预填：B，优先保存你强调的冲突责任目标。** “出现本地身份”先作为必要依据，不能独自成为充分条件。原第 4 题的主体优先顺序仍未获批准，我不会代入。

如果你最重视宽准入并接受下游组合失败，A 是一致的选择。两种目标不能由同一句“至少出现一个本地项”同时保证。

### R4. 新 phase 标记下，归属、执行阶段与关联项共享怎样接合？

默认 runtime phase、属性选择 comptime phase、使用 comptime impl / comptime derive，均已明确，不重问。

**R4a：属性与函数前缀的关系。**

- A. 属性决定 trait phase，函数前缀独立决定执行阶段；所选成员仍须满足执行和跨阶段契约（建议）。不因函数要在 comptime 执行就暗自改选另一相。
- B. 采用有限组合矩阵，例如 comptime phase 只接受 comptime 函数；其他组合静态拒绝，需要列明合法组合。
- C. 属性同时决定执行阶段，函数前缀在 trait 内受该属性解释。需明确它与独立函数标记的不同适用域。

**助手预填：A。** 这不预先承诺所有参数、结果和 phase 组合都能交付。

**R4b：ConcatWith 的来源。** 默认相关联项被两个 comptime-phase 方法引用；模式草稿也有分别命名两相关联项的形式。

- A. 只读签名依赖可明确共享，跨相实现须给出一致关联项值（建议）。共享不使另一相的方法自动可调用。
- B. 关联项严格分相，comptime 方法改用本相的关联项。两相描述独立，例子相应调整。
- C. 增加明确的共同关联项归属，普通项仍分相。表达清楚，但需另定声明方式。

**助手预填：A。** 共享限于已交付且对当前阶段合法的签名依赖，不使 runtime 值内容变成 comptime 常量。这是候选，不能由例子直接视为确认。

现行两相正文按函数前缀解释 phase，需要按新规则同步；本轮不改正文。

### R5. SomeType 中无法从 target 恢复的 space，在哪里绑定？

Tensor 的 allocator 已在目标参数中；SomeType 的 space 是实现体需要而匹配输入没有提供的自由依赖。

- A. 普通 impl 只接受能从 target/trait 输入恢复的依赖；SomeType 改成有明确依赖参数的目标或携带者。边界直接，但限制当前开放实现写法。
- B. 允许开放实现，在显式实现视角建立处绑定缺失替换；能力随值或合法外层环境携带，后续调用复用（建议，贴近你的探索）。另定公开绑定语法，不按词法名字找对象。
- C. 缺失依赖改成方法显式参数。调用直接，但不能独自解释自动 Drop 的参数来源。

**助手预填：B。** resolution 可返回尚需封闭的实现描述；替换来源明确且环境可交付后，才取得可调用能力。绑定不自动延长 space 的生命。

some.as(Drop) = space 仍是你标“也许”的草案。我不会将它写成已有赋值语义。采用 B 后，再比较明确的绑定入口。

Tensor 的 dyn(Clone) 还留一个动态接口检查点：allocator 环境如何跨封装边界，以及 clone 的 Self 结果如何交付。暂不按其他语言的规则禁用，也不替你选择重新封装或保留隐藏类型关系。

我想需要设计一套易用、心智负担低、合理的substitution显式绑定、推导、传递的机制（对于runtime dependency，尤其是impl中的），不仅要用户友好，也要elaboration友好

### R6. 显式 deriver、默认 deriver 与生成物分别受哪层归属约束？

引用草稿的两个范围尚未收束：

- 2026-8-13-derivation.fl 第 12–13 行为 default deriver 写了孤儿约束。
- 2026-9-12-implitcit-substitution.fl 第 21–23 行提出 deriver 方案不需要孤儿约束，并用具名策略承接扩展能力。

如果任意包都能把外部 trait/外部类型的新实现注入共同世界，策略换个名字并不解决冲突。

- A. 策略定义单独准入，所有生成的普通 impl 都遵守共同归属/coherence。显式选择不赋予产物额外权限。
- B. 默认策略注册受归属约束；显式策略可产生有明确选择依据的局部能力，不自动进入无资格的全局候选世界（建议的演绎）。需明确其作用域与证据传递。
- C. 显式派生也可注册外部实现到共同世界，冲突在依赖合流时拒绝。开放程度大，应与 R3 的政策一致。

**助手预填：B，待确认。** 这分别表达可由用户选择的策略和自动可见的实现，但现有材料没有批准这种局部能力作用域。若所有产物都是同一种全局 impl，应选 A 或 C，不能把 B 当作我已替你定下的机制。
是的，有关文本是早期探索缓解孤儿原则方法是的设想，现在impl的孤儿原则已经可以由receiver adaption优化。
另外，改一下指定deriver的语法，改为post use语法（就是use handler那个），避免在projection语法上开洞：
    derive Serialize<.json>.use(by_repr) for Student

### R7. handling capability 的 trait 草案覆盖哪一层？

我的起点：handler 类型提供具体错误或完整 effect operation 的入口，普通 resolution 取得入口与替换环境；既有行与控制流契约检查它怎样处理当前计算。

- A. trait 描述分支入口；行消去、效应层次与恢复包装按既有类型/控制流规则检查（建议）。用户 impl 不可自证任意消除错误或效应。
- B. trait 还提供整体处理结果的关联描述，但实现必须证明符合既有行与控制流契约。协议更可编程，检查接口更大。
- C. 先只抽象错误分支与直接返回的 effect 分支；可恢复处理留到资源/控制主题后接入。首轮范围更小。

**助手预填：A。** 关联值可以表达类型信息，不自动成为可信的 row split 或恢复有效性证明。公开名称和完整参数列表仍为草案。

### R8. LiteralInto 的结果推断与 char 后缀边界

**R8a：同一后缀能否对应多个结果类型？**

- A. 可以，按完整 trait 实例区分；目标上下文参与推断，无唯一解时报告该次查询歧义（建议）。
- B. 同一输入字面量类型与后缀必须唯一决定结果类型。注册时检查唯一性，调用更直接，但增加限制。
- C. 多结果只通过显式类型/实现视角使用，裸后缀必须唯一；不依靠周围目标选择。

**助手预填：A。** 内部引入结果元变量，通过候选和上下文解它。原文的 forall T 是非正式说明；这里并非要求每个 T 都存在实现。不同完整实例可共存，与同一实例同精度重叠分别诊断。

**R8b：列入 char 是否也扩展后缀表面范围？**

现行 docs/basics/literals.md 只列 integer、float、str；新增材料把 char 也列为字面量扩展主体。

- A. char 也采用相同后缀形式，在字面量章节同步加入（建议，与此次并列最一致）。
- B. char 可实现普通转换能力，但后缀形式仍仅用于原三类。
- C. char 后缀范围留字面量专轮，本轮只确定其能力方向。

**助手预填：A，待确认表面增量。** 现行“按类型成员调用”的说明也需接到 trait 机制；后缀脱糖不规定 Pixel 构造函数内部如何检查范围。

## 5. Handling capability：可以审阅的草案

**DRAFT — NOT CANONICAL。** HandleError 是本节提议的名字，不是声称已有的标准库 API。下面使用现行声明、调用和错误处理语法，不增加 handler 表面写法。

~~~flurry
trait HandleError
where E, R {
    fn handle_error(*self, value: E) -> R;
}

enum LimitError {
    too_large(u32),
}

struct ClampTo {
    limit: u32,
}

impl HandleError<LimitError, u32> for ClampTo {
    fn handle_error(*self, value: LimitError) -> u32 {
        value match {
            .too_large(_) => self.limit.*,
        }
    }
}

fn checked_count(count: u32) -> ![LimitError] u32 {
    if count <= 8 {
        count
    } else {
        error LimitError.too_large(count)
    }
}

test {
    let handler = ClampTo { limit: 8 }
    let result = checked_count(12)! {
        .too_large(rejected) => {
            handler.as(HandleError<LimitError, u32>)
                .handle_error(LimitError.too_large(rejected))
        },
    }
    println("count: {}", result);
}
~~~

已给出我的版本。

意图输出 count: 8。错误分支通过显式接口调用 handler 类型的能力；入口选择、接收者和字段访问都是普通机制。这段完整写法展示候选协议如何工作，尚未宣称编译器能从 handler 类型自动生成整个处理块。

采用 R7-A 后，自动接合负责确定“哪条分支要求哪个能力”，再用同一 resolution 取实现。其他错误保留向外路径；分支再产生错误时，输出行仍须容纳它。找到 HandleError 实现不意味着其他错误一并消失。

effect 能力族先用**内部契约记号**描述，不发明 operation 元对象的公开类型名：

~~~text
完整 operation 实例 op 的签名：X → Y
handler 类型：H
分支的外部允许行：ε

直接返回入口：
    (H 的合法接收者, X) → Comp(ε, Y)

可恢复入口：
    k : Y → Comp(ε, B)
    (H 的合法接收者, X, k) → Comp(ε, B)
~~~

查询身份包括完整 op 实例；其参数身份和 scoped occurrence 路由都要保留。Y 是给调用点的值，A 是主体正常结果，B 是整体处理结果；直接分支返回 Y，不能因 handler 想产生 B 就改 operation 签名。

可恢复能力仍受单次、不逃逸及合法清理约束；分支在外部环境执行，恢复或转发后重新安装应有 handler。入口可编程，不使普通实现获得任意声明这些不变量成立的权力。

建议将 error/effect 完整案例作为本主题与控制主题的共同验收：能沿 resolution 找到能力，也能说明结果、剩余行与环境如何交付。

## 6. 文档地图与源码复核备忘

后续内容围绕“声明身份与参数—结构索引与选择—替换及环境—派生与资格—动态交付—语法协议入口”组织。详细用户语义仍归所属章节：

| 材料 | 本主题承担 | 详细归属 |
|---|---|---|
| 运算符能力 | 一元/二元算术、比较、连接与布尔组合的实现入口 | 运算符/控制流；参数求值方式由对应操作契约规定 |
| Index / SpaceApply | 选择身份及优先关系的接合点 | 下标/应用；缺候选、要求未决、运行时失败不能擅自共用回退规则 |
| PatternView 系列 | 两相能力与关联项来源 | pattern-matching |
| 两类 VariadicCollector | 实现查找与替换 | functions；临时承载物及返回视图的有效性按资源契约检查 |
| ImplicitPassing | 明确的协议身份与来源 | functions |
| LiteralInto | 后缀实例与结果推断变量 | 字面量 |
| Splice | 两相体系中的正常能力入口 | comptime |
| Once / Clone / Copy / Drop | 实现与依赖环境 | resource management |
| unsafe trait / impl、reachability | 保留接口位置 | 程序验证 |
| handling capability | 分支能力 resolution | error/effect 与控制流 |

引用的 pattern 草稿有放弃特定穷尽性检查的设想，本轮只记录为该主题草案，不据此改写匹配规则。derive 反射 API、变参临时对象布局和资源 outcome 不在此擅自定稿。

**源码复核记录：**

- 本轮 HandleError 示例已按 grammar/style 自检；没有假造库函数或使用外来指针、布尔语法。未通过编译器执行验证。
- 作者 ConcatPatternView impl 只展示 prefix，作为片段保留；完整例子还需明确关联项与其余必需成员，不因片段缺项判接口非法。
- comptime impl / comptime derive 是作者明确的新规则，global grammar 待同步；phase 属性复用现有属性形式，名字可见性按普通表达式处理。
- concat pattern、资源条款、显式替换绑定与草稿 derive 函数形式尚未全部被现行 grammar 覆盖。保留为草案，不悄悄替换成我猜测的形式。
- variadic 草稿中的 ::、省略体和示意访问不作为规范代码；展开时采用现行投影，并先确认未知收集/反射 API。
- phase、字面量以及冻结的委托/封闭多态各有正文同步位置。本轮未改正式 docs，也未填充新建的 substitution-inference.md 与 type_id.md 空文件。

**当前审阅入口：先看 R1–R3，再看 R4–R8。** 原条目其余未答部分继续保留，所有助手预填仍待确认。



现在auto dereference/reference太保守了，优化elaboration机制

---

# 本轮审阅：依赖替换的交付与接收者适配

日期：2026-09-26；history：047；theme：trait-implementation；第 03 轮。

**DRAFT — NOT CANONICAL。** 恢复文档协作和逐轮记录。本轮先收束你在 R5、R6 和末尾提出的新要求；下面由我预填方案供修改。未改动的旧预填不视作批准，不重复搬回整套旧问卷。

## 1. 对本次回复的理解

你要求 substitution 的显式绑定、推导和传递形成统一、易用且适合 elaboration 的机制，重点是 implementation 的 runtime dependency。你也要求扩大 auto dereference/reference 的能力，而不是维持当前正文的保守边界。

指定 deriver 的写法已经明确为：

~~~flurry
derive Serialize<.json>.use(by_repr) for Student;
~~~

这里按你的决定复用 `handler_application -> expr.use(expr)`，其结果仍作为 derive 的接口表达式；不为 deriver 选择另造 projection 或专属应用节点。上面只是调用片段，Serialize、by_repr、Student 的声明沿用你的例子。

你指出“绕开孤儿规则”的 deriver 文本属于早期探索，现在可以借 receiver adaptation 改善使用体验。我的理解是：优先让用户通过明确视角使用本地适配类型的实现，而不是默认授予派生器全局注入外部实现的特权。你在 R6 的“是的”紧接着解释了历史来源，我不把它扩大解释为批准我先前提出的整套“局部能力世界”。下面给出更小的方案供审阅。

handling capability 以你给出的 `/home/fin/vilya/examples/2026-9-18-handling-capability-by-trait-system.fl` 为讨论起点，不继续推进旧 ClampTo 草案。该文件中的行和 handler 规则仍按具体声明区分，不把整个草稿一并视为正式规范。

## 2. 我建议的共同表示：一份替换，两种交付

实现匹配产物可以用以下**内部分析记号**表示，不是新增源码类型或编译器 API：

~~~text
实现证据 = (声明身份, 完整接口实例, 参数替换 σ, 要求的证据, 环境取得计划)

σ 的条目可以是：
    comptime 参数 -> 已确定的值或仍待求解的符号项
    runtime 依赖参数 -> 确定的绑定 identity

环境取得计划：
    依赖 identity -> 当前操作如何取得对应运行时对象
~~~

**知道依赖是谁，与当前能拿到它，是两个检查。** Tensor 的类型匹配可以确定 allocator 的 identity，但跨函数返回或 dyn 封装后仍要交付对象。反过来，作用域里恰好有一个同类型 allocator，并不能证明它就是目标依赖。

这不要求每个值增加环境字段。已知的运行时依赖类型规则允许共享外层环境、隐藏参数或必要的封闭携带者。这里补的是：由哪份实现证据要求这些依赖，以及如何使所有调用使用同一份替换。

普通函数、方法、闭包调用和自动清理可以共享这个表示。它们的触发规则不同：方法在调用点需要环境，Drop 在每条实际清理边上需要环境；不能到清理时才临时重新选择一个依赖对象。

### S1. 推导依赖时，允许哪些来源？

以 Tensor 为例，建议从目标和完整 trait 实参解出 T、A、allocator；必要时从已经交付的实现证据取得环境。SomeType 的自由 space 若没有这些来源，就需要显式绑定。

- A. 从显式绑定、匹配输入和已携带证据统一收集约束；无来源则留下具名缺口，必须在能力交付前补齐（建议）。各来源必须一致，不按“哪个优先”覆盖不同 identity。
- B. 除上述来源外，允许从当前作用域寻找唯一同类型对象。短代码更少，但增加一个局部声明可能改变选择和缓存依赖。
- C. implementation 的所有 runtime 依赖都必须每次显式传入。检查简单，但重复 notation 多，自动清理还需另设入口。

**助手预填：A。** 所谓推导，是恢复已有关系；所谓绑定，是提供此前缺失的关系。显式传参也不能把目标中已固定的 allocator 换成另一个对象。无来源的依赖不通过名字相同或当前内容相等来补齐。

### S2. 显式补齐开放 implementation，是否复用范式应用？

我建议先统一“选接口，再补参数”的表达式组合。候选表面片段如下：

~~~flurry
-- DRAFT：新增的是开放实现视角的应用语义，不是新 AST 节点。
some.as(MyTrait)<.space = space>.operation();
~~~

MyTrait、operation 是占位接口与方法，不是标准库 API。这里的 space 归属于已选实现的开放参数，不是给 MyTrait 擅自加参数，也不是给 operation 加普通实参。现行语法能组成这棵表达式树，但**现行语义尚未规定实现视角可这样应用**。

对于 `quote space` 一类依赖槽，绑定保留 runtime 对象的 identity 与合法取得关系，不在 comptime 读取其内容，也不因写在尖括号里就复制该对象。这个应用必须复用依赖参数的阶段契约，不能降成接收任意 runtime 实参的普通 comptime 求值。

若同一查询仍有多份实现候选，命名参数只能在隔离的候选环境中匹配；不能因为先看到某个 impl 的参数名，就提交它。该设计还意味着这类开放参数名成为可被用户引用的接口，重命名不能再被当作完全私有改动。

- A. 采用上述组合；参数名对应所选实现公开的依赖槽，应用只封闭既有开放参数（建议）。普通非开放值仍不能随意接受尖括号参数。
- B. 首版要求把依赖放进 target 类型参数，由普通类型应用和构造绑定。无需开放实现视角的新语义，但 SomeType 的原写法需要改成依赖类型或本地携带类型。
- C. 使用具名普通工厂来构造携带能力的值；工厂参数是公开 API，impl 参数保持私有。用户入口更稳定，但需给每类开放实现提供工厂。

**助手预填：A 用于直接表达开放实现，库可以用 C 包装常用入口。** A 与 C 这里比较的是基础语言是否提供 A；只采用 C 时不隐含存在 A。先确认这层语义，再写完整的定义和调用例，避免用一段看似可运行的代码掩盖尚未确定的能力承载方式。

### S3. 被绑定的能力如何保存，尤其是 Drop？

我建议区分短期调用视角与需要跨边界保存的能力：

~~~text
短期调用：
    选择 impl -> 封闭依赖 -> 使用一次视角调用

跨边界携带：
    值本体 + 必要实现证据 + 环境取得关系 -> 一起交付

具有自动清理责任的值：
    建立清理责任时固定所需证据 -> 移动时传递 -> 每条退出边兑现
~~~

这不是规定三个源码类型。具体何时有物理环境，仍由布局和逃逸需求决定。

风险最明显的是：为同一个待清理对象，先建立 Drop 使用 space1 的视角，再建立使用 space2 的视角。如果视角建立能随意改写清理责任，那么看似普通的成员适配就改变了资源行为。

- A. 普通调用视角可短期存在；需要持久使用时显式携带。Drop 的依赖随清理责任建立而固定，普通 `.as(...)` 不改写它（建议）。首次绑定清理责任必须接入构造/交付，不能留到销毁时搜索。
- B. 首次使用某种 trait 就在原对象上永久登记绑定。调用简短，但分支、别名、首次使用顺序都会影响后续行为，需要新的对象状态协议。
- C. 开放实现仅支持显式调用，不参与自动 Drop；自动清理依赖只能来自 target。限制明确，但不能统一所有开放实现。

**助手预填：A。** 如果 SomeType 的唯一合法 Drop 依赖 space，那么创建这种资源时就要完成相应绑定；不能先把资源交付到会发生清理的程序路径，再期待后续方法调用补齐。具体构造与绑定之间的失败路径、责任转移归资源主题细化；本轮不会把 S2 的一次方法调用写成已经解决自动 Drop 的完整例子。

函数只使用已有依赖能力时，可以沿明确的参数契约传递隐藏环境；函数若需要调用者新提供 space，就必须让这一要求出现在可检查的接口中。局部 body 中发现缺口，不等于可以自动向所有调用者增添一个未声明的依赖。

分支汇合也必须保留来源：space1 和 space2 不同，不能因为两边本体都叫 SomeType 就抹平能力环境。若选择封闭表示，需要实际保留分支对应的证据；若接口不容纳这种表示，应在边界要求显式处理。这里不默认为所有分支合成新 union 或 dyn 类型。

## 3. 接收者适配：扩大可达路径，同时明确顺序

当前正文 [receiver-adaptation.md](docs/type/receiver-adaptation.md) 规定 Deref/Reference 只用于显式操作；[member-projection.md](docs/type/member-projection.md) 要求定义端固定成员来源。你的新要求意味着前者要扩展，但不必放弃后者。

我建议：**先形成有限的有序候选路径，记录路径要求，最后只执行选中的路径。** 自动解引用方法可能带副作用、需要环境或产生临时值，查找本身不能运行它，更不能把失败候选的适配代码也执行一遍。

### A1. 自动解引用与自动取址开放到什么程度？

- A. 方法接收者允许多步内建解引用及唯一确定的 Deref 路径；选定方法要求指针接收者时，再做一次必要的取址或 Reference 适配（建议）。Reference 由目标接收者需求触发，不反过来无限产生新查找主体。
- B. 允许任意组合 Deref、Reference 与 cast，寻找可用方法。覆盖更多程序，但需要处理大量路径、环、优先级和不同副作用路径的歧义。
- C. 只开放多层内建指针处理，重载 Deref/Reference 仍显式。实现最小，但不能充分满足智能指针等包装的使用体验。

**助手预填：A。** 内建取址用于合法可取址对象；自定义 Reference 若存在多份同优先级可行转换，不猜一个。所有路径上的 trait 查询都使用同一 implementation 选择机制，保留真实效果、依赖和资源要求。

例如在缺少原主体同名方法的前提下：

~~~text
原接收者类型 Box<Widget>
Deref 的确定结果为 *Widget
Widget 方法要求 *self

候选动作：调用已选 Deref -> 得到 *Widget -> 调用已选方法
不是：执行 Deref -> 复制整个 Widget -> 再取新 Widget 的地址
~~~

这是内部路径例，Box 不被声明为 Flurry 已有标准库类型。Deref 的精确协议以该主题最终定义为准。

### A2. 视角顺序、适配深度和 impl 精度怎样组合？

这三个次序不能简单相加成一个分数。你确认的结构精度解决的是同一主体查询的实现选择；它没有说“另一种接收者上存在更具体 impl，就能跨路径抢走方法”。

建议按以下次序查找：

1. 显式接口视角先限定成员来源；类型视角则保留现有从后往前的顺序，例如 D、C、B、A。
2. 对当前类型视角，从原接收者形态开始，再逐层考虑解引用后的形态。
3. 每个形态内部遵守已有能力/成员来源规则，trait 查询按 normalized IR 结构精度选择实现。
4. 所选方法的接收者需求触发末端取址。不同 trait 的同名成员不会因为一个少做一次取址就自动消歧。
5. 优先位置未决时保留义务；只有已证明不可用才越过它。选定来源后，参数或资源契约失败应诊断该调用，不能用另一来源把错误藏掉。

- A. 采用上述嵌套顺序（建议）。显式视角表达的偏好高于其内部解引用长度；同一位置的实现精度独立计算。
- B. 在所有视角之间优先选择最少适配步骤。可能使显式排在最后的视角输给较早视角，须修改现行视角优先含义。
- C. 只要多个路径都能调用就报歧义，要求用户显式展开。规则直接，但削弱自动适配的便利性。

**助手预填：A。** 定义端固定来源及动作路径中的符号契约；实例化负责兑现，不能见到具体类型新增的方法就重新开始名称竞争。对于尚未决定的推断变量，记录候选查询，不伪装成已经选定的调用。

此方案仍需与旧 R2 一起确认：我建议更高优先候选 Unknown 阻止最终提交，而不是把 Unknown 当作不存在。这里给出统一规则，不另开一套接收者专用的未知处理。

### A3. 是否同时让目标类型引导内建指针读取？

用户提供的 handler 例子在 `*self` 中返回 `self.limit`；当前规则下字段投影得到 `*u32`。如果想减少此类 `.*`，仅扩大方法查找不够，还需有目标类型引导的值适配。

以下是候选规则的完整教学例，前后共享同一字段投影语义：

~~~flurry
struct Counter {
    value: u32,

    fn read(*self) -> u32 {
        -- DRAFT：返回目标 u32 触发合法的 Copy 读取。
        self.value
    }
}

test {
    let counter = Counter { value: 7 }
    let pointer: *u32 = counter.ref.value;
    let value: u32 = counter.ref.value;
    println("{} {}", pointer.*, value);
}
~~~

候选语义下输出 `7 7`。原始字段投影仍是指针；第二个绑定根据显式目标插入读取。没有目标的 `let field = counter.ref.value;` 仍保留字段指针。

- A. 对已有明确目标的检查位置，允许有限次内建指针读取；每步须满足有效性，读出值须具有 Copy 能力；不自动搬出非 Copy 对象（建议）。先从初始化、实参和返回值位置接入。
- B. 仅扩大方法接收者适配，值读取仍要求显式 `.*`。风险更小，但 handler 等例子继续补解引用。
- C. 连任意用户 Deref、Reference、cast 都参与一般表达式目标推断。覆盖更广，但涉及转换搜索和更多隐式效果，应独立定义。

**助手预填：A。** 目标引导的自动读取不变成一般隐式转换，也不把 Copy 当成可从无效内存读取的许可。对非 Copy 值，显式读取或移动仍按资源规则检查。

需要强调：这仍不能使 ClampTo 中返回 u32 的方法兑现任意 `T` 结果。自动解引用最多得到 u32，不能得到任意类型。该草稿的 `impl HandleError<LimitErr, T> ... where T` 若要继续返回 limit，应收窄为 u32；保留任意 T 则须使用能够交付 T 的恢复逻辑或不正常返回的逻辑。此处不擅自改作者草稿，也不把它当作“适配会解决”的例子。

## 4. Receiver adaptation 怎样改善孤儿规则的使用体验

我的建议是沿用你已经给出的本地 newtype 与显式类型视角路线：实现仍属于本地适配类型，调用者可以通过视角使用它，避免每次手写包装和拆包。

~~~text
外部类型 S
本地 newtype LocalS，具有已允许的双向 cast 关系
本地实现 Trait for LocalS
接收者带 S.as(LocalS) 视角

成员调用 -> LocalS 上的实现 + 必需适配动作
不因此产生 Trait for S 的全局实现
~~~

使用便利性的改进，并不单独证明任意嵌套本地身份足以解决跨包 coherence。旧 R3 的 `Pair<A, T>` / `Pair<U, B>` 反例仍存在；本轮先不重复询问整套归属规则。

### D1. 显式 deriver 的产物是否回到普通 impl 规则？

- A. 产物遵守共同的 impl 归属与冲突规则；外部类型需要适配时使用本地类型和 receiver adaptation（建议）。`.use(by_repr)` 选择生成策略，不为产物赋予豁免。
- B. 保留独立的局部能力产物，可不注册到普通实现世界。需要另定作用域、传递和冲突规则；接收者适配不能代替这些定义。
- C. 显式派生可以向普通世界注册外部实现，由下游冲突检查兜底。应明确接受组合两个独立包后失败的情形。

**助手预填：A。** 这是根据你对早期探索的说明作出的收敛建议，不将原 R6 的助手 B 悄悄升级为已确认。默认策略注册的准入与生成产物的准入继续分开；本题只确定产物，不据此推导完整 default deriver 优先级。

## 5. Elaboration 的可实现骨架

上述方案若采用，可以按以下步骤实现，而不要求在一次搜索中解完所有问题：

1. **建立查询。** 记录成员名或完整 trait 查询、所需 phase、接收者类型及视角、现有参数约束、实现世界。运行阶段与 trait phase 仍分开保存，旧 R4 尚未全部确认。
2. **按序展开路径。** 为当前接收者形态检索候选；没有可用来源时才向下一个允许的解引用形态推进。记录动作描述，不执行 runtime 代码。
3. **隔离试探。** 每个候选用独立的推断状态，产生 σ、要求、开放依赖和动作。未获胜候选的推断赋值不得泄漏到公共状态。
4. **处理未决。** 路径中 Deref 的关联结果未知，就等待相应义务；不能编造下一个主体。相同查询状态重复出现报告循环；持续生成不同类型的展开也须受资源预算限制，耗尽报告未决或超限，不能伪称无成员。
5. **确定来源。** 路径顺序与实现结构精度分别计算。候选的显式参数绑定只补充其约束；更高优先候选未排除前不提交较低者。
6. **封闭依赖。** 对所选证据的每个 runtime identity 确定取得方式。已知 identity 却当前无法取得，与根本没有绑定来源，给出不同诊断。
7. **检查适配与调用。** 接收者求值一次；执行选定路径的转换、解引用、末端取址和调用。沿途效果、临时值清理、依赖有效性以及仿射操作进入实际控制流检查。重复调用所需的环境是否还能使用，也按契约检查。
8. **跨边界交付。** 返回、捕获、移动和动态封装时保留必要依赖；自动清理使用已经固定的责任与证据。Any/any 不能充当遗漏环境的补丁。

缓存至少区分三个结果：候选集合、符号化选择/替换、使用点的环境交付检查。结构索引可以广泛复用；带 runtime identity 的语义结果不能仅按依赖类型名合并；代码模板可在证明成立时共享，但共享机器代码不表示两份依赖身份相等。内存状态变化后的合法性检查也不能靠旧的“实现存在”缓存跳过。

验收例按程序行为组织：

| 情形 | 预期 |
|---|---|
| Tensor 目标已固定 allocator1，同时显式传 allocator2 | 绑定不一致，不覆盖原依赖 |
| 两个 allocator 内容相同 | 不以内容相等合并 identity |
| SomeType 缺 space，局部恰有同名变量 | 仍报告缺少明确绑定 |
| 所需依赖只能留在已退出的环境 | 在交付边界拒绝 |
| 一个值移动到新作用域后清理 | 使用随责任传递的同一 Drop 证据 |
| Deref 存在副作用，多个候选被检索 | 只执行最终路径上的动作，次数不因搜索增加 |
| Deref 自环或不断产生新包装类型 | 诊断循环或展开超限，不静默选择另一路径 |
| Pointer 字段有 Copy 值目标 | 按 A3 决定是否插入读取；无目标仍保留指针 |
| 高优先来源已经选定，但资源契约失败 | 报告原调用错误，不换来源继续 |
| 两个 trait 提供同名成员 | 不用解引用成本替代来源消歧 |

## 6. 内容规划与本轮状态

建议正式章节按以下依赖顺序推进：

1. **Trait 实例与实现证据**：声明、参数、关联项、所需 phase；让读者看清一次调用究竟取得什么。
2. **实现检索与选择**：结构索引、精度、适用性和未决；旧 R1–R3 的剩余决定在这里合拢。
3. **Substitution 与运行时环境**：本轮 S1–S3；完整使用 Tensor、有缺口的实现、跨函数交付三个例子。
4. **成员与接收者适配**：本轮 A1–A3；更新类型主题中的相关页面，并接合定义端来源固定。
5. **Derive 与策略**：已明确的 `.use(...)` 组合，加 D1 的产物规则，再定义默认策略。
6. **动态接口与协议入口**：dyn 的环境与 Self 结果另用完整案例收束；handling、pattern、变参等只在 trait 篇解释共同机制，各自语义回到相应主题。

不需要先把所有上层协议写完才能确定依赖交付骨架，但也不借这一骨架预先发明它们的完整 ABI。

**已确认：** 恢复文档流程；新的 deriver 选择写法；需要更易用的 substitution 与更积极的接收者适配；handling capability 采用作者版本继续讨论。

**候选：** S1–S3、A1–A3、D1 的全部预填。旧 R1–R4、R8 以及早先未答基础问题中未被作者明确陈述覆盖的部分仍待定。

**待同步正文：** trait phase 新标记与执行阶段的区别；接收者适配规则；deriver 选择形式。前两项尚有具体语义待审，不按旧正文反驳新设计，也不提前把候选写为规范。

**语法与风格自检：** `.use(...)`、`.as(...)`、投影、范式应用沿用现有组合；S2 明确标注新增应用语义；A3 明确标注新增适配语义。例子使用 `.ref`、`.*`、就地定义的 Counter 与已知 println，分隔符按 grammar 核对。内部算法与未定协议使用 text 记号，没有伪造可编译的 Flurry 实现。未运行当前编译器作为语言语义验证。

**本轮落盘：** 原 prompt 全文追加到 046；本审阅追加到 prompt，并完整保存到 047。正式 docs 暂不写入这些待确认规则。

``````
