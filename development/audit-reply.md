# 源码检查与执行的架构复核

11 月 18 日晚报以 `2797e5f6` 为基线。以下按可定位的生产入口区分已经接通的能力与下一处必须改变的接口。报告后的修正分别标在对应入口。早期逐次检查的日志随各自提交保存，初始 R01–R20 的语境见[架构审计](architecture-audit.md)。

## 源码检查与原调用的交接

普通 comptime 函数可以返回 Type。假设 `count` 接受 `where T, ...values: meta.List<T>`，下面这次调用的参数书写顺序与参数域依赖顺序相反。

```flurry
let size = count<
    .values = do { trace = trace * 10 + 1; [1, 255] },
    do { trace = trace * 10 + 2; Byte() }
>();
```

调用准备有两个次序。检查器可沿着 `values → T` 寻找类型信息。调用者从左到右执行实参，第一次写入先于第二次写入。`Byte()` 在它自己的执行位置返回 Type 后，检查器才取得整数文字的元素域。如果先单独执行第二个实参，再重新进入整次调用，就会改变写入的次数或顺序。用 `Byte` 的函数体推测一个域，也会丢掉这次普通调用的执行行为。

| 当前所有者 | 已持有的内容与返回协议 | 交接仍缺什么 |
| --- | --- | --- |
| [检查区域](../compiler/elaboration/src/program/block_regions/driver/region.rs) | 区域持有检查帧、结果消费者与 HIR 借用，每次 poll 单独借用 Checker。声明实参和普通调用的依赖准备都可作为根消费者取得已检查的结果。[驱动](../compiler/elaboration/src/program/block_regions/driver.rs)在一次同步调用中推进到完成。 | 一次源域观察需要保存尚未完成的检查以及交付目的地，允许原调用稍后送回一个值。 |
| [实参依赖任务](../compiler/elaboration/src/program/dependent_inputs/tasks.rs) | 按槽位调度普通参数域与默认值的依赖，所需源码实参通过共同区域接受 staged 检查。 | 此队列只知道依赖哪个槽，缺少原实参的执行位置、活动绑定和控制出口。 |
| [required 入口](../compiler/elaboration/src/program/required.rs) | 为已有受检根准备运行所需的声明和可用输入。 | 尚待元素域的源码表达式无法作为受检根交付。重新准备一个根也无法恢复原调用栈。 |
| [调用会话](../compiler/comptime_interpretor/src/invocation/session.rs)与[宿主接口](../compiler/comptime_interpretor/src/invocation/host.rs) | VM 可以在原调用中运行已检查的实参片段、传递循环控制，请求选择或定义。 | 收到 Type 后仍缺将它送到原源码检查位置，并以检查结果继续该调用的路径。 |
| [定义端片段检查](../compiler/elaboration/src/program/open_functions/fragments.rs) | 将可见输入投影到紧凑参数帧，重绑定片段中的引用，NIR 的 `DefinitionRegion` 在真实依赖目录旁独立接受阶段和算术检查。 | 投影参数只有定义端的域与前提，实际存储、已执行前缀和外层控制目标仍归原调用持有。 |

定义片段记录词法所有者、重绑定后的参数与类型前提，以及被检查的表达式。阶段检查验证这份参数帧，调用引用仍在真实声明目录中解析。即使片段与目录中的某个声明来自同一所有者，它们各自的算术义务也须分别证明。程序验证直接借用片段的参数帧、类型前提和表达式。若类型前提给出有限的整数域，验证器对每个可行替换分别形成文字并检查运算，保留原片段和源码位置。例如 `255 + 1` 在单一 u16 域成立，加入 u8 就失去溢出安全证明。函数默认实参单独验证，此时尚不能使用被调用函数的入口前提。依赖声明和方法契约仍随临时依赖包检查。[阶段遍历](../compiler/nir/src/phases.rs) · [程序验证](../compiler/program_verification/src/flow.rs)

列表与元组的字段检查也使用源码区域。帧按书写顺序保留已经检查的字段；已知列表元素域可复用于各字段，元组按位置取得各自的期望类型或调用候选给出的部分上下文。上下文仍待确定时，接受这种输入的消费者保留原文字与嵌套结构。普通 callable 的动态实例持有已经检查的准备语句和声明槽位。尚待后续 Type 的列表无法交付其中的已检查操作数，仍需要下述跨源码与 VM 的恢复机制。[字段检查](../compiler/elaboration/src/program/block_regions/aggregates.rs) · [源码调用用例](../compiler/luna/tests/normal_input_plans.rs)

已经进入 VM 的声明实参用结构位置标识选中的文字与集合，原调用保留已求得的值。后续绑定交付类型时，VM 检查原始模板中的域标注，包含未选分支。源码执行器在域已知、待定两种情况下共用字段游标与词法退出；字节码和源码执行器都把延后展开的错误定位到原字段。普通 callable 中尚未交付给 VM 的表达式仍受上表的源码检查限制。[执行与域交付](values-staging-and-publication.md#接收域到达前后的执行) · [输入恢复](../compiler/comptime_interpretor/src/input_collection.rs)

### 一次调用内的恢复

1. **建立输入位置。** 调用准备记录书写顺序、公开槽位、源码位置和接收域的依赖。每个尚待上下文的表达式保留其原词法环境与目标位置。域已知且冲突时直接报告，即使另一个槽位仍未决。
2. **执行已定的前缀。** 上例先完成第一次赋值。整数文字保留原始值及其待检查位置，等待元素域后再完成构造。对于 `[byte]`，读取必须发生在源码规定的位置，否则后续的 `byte = 9` 会篡改较早的输入。保留读取结果或合法值引用取决于原表达式的值语义，不能统一深拷贝或推迟所有读取。
3. **由原调用交付 Type。** 执行游标到达第二个实参时才执行赋值和 `Byte()`。结果连同所属世界和请求位置返回原检查区域。在该域下检查范围、收集或拒绝字段，随后进入 callee。检查期间可以查询稳定的声明与代码。堆栈、局部存储、预算和活动授权随本次调用存活。
4. **保留退出目标。** 如果第一个实参执行了外层 return，后一个实参和 callee 均不会执行。break 和 continue 回到词法目标，错误随原调用传播，等待该路径的检查结果不得触发再次执行。静态上可判定的定义错误仍由定义检查负责。
5. **区分可延后的检查与尚不能执行的操作。** 上例可以延后文字的域检查。若先前操作本身的选择或副作用取决于后续 Type，分割前必须说明在语言求值次序下能先观察什么。当前三个例子没有给出这种依赖环的普遍解法，不能据此授权重排、投机或增设拒绝规则。

[源码顺序用例](../compiler/luna/tests/type_input_execution_order.rs)逐项区分三种观察：最终 `trace == 12` 且 `Byte` 与 `count` 各进入一次；早退时两者均不进入；较早读取保留 7 且最终局部值为 9。已知域及类型已确定的参数包提供对照。以下命令显式运行待实现用例。此次工作区得到 11 通过、3 失败；三项都在列表缺少元素类型上下文时停止，尚未执行到 VM。这与后面依赖任务拒绝越序执行是两个不同位置。

```sh
python3 -B development/check.py --timeout 900 -- test -p luna --test type_input_execution_order -- --include-ignored
```

另一个对照是[名义值参数组](../compiler/luna/tests/nominal_value_parameters.rs)。在独立的 `3b4b54d9` 源码快照中运行整组，297 项通过、11 项失败。失败涉及开放父参数的成员域、别名文字的整数域要求、required 输入可用性及旧用例缺少 Splice 协议。复核输入恢复时，应按用例和诊断与这个基线比较。

这条迁移的验收需要同时观察域、值、控制出口和调用次数。原检查与执行协议闭合后，才可删除专为探测域而重复准备表达式的路径。两种执行入口及会话内重复调用应保持同一可观察行为。完整的默认值、签名发布和取消边界见[状态与发布协议](elaboration-state-and-publication.md)。

## 公开接口与值传送

动态 NF 的位置输入已经进入声明输入计划。具名供应 `F<.Es = Es>` 仍需要目标函数类型承载公开名；完整列表本身就是一个普通 comptime 值。按槽位适配的实现可以将该槽声明为 `Items`，调用点仍按目标接口使用 `Es`，默认供应也由目标接口决定。更名、前向转交与缺省值的证据须随接口传送。通过名字识别 Tuple 或把列表改为特殊 NF 参数均不能交付这个协议。[接口设计](callable-input-interfaces.md) · [动态入口复现](repros/dynamic-nf-interface-forwarding.fl)

NF 保留头身份。具体 Type 描述能为消费者提供参数域或结构观察，不能令不透明头与函数体返回类型相等。值的准入还须核对接收世界。恒等交付一个已授权的抽象错误结果时，可以保留原值；需要重映射错误标签时才观察具体分支。comptime 值引用的存储同样由当前会话所有；runtime 指针的存在不能用来推导 comptime 的取址能力。[值与发布](values-staging-and-publication.md) · [值准入](comptime-value-admission.md)

## 执行、求解与交付边界

| 晚报范围 | 当前依据 | 后续迁移所需的证据 |
| --- | --- | --- |
| D01–D02 执行与重用 | 分期物化使用源码 VM。声明理论与代码可跨调用共享；活动调用的局部授权保持隔离。字节码仍有清理、效应和深度门控。 | 在原执行栈交付控制与清理。缓存编译代码和缓存 invocation 结果分开；后者仍受纯度、Type 输出、依赖及成本约束。 |
| D03–D05 交织检查、公开接口与 NF | 上述顺序用例定位源码端断点；位置供应已有共同准备入口，NF 仍有专用参数表示。 | 对不同声明头使用同一准入协议，同时保留调用来源、公开入口与结果身份。 |
| D06–D08 runtime 依赖类型与布局 | IIR 的闭合 TypeRef 消费只覆盖空 substitution。`lift` 尚停在 AST lowering，开放 BitVec 位宽仍受限。 | 从捕获到调用和清理跟踪带域的运行时环境，分别验证类型身份与布局取得。[类型引用与环境](type-references-and-environments.md) |
| D09 PTC | 已有 CDCL 学习与回跳。行节点纳入同余议程，增量整数事实可复用类型闭包；模型搜索遇 Trait 仍缺可交付见证。 | 区分理论传播、冲突理由、模型及可用替换。程序状态、内存和副作用规格进入程序验证层。[类型模型与理论证据](type-models-and-theory-evidence.md) |
| D10 查询 | middle 的包级 HIR 查询与 VM、声明目录各有所有者。 | 复用结果需记录输入、世界与失效规则；活动候选和未完成 invocation 留在原事务。[依赖观察](dependency-observations.md) |
| D11 工具与跨阶段代码 | `luna lsp`、`luna fmt` 已接入单 CLI。晚报后 `296ff6b2` 修复 contextual 方法签名与谓词的源码类别检查。 | 普通参数的异构 Splice、模板体、宏和 ABI 降码仍各需独立的跨层交付。[跨阶段值交付](runtime-value-delivery.md) |
| D12 内置包 | 真实 sysroot 的 `println(comptime format: str, ...varargs: defer std.fmt.Parse(format))` 已能在 Linux 原生输出异构字段，按字段建立 Display 证据。 | runtime 文本内存来源、String、分配器与更多 Display 实现仍待交付。[可运行演示](std-host-foundation.md) |

报告中的规模、枚举变体和 ignore 数量只能帮助定位源码。各条结论以对应入口能交付的语义为边界；未来实现应从上述接口接线，保留能区分行为的用例。
