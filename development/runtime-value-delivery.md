# 从 comptime 输入到 runtime 值的交付

本专题补齐 M2/M4/M5 之间的阶段交接，源于真实 std writer 暴露的问题。
这里描述目标职责与迁移条件，不表示这些载体已接入编译器。正式语义以
[跨阶段值传递](../docs/comptime/quoting-and-splicing.md)、
[函数参数阶段](../docs/functions/comptime_fn.md)和作者关于 comptime 值引用的决定为准。
现状复核基于 `689d45c5` 的工作区，不改正式 docs，也不把当前残余重建方式补定为
语言规则。工作区原有资源分析排版改动仍在，不能称为隔离的纯 HEAD 验收。

## 1. 已确认的现状分裂

同一个源值从不同入口到达 runtime，目前经过不同检查：

| 入口 | 当前消费者 | 实际行为 |
| --- | --- | --- |
| 命名常量 | expression_values/bindings → splice_constant → splice_value | 选择普通 runtime Splice 方法，以 comptime 输入实例化 |
| comptime 函数结果 | call/finish_prepared_call 等 → deliver_comptime_result | runtime 数据使用走转换；作为下一个 comptime callee/Type 构造头时保留源值 |
| 普通函数的 comptime 参数 | expression_values/bindings 的 ResolvedFnParam | 通常直接生成 K::Local，先按源域核对 runtime 预期域，没有同等的交付步骤 |
| 传给后续 staged 输入 | iir::staging::substitute_forwarded_inputs | 在被标记的 staged 位置替换已完成值，检查写入/地址逃逸与捕获作用域 |
| 剩余参数读取 | iir::staging::wrap | 以 runtime Let 恢复值，随后按 runtime 移动、捕获、存储规则分析 |

源码锚点：
[参数读取](../compiler/elaboration/src/program/expression_values/bindings.rs)、
[转换选择](../compiler/elaboration/src/program/splicing.rs)、
[阶段输入重建](../compiler/iir/src/staging.rs)。这些路径不能仅因都能输出 Expr
就视为同一阶段语义。

[类型改变复现](repros/staged-parameter-splice.fl)使用普通用户类型 Seed，其 Splice
的 RuntimeTarget 为 u8。直接返回 comptime 参数时，当前前端报 Seed/u8 类型不匹配。
[对照](repros/staged-parameter-splice-control.fl)中的命名常量和显式 Splice 调用均能
完成相同转换。显式调用是定位对照，不是要求用户改写所有参数读取的最终接口。

[文本观察复现](repros/staged-text-observations.fl)的域相同，因此逃过上述类型差异；
直到 residual 把参数恢复为 runtime str 局部，第二次读取才报 unavailable value。
这两个错误共同暴露阶段交接缺口，但拒绝来自不同检查层，不能假定一个补丁就能
同时解决，也不能只在文本优化或 Copy 查询里处理。
文本路径还缺 str 的 Splice/目标存储来源接线，不能仅消掉移动错误就宣称完成。

## 2. 先明确输入、使用与目标

参数 sort 决定输入在哪里形成，不自动决定每次读取如何进入另一阶段。
要分别保存：

- 源绑定身份、所属 activation/scope、实际域和输入阶段。
- 当前使用的角色：输入供应、类型/定义观察、代码拼接、普通 runtime 值或合法位置。
- 接收端的预期域、调用接口与需要执行的构造；不能先把源域塞进 runtime 位置再补查。

这些事实属于共同输入与检查上下文，不能以 str、Tuple、meta.List 等名字重建。
NF 输入仍是普通 comptime 值，normal-form 头仍不展开。已知同一结构视图不代表源域
与 RuntimeTarget 相同，也不授予 Copy、Drop 或内存访问能力。

以下是职责划分，不是新增源码语法，也不是已存在的 Rust 类型：

```text
SourceValue = (binding-or-result, domain, phase, formation-owner, availability)
UseContext  = (consumer-role, expected-domain, lexical-scope, execution-boundary)
Delivery    = same-stage-reference
            | checked-runtime-construction
            | contextual-code-expansion
```

same-stage-reference 不复制 producer；checked-runtime-construction 保留选中的
Splice 声明/实例、已形成输入、RuntimeTarget 与普通调用契约；代码展开沿既有
CMTT 规则，不把 quote payload 当作普通 RuntimeTarget。尚未完成的域/实现选择
继续属于活动检查状态，不能冻结为“直接恢复一个 Local 即可”的成功计划。

## 3. 转换发生在值消费边界

不能在遇到任意 ResolvedFnParam 时机械插入 Splice：

```flurry
struct Seed { byte: u8 }

fn field(comptime data: Seed) -> u8 { data.byte }
fn explicit(comptime data: Seed) -> u8 { Seed.as(Splice).splice(data) }
```

第一处需要先在源值上解释字段观察，再交付观察结果；不能先把 Seed 转成 u8，
再尝试访问 u8.byte。第二处 data 是 Splice 方法的 comptime 实参，不应再次要求
runtime 转换。把入口合流做成“所有名字读取一律 splice”会立即破坏两者。

检查器应在消费角色已经明确的地方要求交付，保留同阶段投影、参数准备和类型观察。
源值已知也不授权在 comptime 调用一个 runtime 函数：普通 runtime 操作继续留在
生成程序中，除非后续优化有保持行为的依据。不能把这一设计理解为任意封闭表达式
都可以提前执行。

参数、命名常量、已完成调用结果和合法捕获应使用同一交付服务；入口差异只负责
取得各自的源值/来源，不各自重新决定运行时构造规则。预期类型引导交付检查，不能
为满足预期而改写源值域或忽略选定 Splice 的关联 RuntimeTarget。

规格表达式仍遵守原有阶段与纯度规则，safe predicate 不能因此被改成 comptime
执行。不纯的构造不能被植入规格后免检；既有 staged aggregate 契约也不能因参数
变成普通 runtime Local 而丢失其源值事实。程序规格与类型命题仍由不同层处理。

## 4. 保存形成次数与执行位置

comptime 输入形成、Splice 实例准备和 runtime 构造是三件事。
已完成的输入可以供多个使用点引用，代码实例可以共享，但 runtime 构造仍按各
使用点及控制流执行。不能缓存构造结果来代替有副作用的 Splice 调用。

| 源码情景 | 必须保留的行为 |
| --- | --- |
| 同一源常量在两个执行到的值位置使用 | 各自的 runtime 构造执行；不因输入相等合并可观察副作用 |
| runtime if 的两支都使用同一常量 | 只执行所选支的构造 |
| 显式 runtime let 保存转换结果 | 后续使用该 runtime 值，按其资源规则复制/移动，不重新运行源 producer |
| runtime 闭包主体读取 comptime 输入 | 输入作为代码准备依赖保留；主体的 runtime 构造属于闭包调用 |
| 在闭包形成前先保存 runtime 转换结果再捕获 | 构造在形成前发生，捕获取得实际 runtime 值及其资源责任 |
| 默认输入或参数展开 | 沿既定供应顺序和提供者规则执行；交付不能重放已完成的默认值/实参 producer |

副作用、异常/错误、拒绝、清理和生命周期均属于构造行为。只保留结果类型，或把
Splice 当 comptime fn 执行，都不满足该交付契约。已提交 native 对照覆盖命名常量、
显式调用与所选 runtime 分支，尚未分别覆盖隐式参数读取、转换后局部复用、闭包、
默认值和任意聚合。不能把三个输出次数推及这些未执行的场景。

## 5. 构造的底层终点必须明确

[builtin 的标量 Splice](../library/builtin/protocols/staging.fl)当前采用返回 data
的身份实现。它依赖现有参数重建通路。如果直接把所有参数读取改为再次调用 Splice，
这些方法会形成自递归；对泛型返回、字段选择和同域用户转换同样不能草率处理。

迁移应明确由类型/阶段检查授权的字面量或代码构造底层能力，再由普通 Splice 实现
使用它。底层构造负责目标常量/代码表示，不负责替普通源值选择 Splice。它属于
六包集中绑定与受检 IR 的构造边界，不新增源码关键字；具体绑定接口尚未交付。
不能通过 `函数名 == splice`、内置包名称或“输入输出类型相同”跳过正常转换。
即使源域与目标域相同，用户实现也可能有可观察行为。

这也是普通 runtime literal 与 comptime 参数读取不能混为一类的原因：前者已有
字面量构造依据，后者先要解释跨阶段交付。允许直接常量表示并不等于允许删除
本应执行的自定义转换。优化必须以已经选定且检查过的构造为对象。

源为 str 时还必须提供目标数据的有效来源。静态文本可以由合法常量存储支持；
动态字符串、切片、拥有型对象和带引用的图各有不同依赖。全局追加 Copy、将源 VM
地址带入目标程序或将所有 frozen payload 直接当 native 数据均不成立。

## 6. 替换 wrap 的条件

当前 wrap 同时承担“给残余补一个局部位置”和“把完成的 staged 输入恢复出来”。
这使源阶段信息消失，后续资源错误才暴露矛盾。退役时要分开：

1. 仅供 comptime/类型/代码准备使用的输入不获得 runtime 存储；普通值与代码
   的实际准入仍保留，不能因没有 runtime 存储而跳过域检查。
2. runtime 值使用携带明确构造计划，完成后的调用/构造结果才遵循 runtime
   存储、Copy/Once、借用与 Drop 规则。源 comptime 槽不以同名 Let 伪装成该结果。
3. 合法的 runtime place/临时位置由对应表达式规则创建，不能通过抹除源阶段
   制造一个可在 runtime 回写的 comptime 参数。显式 runtime 副本拥有独立位置。
4. 闭包、默认值、入口快照与契约按各自作用域改写；不能全局替换相同 LocalId，
   也不能把完整 producer 复制到每个使用点。
5. 构造选择和输入准备失败时不发布部分实例。NIR safe 检查、选定 residual 的
   资源检查、IIR 与后端应消费同一交付决定，不能各自猜测常量或指针的来源。

同一个 wrap 调用是否可删除，取决于对应位置已经由上述能力接管，而不是当前
fixture 是否能展开为 Literal。先移除它再依赖各后端补猜，只会制造另一套真相。

## 7. 实际消费者与交付状态

新服务应替换下列职责，不是在 `wrap` 前后附加一张只有生产者的表：

| 当前位置 | 应交付的信息/工作 | 不得继续承担的职责 |
| --- | --- | --- |
| elaboration 的参数、常量、调用结果读取 | 源绑定/结果身份、阶段、域、形成位置与消费角色 | 以源域直接核对 runtime 目标，再把失败归为普通类型不匹配 |
| `program/splicing.rs` | 在明确的 runtime 消费点选择普通协议，检查关联目标和调用契约 | 猜测一个 Local 属于哪个阶段，或以 callee 名字跳过转换 |
| `materialization` 的 staged input 准备 | 按现有输入顺序取得值；把稳定结果和选定实例一起交付 | 重放已经完成的 producer，或用资源检查失败触发另一套准备路径 |
| `iir::staging::bind_inputs_with_layouts` | 在正确 scope 中代入源输入，保留运行时构造位置和契约 | 仅因参数 ABI 已擦除，就为每个存活读取制造 runtime Let |
| NIR safe 与实例资源检查 | 分别验证源阶段使用及构造后的目标资源/来源 | 把 comptime 值引用当 runtime Copy 义务，或因源是常量而免除目标借用检查 |
| IIR 与后端 | 消费已选实例、合法常量表示和明确存储来源 | 自行再选择 Splice、重建源 VM 地址、按名字恢复阶段 |

当前 materialization 的 `bind_call_with_layouts_and_type_views` 调用还需要保留
trait 来源、接口契约、类型视图和程序义务。迁移不只改主体：`bind_inputs_with_layouts`
中的前/后置条件及默认输入也调用 `wrap`，闭包初始化由自己的捕获 frame 维护。
这些消费者须一起纳入范围，不能只让返回值用例通过。

准备按以下完成点区分，名称仅描述实现职责：

```text
(source-use (binding ...) (phase comptime) (domain ...) (scope ...))
    -> (delivery-demand (role runtime-value) (expected ...) (control ...))
    -> (selected-construction (impl ...) (target ...) (world ...))
    -> (prepared-input (value ...) (formation ...) (dependencies ...))
    -> (checked-residual (call ...) (contracts ...) (storage ...))
```

这是未来 Display 的结构约定示意，并非当前可导出的 IR。稳定 dump 应能看出源输入
与目标调用是两个实体，显示绑定/控制/实例关系，并正确转义名称；实现新 IR 时同步
提供携带上下文的 Display，不以地址或 Debug 输出充当身份。

阶段选择可能依赖待定域，输入准备也可能需要源 invocation。等待应归现有 Region/
准备后继，持有必要 world 与 source scope；同一次恢复不重跑已完成输入。不发布
携带活动 Meta 或候选选择的“完成”对象，不把阶段错误降成直接传送源值的 fallback。
代码实例的缓存可复用检查产物，但执行时仍保留每次 Splice 调用；不扩大当前仅 Type
结果 invocation 的缓存范围。

错误需要保留失败的真实边界：缺协议/实现、目标域不符、源输入尚未完成、资源耗尽、
目标表示无效分别报告。Seed 正例当前在选择前报源域错误，是待修行为；不能只换一条
错误文案后把它登记成已接通。没有合法 runtime 用途的 Type/代码值也不能靠新交付
机制获得权限，仍走原有类型观察或 CMTT 的上下文准入。

## 8. 纵向迁移与验收

这条工作线归入现有 M2/M4/M5，不是新的并行框架，也不建立额外调度账本。

| 迁移闭包 | 必须接通 | 退役/拒绝条件 |
| --- | --- | --- |
| 源值与使用上下文 | 参数、常量、函数结果、投影、所需阶段和预期域 | 不能只修返回位置；同阶段输入及类型/callee 观察不能误插转换 |
| 构造选择和底层表示 | 实际 Splice 身份、普通 runtime 调用、基础字面量构造 | 同域转换不跳过副作用，builtin 身份实现不自递归；缺实现/错误目标给阶段交付诊断 |
| 参数/捕获残余 | 稳定源值依赖、作用域、默认供应、真实 runtime 存储 | producer 不重放；闭包捕获与调用时构造不混淆；源值不变 runtime 可写槽 |
| 文本与库 | str 的转换及来源、TextBytes、通用 writer 文本方法 | 保留生命周期/读写检查；源码文本复现真正贯通，不只是换一个错误 |
| 两套执行与后端 | comptime invocation 的输入规则、runtime reference、LLVM O0/O3 | 执行次数、错误、清理及未选分支一致；活动域/预算失败不污染下次准备 |

已确认的未知仍保持未知。实现这些路径不需要修改公开 callable Type 的名字相等性
决定，也不应借此扩大 Any、quote 或 PTC 的权限。新载体必须替换真实消费者后才可
计为实现进度；仅增加枚举、side table 或成功打印的专用分支不算接通。

## 9. 证据与当前限制

此次收拢阶段分析和源码复现，运行行为对照已在 `689d45c5` 提交；没有改变 compiler 的交付语义，
没有宣称修复隐式参数转换或文本来源。工作目录均为 `/home/fin/luna`。

- `library_std::splice_construction_runs_per_use_after_staged_inputs_are_ready`
  使用真实 sysroot 与资源检查，在 O0/O3 输出三次 `convert;`：命名常量、显式
  staged 参数调用、runtime 分支各一次。构造没有提前进入 comptime 或被当作
  相同输入的缓存结果。测试通过，命令为
  `python3 -B development/check.py --timeout 900 -- test -p luna --test library_std -- --test-threads=1 splice_construction`，
  `/tmp/luna-tools-v4xslvoa/result.json`，退出 0。
- Rust 测试文件格式化退出 0，`/tmp/luna-tools-ifwb1n06/result.json`。
- `689d45c5` 上完整 `library_std` 定向组已通过 11 项，报告为
  `/tmp/luna-tools-xi8zy3i8/result.json`。这是前一轮证据，本轮没有重复该测试组。

本轮在仓库内复现，构建共用以下选项（SOURCE 和 OUTPUT 是下面各行的实际路径）：

```sh
python3 -B development/check.py --timeout 900 -- run -p luna -- build SOURCE --entry run --sysroot library --fuel 10000000 --output OUTPUT
```

| SOURCE | OUTPUT | 构建结果与报告 |
| --- | --- | --- |
| `development/repros/staged-parameter-splice-control.fl` | `/tmp/luna-staged-parameter-splice-control` | 退出 0，`/tmp/luna-tools-67x1wz8z/result.json` |
| `development/repros/staged-parameter-splice.fl` | `/tmp/luna-staged-parameter-splice` | 退出 1，预期 u8 而得到 Seed；`/tmp/luna-tools-0p0s7ok6/result.json` |
| `development/repros/staged-text-observations.fl` | `/tmp/luna-staged-text-observations` | 退出 1，资源检查报 `use of unavailable value`；`/tmp/luna-tools-ec4qzde3/result.json` |

执行 `python3 -B development/command.py --timeout 30 -- /tmp/luna-staged-parameter-splice-control`
退出 0，输出 `(bool true)`；`/tmp/luna-tools-2j0phy_q/result.json`。
失败行是仍待修复的合法源码，不是正式拒绝测试；保留成功对照用于区分协议不可用
与参数入口缺交付。上述准备状态和底层构造接口仍为设计，不能计为新的生产接线。
