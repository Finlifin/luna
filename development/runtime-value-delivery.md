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
| quote 主体读取生成器数据 | bindings 中的 generator_data / materialize_generator_data 提前返回 | 已有 payload 可直接成为主体数据；当前同域自定义 Splice 的可观察行为可能被跳过，见下文 CMTT 复核 |

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
使用它。底层构造负责目标常量/代码表示，不负责替普通源值选择 Splice。固定字面量
和普通 CMTT 组合已经能表达部分构造，因此不预先要求新增 builtin intrinsic。
若性能需要额外构造操作，应沿六包集中绑定与受检 IR 交付完整契约，不新增源码
关键字，也不将重建任意 VM payload 的内部函数直接暴露为公共逃生口。
不能通过 `函数名 == splice`、内置包名称或“输入输出类型相同”跳过正常转换。
即使源域与目标域相同，用户实现也可能有可观察行为。

这也是普通 runtime literal 与 comptime 参数读取不能混为一类的原因：前者已有
字面量构造依据，后者先要解释跨阶段交付。允许直接常量表示并不等于允许删除
本应执行的自定义转换。优化必须以已经选定且检查过的构造为对象。

源为 str 时还必须提供目标数据的有效来源。静态文本可以由合法常量存储支持；
动态字符串、切片、拥有型对象和带引用的图各有不同依赖。全局追加 Copy、将源 VM
地址带入目标程序或将所有 frozen payload 直接当 native 数据均不成立。

### CMTT 数据读取与代码构造必须分开

在 `dec912f4` 工作区补查：
[quoted-data-splice](repros/quoted-data-splice.fl)定义同域 `Seed -> Seed` 转换，
每次 Splice 输出 `convert;`。一个源常量直接交付、另一个值经生成器 `'(data)`
交付，程序应输出两次；实际只输出一次，然后返回 true。这是成功编译后的行为
偏差，不是缺实现的正常拒绝，不能把现有输出登记成语言预期。

源码上有两处相连的缺口：

- `bindings_value_expression` 的 generator_data 和活跃 receiver 观察分支在普通
  交付前返回重建 Expr；payload 已检查/快照化不等于已经执行了跨阶段转换。
- `quoted_runtime_boundary` 已切换 in_code_body、required 与 staged 状态，但
  `runtime_value_context` 还以外层 signature_owner 的 comptime 修饰判断。runtime
  主体的使用阶段不能只从外围生成器阶段推导，也不能只改一个布尔条件而遗漏前者。

目标行为是先在 quote 构造位置观察当时的源数据，保存其输入身份/快照及转换计划；
Splice 主体放入生成的 runtime 程序。之后再修改生成器局部值，不能改变先前引用；
同一份代码接入两个实际执行的位置，转换也分别执行。纯生成器生成带输出行为的
代码不矛盾，构造代码与执行代码的纯度检查须保持分开。

作为对照，[quoted-literal-control](repros/quoted-literal-control.fl)只用 comptime
布尔值选择 `'(true)` 或 `'(false)`。quote 主体没有读取普通生成器数据，因而没有
被省略的数据转换。这条真实 sysroot 路径可编译运行，返回 true。它是合法字面量
构造的参照，不能据此把 `'(data)` 宣布为免 Splice 的通用操作。

在此基础上，[scalar-code-construction](repros/scalar-code-construction.fl)用八次
迭代从最高位到最低位选择 0/1 字面量，通过单洞 quote 与固定宽度 BitVec 拼接产生
一个字节。源码层每个 quote 只接入前一段代码，没有把 data、mask 或 remaining
嵌入 runtime 主体，也不依赖 u8 的 Splice 身份实现。当前连单个 255 案例仍耗尽
前端准备额度，未取得原生产物，故不能将这条路径称为已可用的数值构造库。

此外，[staged-code-input](repros/staged-code-input.fl)只把上述布尔生成器放入普通
函数，让它接收该函数的 comptime 参数；当前仍报输入不可用。这与 quote 中遗漏
runtime Splice 是两种交接失败：前者要延后到源输入可取得的准备点，后者要在目标
主体中保留构造。不能用直接字面量调用的成功代替参数转发验收。

因此下一项生产迁移须同时明确：哪些读取属于普通数据交付，哪些是代码值接入，
哪些只是生成器自己的源阶段计算。源语法的单洞检查仍按原 CMTT 规则执行；数据
Splice 的 runtime 调用不能被假装成拥有任意上下文权限的代码值。

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

### CMTT 补查证据

本轮先在 `/tmp` 探测，再将下列源码原样保留到 `development/repros`，只添加说明
注释；下面记录实际执行的临时路径，不把未重跑的仓库路径冒充日志输入。构建命令
与前表相同，fuel 均为 10000000，OUTPUT 为 SOURCE 去掉 `.fl` 后的路径。

| 保存的复现 | 实际 SOURCE | 结果与报告 |
| --- | --- | --- |
| quoted-data-splice.fl | `/tmp/luna-quoted-data-delivery.fl` | 构建退出 0，`/tmp/luna-tools-9cpmhs5z/result.json`；运行只输出 `convert;(bool true)`，`/tmp/luna-tools-223qirr0/result.json`，行为仍错误 |
| quoted-literal-control.fl | `/tmp/luna-quoted-literal-control.fl` | 构建退出 0，`/tmp/luna-tools-q9q1aspo/result.json`；运行返回 `(bool true)`，`/tmp/luna-tools-gktyve11/result.json` |
| staged-code-input.fl | `/tmp/luna-staged-code-input.fl` | 构建退出 1，`required comptime input ... is not available in this invocation`；`/tmp/luna-tools-zlz9y20_/result.json` |
| scalar-code-construction.fl | `/tmp/luna-scalar-code-construction.fl` | 单个 255 的八步构造在 generator checkpoint preparation 报 StepsExhausted，退出 1；`/tmp/luna-tools-2hghr316/result.json` |

两个原生产物均通过 `python3 -B development/command.py --timeout 30 -- OUTPUT`
运行，退出 0。退出成功不能替代第一行的可观察行为比较。

规模诊断时还执行过四个字节值的同类构造；fuel 为 10000000 和 100000000 时均在
前端准备失败，报告分别为 `/tmp/luna-tools-0cap8plq/result.json` 与
`/tmp/luna-tools-_qr3cl3x/result.json`。这**不是提高前端额度后的性能测量**：
`packages::compile_observed_with_preludes` 当前调用 `elaborate_packages_for_target`，
后者仍内部提供独立的 1000000 invocation/preparation 步额度，build 的 `--fuel`
没有传入此处。不能据相同失败推断生成器无限循环，也不能把它归为位运算语义错误。
实际准备成本的分布尚未测定；通用预算传送和减少重复准备仍需后续调查，不能简单
调大内部常量或取消准入检查后宣称代码构造已经满足库的性能要求。

### 回滚保留不再准备值观察世界

沿八步构造的失败位置检查，`checkpoint_generator_cells` 原来与真实读值共用
`generator_storage_world` 和 `with_storage_vm`：每个 generator frame、词法 scope
和更新 frame 都会复制表示元数据并准备 VM，即使只为失败时恢复原有存储。
这与实际取得/准入源值的职责不同。现在回滚复用已有的 `capture_retained`；图遍历、
复制及恢复计量保留，实际读值与 old 入口复制的世界检查没有删除。

新增 `cmtt::code_generator_rollback_cost_does_not_scale_with_unrelated_layouts`
用 256 个记录和八层代码生成验证该边界。对照只切换本次修改的 places.rs：

- 旧实现仍使用 `1a6296fb` 的代码，同一新测试退出 101，错误为 generator storage
  world capture budget exhausted；`/tmp/luna-tools-cfryc7bk/result.json`。
- 修改后的单例退出 0；`/tmp/luna-tools-52owl6hm/result.json`。两次命令均为
  `python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt -- --test-threads=1 code_generator_rollback_cost`。

最初用十六层生成器触发了独立的 code construction depth 限制，随后将费用用例
限定为八层，避免把深度限制混入回滚成本测试；没有放宽生产深度或额度。

真实 sysroot 再运行
`python3 -B development/check.py --timeout 900 -- run -p luna -- build development/repros/scalar-code-construction.fl --entry run --sysroot library --fuel 10000000 --output /tmp/luna-scalar-code-checkpoint`，
仍退出 1，现报 `required comptime declaration directory: value admission resource
budget exhausted`；`/tmp/luna-tools-8bygfufu/result.json`。不能以错误位置推进代替
字节构造成功，也不能据这项局部清理宣称准入世界已共享或 CMTT 性能问题已解决。

最终定向检查（工作目录 `/home/fin/luna`，均使用 development/check.py）：

- `test -p luna --test cmtt --test cmtt_methods --test library_std -- --test-threads=1 code_generator_rollback_cost generator_field_updates generator_value_writes generator_storage_checks unused_code_generators_reject generator_readonly_nominal_captures generator_entry_aggregates trait_default_forwarding_preserves_value generator_refutable_tuple_filters generator_var_else_failure generator_callback_snapshots println_and_display`
  退出 0，分别 11、4、1 项通过；`/tmp/luna-tools-2wwhom9y/result.json`。包含原存储
  身份、快照独立性、失败后的状态、runtime 指针域拒绝，以及真实 std 的 O0/O3 输出。
- `test -p luna --test cmtt_storage_world -- --test-threads=1` 为 4 通过、1 失败，
  退出 101；`/tmp/luna-tools-imurfhgx/result.json`。失败的 nested_contextual_retries
  用例在 required comptime preparation 耗尽额度。切回旧 places.rs 后，以同一命令
  追加 `nested_contextual_retries` 复核仍失败，此时在 lexical scope snapshot 耗尽；
  `/tmp/luna-tools-6c7l4gvq/result.json`。这是当前基线的预算限制，没有改测试预期。
- 恢复本次实现后，`test -p elaboration -p comptime_interpretor --lib -- --test-threads=1 storage_observation::tests memory::checkpoint::tests`
  退出 0，3 项观察预算与 2 项存储图测试通过；`/tmp/luna-tools-r3borewb/result.json`。
  图保留不授予陌生名义描述成员资格，失败捕获不修改原图，恢复保留共享身份和责任。

以上命令均以 `python3 -B development/check.py --timeout 900 --` 为前缀。
两个修改的 Rust 文件已经定向格式化；git diff --check 通过。未执行全 workspace。

### Required comptime 准备不再建立 runtime 指针权限世界

`PreparedRequired` 原来在准备源码世界之外，还复制所有已检查的 trait 与 impl，
建立独立的 Copy/Drop 声明目录。该目录只服务于解释器的 runtime 指针读写权限
回调；comptime VM 的 `eval_pointer` 在调用该回调前就拒绝所有指针操作。
因此本阶段删除这份重复世界、其查询回调与 Type Session 的失效安装入口。
comptime 值引用仍通过自己的存储与准入规则处理，不借用 runtime Copy 权限。

runtime 解释器的指针权限检查不变；Type 值描述 runtime 指针、函数签名以及
quote 内的 runtime 指针代码仍合法。实际源码世界的目录、成员查询和存储准入
继续保留。CLI invocation、key 与 memoization 的同类清理已在后续接通，见
[执行角色与资源权限](comptime-value-admission.md#执行角色与资源权限已分开)。
这没有完成通用声明世界共享，也没有完成 staged 参数的 runtime 交付。

八步字节构造仍失败：在 `/home/fin/luna` 执行
`python3 -B development/check.py --timeout 900 -- run -p luna -- build development/repros/scalar-code-construction.fl --entry run --sysroot library --fuel 10000000 --output /tmp/luna-scalar-code-no-pointer-authority`，
退出 1，报 `required comptime preparation failed: interpretation StepsExhausted`；
`/tmp/luna-tools-8oog5bfa/result.json`。失败位置改变不代表代码构造已可用。

当前工作区的端到端检查（工作目录 `/home/fin/luna`）：

- `python3 -B development/demo_println.py` 构建与原生运行退出 0，报告
  `/tmp/luna-tools-alxkh2_h/result.json`。中文、运行时整数与 pid 输出正确，
  产物保留在 `/tmp/flurry-println`，未出现额外的 `(unit)` 输出。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test library_std --test comptime_pointer_phase --test comptime_storage_domains --test cmtt_storage_world -- --test-threads=1`
  退出 0，分别 11、5、7、5 项通过；
  `/tmp/luna-tools-z0uz251g/result.json`。包含真实 sysroot 的 comptime 值引用、
  old 快照、名义对象，以及 runtime 指针类型与 quoted runtime 指针控制用例。
  此前失败的 nested_contextual_retries 用例此次通过，预算常量与测试预期未改。
  std 原生测试覆盖 O0/O3 输出、文件关闭、短写、EINTR 与错误传播。
- `python3 -B development/check.py --timeout 900 -- test -p elaboration --lib -- --test-threads=1 program::required::prepared::tests type_computations::`
  退出 0，24 项通过；`/tmp/luna-tools-o69fgbqr/result.json`。覆盖准备失败的预算
  结算、值引用写回、目录身份隔离、重入和 Type/非 Type 两条调用路径。

本阶段未执行全 workspace 测试；剩余预算失败如上单独保留，不以这些定向通过
替代完整 CMTT 性能交付。
