# 依赖观察、精确捕获与稳定查询

状态：目标设计；配合 [状态协议](elaboration-state-and-publication.md) 阅读。
本文件说明“缺少什么”如何成为可检查的依赖，而不是继续增加全包扫描或布尔 ready 标记。
依据为 [局部依赖](../docs/compilation/local-dependencies.md)、
[稳定查询](../docs/compilation/incremental-queries.md)和
[冻结](../docs/compilation/staging-and-freezing.md)。

## 1. 依赖不是统一的“引用了某个声明”

同一声明的名字、签名中一个参数域、主体、布局、实现候选集合和运行值具有不同生产者。
如果都用 `DefinitionReady(id)`，会制造假环、全量失效和无关捕获。
反之，只保存“使用了 Type T”而不保存观察方式，会在版本更新时错误复用结果。

| 依赖类别 | 代表观察 | 不能提前要求的内容 |
| --- | --- | --- |
| 绑定/作用域 | 某名称对应的声明、代码端口对应的 binder | 完整函数体或运行值 |
| 形成/域 | 参数的接收类型、TypeRef 槽域、调用阶段 | 该参数的具体值，除非域确实依赖它 |
| 结构 | 已准入类型头、optional 载荷、某字段存在 | 完整布局或 NF 主体返回值 |
| 类型命题 | 原始关系及证明前提 | runtime 内容、内存存活事实 |
| 可见候选 | 给定 SemanticView 的候选集合/负查找 | 其他包或未来 frontier 中的 impl |
| 值计算 | 某个实际 comptime 参数或调用结果 | 未选控制分支的执行 |
| 程序规格 | read/write、初始化、cleanup、稳定观测 | 将其放入 PTC 统一处理 |
| 表示/环境 | 目标布局、依赖对象的取得计划 | 无关 pointee 布局、未来对象内容 |
| 构建交付 | 实际读取配置、生成源码、artifact | 未读取配置或无关成员包 |

这些类别不新增语言 sort。它们用于调度、依赖记录和证据维护。
一个表达式可以同时产生多类依赖，不能只选择最方便缓存的一种。

## 2. ObservationKey 与结果

拟定的键包括主体、请求 facet、binder scope、SemanticView、必要实例输入。
例如 `ObserveTypeHead(t)` 和 `CompleteType(t)` 不同，
`ObserveSignatureParameter(f, i)` 与 `CheckBody(f)` 不同。
拥有相同 core 的 TypeRef 必须把引用端 substitution 纳入相关观察。

Known 交付的不只是 value，还包含成立范围、依赖代次和消费权限：

```text
Observed<T> {
  value: T,
  formation_evidence,
  premise_scope,
  dependency_stamps,
  provenance
}
```

这不是把所有证书都塞进每个节点。数据可以共享，具体观察只引用实际必要的证据。
只有各自层次的构造入口能签发形成证据；Debug/反射看到了某字段不授予使用该字段的权限。

Pending 必须关联到具体生产者和已观察代次。它不能只是某个 string 报错里包含 waiting。
Unresolved 应区分预算、未支持片段、缺少前提、歧义和不允许的阶段；只有新增真实信息
或明确改变求解条件才有重试依据。

## 3. 三种依赖结构

| 结构 | 节点寿命 | 边的意义 | 如何结束 |
| --- | --- | --- | --- |
| 词法/接口依赖 | 定义或其稳定版本 | 检查/实例化必须供应哪些绑定及契约 | 形成稳定摘要，随定义版本失效 |
| 局部生产依赖 | 一次 region/task 代次 | 哪个结果尚由哪个活跃生产者生成 | 完成、失败、回退或取消时注销 |
| 稳定 query 依赖 | 可复用结果的寿命 | 本次结果实际读取哪些稳定输入/证据 | 依赖变化后重算或重证 |

三者可互相产生请求，但不可相互冒充。词法依赖里出现一个递归声明不意味着具体值环；
活动 waiter 不是冻结定义的一部分；query key 相同不意味着可以恢复上次 VM 的槽。

## 4. 精确词法输入分析

当前 `program.rs::ensure_signature` 把父 type_parameters/captures 和 staged inputs
保守加入局部声明，`lexical_function_arguments` 随之供应这些输入。
`program/lexical_inputs.rs` 能提升部分输入域，也明确诊断缺少 invocation environment。
新分析必须替换这些生产行为，而不是加一张只供 dump 的“精确依赖表”。

### 4.1 收集什么

使用 resolver/HIR 的真实身份，不按名称字符串收集。每条边记录引用位置和用途：

```text
DependencyUse {
  from_definition, target_binding,
  role: domain | value | proof | runtime_identity | code_context | control,
  phase, guard_or_residual_origin, source
}
```

扫描参数域、返回域、默认表达式、子句及程序契约、属性、主体、模式导出和嵌套定义。
“仅在 ensures 或属性中使用”也是真实依赖；但不同用途不必都变成隐藏值参数。
例如选定实现所需的类型证明可以通过证明环境交付，不能重复调用一个值表达式来重建它。

源函数的显式参数/子句保持完整，即便未使用也不能从 callable 签名删除。
精确化只决定局部声明的隐式外层依赖及所需环境，不改变用户声明的 arity、默认值责任和
源执行顺序。这一点与性能上的依赖切片必须分开。

### 4.2 闭包传播

先计算直接依赖，再沿局部调用和类型/契约引用传播。callee 输入通过该调用的实际
substitution 映射到 caller；不能将 callee 的本地 binder 原样混入外层。
把新增依赖的域继续展开到前置绑定，直到稳定；互递归组使用单调固定点，不展开函数体
到无限深，也不凭“当前集合没内容”宣称递归纯净。

未知 callback 的环境依赖取自其已检查接口及调用契约；没有接口时保持对应观察未决，
不能猜整个外层都捕获，也不能猜没有捕获。运行时 capture 仍受阶段、拥有/借用和逃逸检查。

派生 proof 所用前提也是边。若检查器曾使用一条外围假设证明操作合法，摘要不能只保留
操作的表面变量而丢掉该假设的域、作用范围和依赖。

### 4.3 检查依赖和执行依赖

未选分支可能仍有 ordinary 必检义务；静态分支选择只能裁剪获准不执行的工作，不能
因此取消定义端检查。摘要分别记录 `check dependencies` 与 `execution dependencies`，
合法 residual 记录其激活控制条件。默认表达式的签名检查不等于本次调用需要执行默认。

例：参数明确覆盖默认值时，不执行默认供应函数，但已有定义的默认域和契约仍须合法。
comptime 条件具备后，只执行选中分支；未选分支的来源和必要检查不靠删除 AST 消失。

### 4.4 从依赖摘要到有序 telescope

对确需携带的外层输入，按域依赖形成有序 telescope，保留每个原 binder 的映射。
先检查引用合法性，再拓扑安排内部槽；不能用重排槽位修复源语言里不合法的前向引用。
多个来源共享同一外部身份可共用一个槽；不同对象即使域相同也不可合并。
域依赖真正成环时输出最小有用的路径及来源，不任意赋一个 Type 打断。

输出至少分为：静态/类型输入、runtime 依赖及环境要求、证明前提、代码上下文及控制端口。
它们进入同一重绑定计划，但不能被统统转成普通 runtime 形参。

## 5. 四个判断精确性是否正确的例子

| 场景 | 应得依赖 | 错误实现 |
| --- | --- | --- |
| 子句默认中局部 `flag() { true }`，接口/契约也不引用该子句 | 无该子句的值依赖 | 把外围所有参数加入 flag，产生自等待 |
| helper 主体恒 true，但参数域或 requires 使用外层 T | T 及其必要域/前提 | 只扫描函数体，宣称无捕获 |
| f 调 g，g 读取外层 seed | 经调用替换后的 seed；递归组保留该闭包 | 只收集直接自由变量，调用时缺输入 |
| 引用 pointer to Link 的大小 | pointer 类别及 target 指针表示 | 先要求 Link 的全部布局，制造递归布局环 |

第一项对应 `compiler/luna/tests/function_template_local_definitions.rs` 中的
`local_comptime_calls_in_clause_defaults_need_precise_lexical_dependencies`。
2026-11-04 已用 `--ignored --exact` 实际运行该测试，未改动源码里的忽略标记或预期：

```flurry
template fn choose(value: bool) -> bool
where .inverted: bool = do {
    pure comptime fn flag() -> bool { true }
    flag()
};
fn first use choose<> { value != inverted }
fn run() -> bool { first(false) }
```

预期为 true；实际在真实 sysroot 的前端检查阶段失败，报告
`required comptime evaluation failed: interpretation Type at (source 151 157)`，
尚未进入 run 的 reference 执行。精确命令为：

```sh
python3 -B development/check.py --timeout 180 -- test -p luna --test function_template_local_definitions local_comptime_calls_in_clause_defaults_need_precise_lexical_dependencies -- --ignored --exact
python3 -B development/check.py --timeout 60 -- test -p luna --test function_template_local_definitions clause_defaults_rebind_their_local_definitions -- --exact
```

| 对照 | 实际结果 | 证据 |
| --- | --- | --- |
| 上述局部 comptime helper | 测试退出 101，前端拒绝；正确正例未通过 | `/tmp/luna-tools-vsh5q_r8/result.json` 与 stdout.log |
| 同模块局部 typealias 的子句默认 | 测试退出 0，预期 true 成立 | `/tmp/luna-tools-bvi2f4uy/result.json` 与 stdout.log |
| 只将同一个 flag 定义移至顶层，保留默认中的 flag 调用 | reference 执行退出 0，`(bool true)`，stderr 空 | `/tmp/luna-tools-4_cv4k1b/result.json` 与 stdout.log |

第三项的原始/变体输入在 `/tmp/luna-lexical-audit-354y8bfn/{original,hoisted}.fl`，
执行命令完整保存在对应 result.json；使用[运行基线](architecture-audit.md#6-可复现的现状基线)
中的独立 driver，已重新核对二进制 hash 与 compiler/library 源码未变。
Cargo 测试同样包含工作区原有资源分析修改，不是纯 HEAD 构建。

静态路径 `ensure_signature` 先继承父 telescope/captures，再添加父 staged inputs，
与上述对照共同支持精确词法依赖作为修复切入点。本轮未采集逐步执行轨迹，不能仅凭
宽泛的 `interpretation Type` 诊断宣称已经排除所有其他原因。

迁移验收应直接展示 flag 的实际隐式 telescope、每个输入的依赖来源及调用供应结果：
此例不能包含未被它使用的 inverted；另一个只在参数域或 requires 使用 inverted 的
变体仍须保留该依赖。不能以全局外提 helper、删掉检查或硬编码恒 true 作为修复。
真正的未供应输入应诊断依赖路径、绑定来源和当前阶段，不能只给一个 Type 错误。

## 6. 接口生产必须有可观察前缀

定义接口的任务按声明依赖供应已成立部分：已注册的 binder 身份、已形成的参数域、
有依据的阶段/调用形状、已检查的契约和必要的默认描述。
这些不是一个“部分 Function 可以随便调用”的对象，每个 facet 各有消费权限。

例如必须先知道参数 0 的 Type 值才能形成参数 1 的域，参数 1 的域观察应依赖该值，
不能要求整个函数主体检查完。反之，调用不能用参数 0 已形成来忽略未完成的结果域。
普通自递归可以通过已声明且合法的接口；具体 const 计算需要自身结果时仍报真环。

旧 active_signatures 的 owner 集合可以辅助防重入，但不足以表示这些不同粒度的环。
新的依赖路径须标明是“形成域”“取得值”“验证契约”还是“交付布局”。

## 7. 冻结不能成为切断依赖的捷径

region 关闭时对必检义务逐项判定，只有语言允许的残余可导出。残余含原操作、稳定输入、
绑定、前提、控制条件及合法延期依据；局部 metavariable 若没有合法外部对应则拒绝。
不能看到某任务仍 Pending 就为它自动引入新泛型参数。

冻结保留 symbolic Type 和合法 runtime identity；不要求把它们变成闭合 witness。
引用端替换与 EnvironmentPlan 的生成是后续实例化的责任，旧活动 observer 不跟过去。
实际 CodeValue 的固定依赖及 Any 背后的原接口仍在冻结值中，不能用描述类型取代实际证据。

## 8. 稳定 query 的输入和失效

query 依赖的是稳定语义输入，不是执行时遇到的任意 Rust 地址。
WorldRevision 可作为初始保守缓存隔离，但不能宣称它天然提供细粒度增量；要缩小失效，
必须记录实际观察 facet 的版本/指纹和前提，并验证同一 SemanticView。

| 结果 | 主要依赖 | 不足的 key |
| --- | --- | --- |
| 定义接口 | binder、签名表达式、属性、实际读取的类型/配置 | 源码路径或函数名 |
| 实现选择 | 目标接口来源、phase、主体/参数、候选 universe、前提 | 仅命中的 impl 身份 |
| NF 形成/域检查 | 原头身份、普通实参、依赖域及契约 | 仅参数数量或 Role::Tuple |
| 代码对象 | 主体、阶段/实例契约、宿主操作接口、实际观察的世界部分 | declaration ID 或返回类型 |
| 资源摘要 | 对应 normalized 定义或 instantiated 产物、检查模式、契约、环境前提、回调和 cleanup | 函数签名或纯度等级 |
| 目标产物 | IIR、所用主体、ABI/target、库/工具版本、实际输入 | 前端检查成功标志 |

结果可以带局部观察投影，允许接口未变时复用调用者检查；必须真正比较该投影，不能
给一个全包 hash 命名为“增量接口”。负查找记录检索空间，新增兼容候选会使它失效。
来源重映射失败时重新生成诊断，不把旧行号直接附到新文件。

## 9. 查询缓存和源执行不能混在一起

缓存 OpenDefinition 中的不纯 residual 不等于缓存其执行结果。导入冻结定义时，为每次
语言要求的执行建立新的进度身份；同一次活动检查重试只恢复原身份。
纯 Type invocation 的结果缓存继续独立遵循作者限制，实参准备和接收方准入不因命中省略。

若稳定查询计算中发生源程序要求的外部行为，需要将“可复用静态结果”和“本次执行需求”
分开记录。不能仅凭调用描述相同就跳过执行，也不能在每次观察同一任务时重复执行。
不可回放的宿主行为遵循 [请求状态协议](elaboration-state-and-publication.md)。

### 命中静态产物仍需处理本次执行责任

复用入口至少区分三种情况：

| 情况 | 可以复用什么 | 仍须处理什么 |
| --- | --- | --- |
| 产物不包含必须重新兑现的源执行责任 | 有效静态结果及其依赖、证据、诊断 | 当前接收世界与来源准入 |
| 同一次源执行已完成，观察者再次请求结果 | 该执行 owner 的完成记录及合法产物 | 原 RequestId、结果所有者与发布状态；不再执行一次 |
| 源语义要求一次新的执行 | 代码、稳定接口、合法冻结 residual 等准备材料 | 新执行进度及结果；不能沿用旧完成记录 |

“新的一次执行”由源调用、实例和阶段上下文确定，不由用户按了几次查询按钮、缓存
是否淘汰或进程收到几帧请求决定。多个观察者可以等待同一源执行；同一函数同实参的
两个源调用也不是同一执行身份。允许的纯结果复用另按 invocation 契约处理，
稳定查询 key 不承担动态执行身份。

拟议复用检查接收静态候选、当前语义视图和本次执行责任，返回可直接接纳、需继续本次
执行、需重新计算静态部分或失败。缺少执行责任摘要时不能假定“没有副作用”。
摘要记录执行来源与动态 owner 的对应、哪些稳定结果依赖执行所得值、发布产物及未完成
外部动作；它不是可以逐条重播的副作用日志，也不保存旧 VM 的活动栈。

**执行一次后继续返回旧静态值也可能错误。** 若一次获准的新执行产生新 Type、定义、
属性或其他被检查器读取的值，要以新结果核对相应观察及身份。仅机器布局相同不证明
新旧类型相等；把日志输出次数补齐也不能让旧类型检查结果重新有效。

处理顺序是：

1. 根据源阶段和控制条件判断本次确实需要的执行，不为缓存验证抢先运行未来分支。
2. 用当前执行 owner 继续或完成调用；已完成步骤保留结果和发布收据。
3. 按真实域和观察契约核对依赖该结果的静态候选，不调用任意用户 Eq 冒充类型身份。
4. 依赖变化时只重新计算受影响的静态工作，沿用第 2 步的完成状态，不重放该调用。
5. 所需检查与接收者准入闭合后，才发布本次 query 成功结果。

不能可靠分离静态工作与执行进度时，先拒绝对应缓存候选；这不授权从头重跑一个已经
发生外部行为的活动任务。失败后的全新调用与同一次调用的恢复仍由执行 owner 区分。
生成定义的重新绑定须验证整个相关依赖闭包，不能只将返回 Type 的声明编号换成新编号。

### 现有缓存的保留点与审计边界

[CompilationCache](../compiler/luna/src/compilation_cache.rs)目前检查完整请求、工作目录、
源码/发现输入、入口路径与目标；它在重验前移出旧候选，失败不回退到旧成功结果。
[驻留入口](../compiler/luna/src/package_stream.rs)仅接收无输出路径的 Compile 动作，
拒绝显式 invocation/interpret/build。这些保护应保留。

但是“命令是 Compile”不等于“前端内部永远没有源执行”。
[required comptime](../compiler/elaboration/src/program/required.rs)具有求值及生成定义的
接线，而 [CompilationInputs](../compiler/luna/src/packages.rs)当前主要记录源码与发现
输入，并不包含上述执行责任摘要。扩大缓存覆盖前必须沿实际调用链核对能到达哪些源
操作，不能只凭 compilation_cache 的模块注释证明所有未来 frontend 产物都可复用。
本轮未复现因此导致的错误省略，不将这个待核对边界写成已经确认的运行回归。

诊断恢复也要区分编译器诊断与源程序输出：前者按逻辑错误、来源和上下文呈现，后者
受源执行次数约束，不能把一次打印当作缓存诊断随意去重或重复输出。
当前 pure Type invocation 缓存限制继续独立执行；不能把裸 invocation 套成一个
“稳定查询”就扩大任意返回值的结果缓存。真正稳定的检查产物按本节契约复用。

## 10. 成本与可观察性

词法摘要按新增依赖增量传播，索引 declaration → used binders，避免每次替换先构造
全包 binder。循环组固定点的成本取决于依赖边和新增条目，不承诺任意程序线性。
任意 meta 编程产生新的声明/请求时，工作量继续计入发布及检查预算。

dump 示例仅表示内部关系，不是源码语法：

```lisp
(observation
  (subject (declaration 7 12))
  (facet (parameter-domain 1))
  (view (world 4) (scope 9) (frontier 3))
  (state (pending (producer 21 2)))
  (dependencies (binding 7 13))
  (source (file 2) (start 80) (end 95)))
```

最终格式随 IR Display 一起维护：排序稳定、符号转义、world/owner/generation 明确。
禁止把 dump 当持久 wire 协议；也不以日志顺序作为依赖顺序。

验收要改变合法出队顺序、输入供应顺序和无关声明规模，并核对相同语义结果、真实执行
次数与正确失效。预算差异单独记录，不将较慢路径的 Unknown 当成另一种类型答案。
