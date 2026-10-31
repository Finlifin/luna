# 语义前端开发检查点（转入 LLVM host pipeline 前）

## 计算子句参数的延后函数工厂（2026-10-25）

`ones<width + 1>` 与 `named<.N=increment(width)>` 可以在工厂定义检查时保留源码计算，待具体 comptime 输入供应后形成函数值。预检查仍验证闭合子句域、输入阶段、重复／缺失参数和默认供应；依赖前面输入的子句域保留原路径，未声称实现通用依赖 telescope 的延后。实际实例化继续检查完整应用要求和 callable 签名。

VM 性能重构前的验证：在 `/home/fin/luna` 运行 `python3 -B development/check.py --timeout 900 -- test --release -p luna --test pure_bitvec --test open_functions --test default_calls --test type_lists --test function_type_dependencies --no-fail-fast`，退出 0，524 项通过，`/tmp/luna-tools-lmjws74g/result.json`。staged-bitvec/list-clauses 的 O0/O3 四次原生执行均通过，`/tmp/luna-host-examples-p2i31i8a/summary.json`。这组记录不替代后续 VM 改动的验证；后续性能工作与 definition builder 缺口见 [comptime-vm.md](comptime-vm.md)。

## 普通 comptime 输入驱动函数工厂（2026-10-25）

声明值工厂之外，`factory(comptime width:Integer)->defer (fn()->BitVec<width>) {ones<width>}` 曾失败于 required invocation，基线 `/tmp/luna-run-host-5kbr77u0/summary.json`。等待判断原先只识别 Rigid 值参数；现同时检查带作用域的 substitution_parameters 中的 ComptimeValue 槽位，继续沿用实际签名依赖检查，不把槽位当成完整值。

回归覆盖普通参数转发、零宽／1024 位、闭包捕获、可选默认参数、显式覆盖失败供应、负宽度／错误域／不匹配结果拒绝，以及 runtime 条件不能作为 comptime 宽度。工作目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test pure_bitvec --test open_functions --test default_calls --test type_lists --no-fail-fast`：退出 0，28 + 389 + 43 + 26 = 486 项通过，`/tmp/luna-tools-ago0xws2/result.json`。
- `python3 -B development/host_examples.py staged-bitvec --release-driver --compare-optimization --timeout 900`：退出 0，O0/O3 均输出 `(bool true)`，`/tmp/luna-host-examples-s1i3_h5i/summary.json`。
- workspace 格式检查退出 0，`/tmp/luna-tools-3_g9fd5t/result.json`；git diff --check 通过。

没有放宽普通 callable 的 comptime 形参协议：这里在调用工厂时先供应这些输入，返回的目标函数只含 runtime 形参。通用符号宽度及完整 sysroot 仍待继续。改动留在 dev，未提交。

## 抽象声明值驱动的函数工厂（2026-10-25）

已解除 deferred-bitvec-factory 复现。describe_pending_function_value 在普通定义检查中识别显式开放函数值应用，检查原声明、函数值参数协议、子句域与应用要求；仅当签名计算仍依赖实参中的抽象值绑定时保留 Any 描述，不形成 callable NIR。具体实例继续走 function_instance／callable_value_view／函数体检查。输入已完成时复用已求得的子句参数，不回退重新执行源码参数求值。

回归覆盖 0／8／1024 位工厂、闭包捕获，以及负宽度、结果域不符、错误子句域、optional 参数协议、普通 comptime 输入、unsafe 擦除和程序前置条件擦除的拒绝。工作目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test pure_bitvec --test function_type_dependencies --test open_functions --test type_lists --test default_calls --no-fail-fast`：退出 0，27 + 37 + 389 + 26 + 43 = 522 项通过，`/tmp/luna-tools-8lyi0l15/result.json`。
- `python3 -B development/host_examples.py staged-bitvec list-clauses --release-driver --compare-optimization --timeout 900`：退出 0，四次原生执行均输出 `(bool true)`，`/tmp/luna-host-examples-lyqszsii/summary.json`。
- workspace 格式检查退出 0，`/tmp/luna-tools-svljlav_/result.json`；git diff --check 通过。

下节记录的抽象声明宽度工厂失败已修复。通用符号 BitVec 类型及剩余调用协议仍待继续；没有将未完成签名变成不透明但可执行的函数类型。改动留在 dev，未提交。

## 延后结构类型中的宽度计算（2026-10-25）

signature_annotation 对显式 defer 的指针、可选、普通／comptime 函数及箭头构造使用 prepared invocation，避免其内部 BitVec 宽度被 type_expr 提前读取。各组成部分仍检查 Type 域，Optional 仍要求构造器绑定；普通 normal-form 头不展开，未标记 defer 的表达式继续即时形成。

回归覆盖不同位宽回调转交、闭包捕获、零宽返回、指针转交、可选载荷与空分支；拒绝回调结果／指针目标宽度不符、负宽度、缺失 Optional 绑定及超宽载荷。工作目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test pure_bitvec --test function_type_dependencies --test open_functions --no-fail-fast`：退出 0，26 + 37 + 389 = 452 项通过，`/tmp/luna-tools-ixxhaheh/result.json`。
- `python3 -B development/host_examples.py staged-bitvec --release-driver --compare-optimization --timeout 900`：退出 0，O0/O3 `(bool true)`，`/tmp/luna-host-examples-qc7groly/summary.json`。
- workspace 格式检查退出 0，`/tmp/luna-tools-466w389b/result.json`；git diff --check 通过。

仍未接通：定义阶段以抽象 N 直接形成 `ones<N>` 函数值，见 `development/repros/deferred-bitvec-factory.fl`，本轮报告 `/tmp/luna-run-host-wf_1j8d7/summary.json`。具体 `ones<65>` 已能原生运行，`/tmp/luna-run-host-qv8ourm6/summary.json`；不能据此把抽象工厂也称为完成。改动留在 dev，未提交。

## 开放结果上下文中的字面量取反（2026-10-25）

已解除 deferred-bitvec-invert 复现：结果描述为 Any、且未选择自定义 Invert 协议时，整数字面量及其连续内建 not 暂不按默认 Integer 固化。定义阶段验证字面量并计入预算，保留原始函数源码；具体实例按实际结果域重新检查。没有新增 Type、伪造宽度或发布占位 NIR，也不允许对未知变量直接取反。无上下文、浮点操作数和自定义协议继续走原检查路径。

回归覆盖 0／1／65／1024 位、连续取反、9 位 not 256 等正常结果，以及超宽字面量、实际结果为 bool、未知变量、浮点和缺少上下文的拒绝；失败后的合法包仍可执行。staged-bitvec 原生例纳入取反场景，原复现文件保留为已修复回归。

工作目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test pure_bitvec --test open_functions --test type_lists --no-fail-fast`：退出 0，24 + 389 + 26 = 439 项通过，`/tmp/luna-tools-cdymyt_s/result.json`。
- `python3 -B development/host_examples.py staged-bitvec --release-driver --compare-optimization --timeout 900`：退出 0，O0/O3 均输出 `(bool true)`，`/tmp/luna-host-examples-h5ijcegs/summary.json`。
- workspace 格式检查退出 0，`/tmp/luna-tools-u599cb21/result.json`；git diff --check 通过。

下节的 not 0 限制已由本节修复；通用符号宽度及未知操作数的定义侧证据仍需继续。改动保留 dev，未提交。

## BitVec 抽象宽度诊断与延后构造（2026-10-25）

库资源协议探测发现 `.BitVec'builtin<N>` 的即时类型形成把 required_value 返回值强行视为 Integer；合法抽象值触发 unreachable，基线 `/tmp/luna-run-host-hjcumcqn/summary.json`。现在返回明确的符号宽度尚不支持诊断。impl 头和 struct 字段的拒绝回归通过，随后具体宽度编译可恢复。

显式 defer 的内建 BitVec 构造此前也被当成稳定类型表达式，过早执行 type_expr。现走已有 prepared invocation 路径，普通不透明 normal-form 头不改变。`where N: Integer` 和普通 `comptime width: Integer` 均可在调用点完成零宽、1 位、65 位及 1024 位类型，拒绝负宽度、错误域和结果宽度不符。

工作目录 `/home/fin/luna`：BitVec 最终测试 23 项通过，`python3 -B development/check.py --timeout 900 -- test --release -p luna --test pure_bitvec`，退出 0，`/tmp/luna-tools-rffpmolh/result.json`。同版生产代码的 open_functions 389 项、type_lists 26 项通过，`/tmp/luna-tools-32zsy0ur/result.json`；该整批退出 101，因为首稿 BitVec 用例涉及下面尚未解决的运算符上下文问题，不计为整批通过。其构建曾遇配额错误，入口清理可重建 target 并自动重试后执行测试。

`python3 -B development/host_examples.py staged-bitvec --release-driver --compare-optimization --timeout 900` 退出 0，O0/O3 输出 `(bool true)`，`/tmp/luna-host-examples-bubftgfv/summary.json`。运行器 9 项通过，`/tmp/luna-tools-1da4wym2/result.json`。

剩余缺口：开放函数体 `not 0` 尚不能等待返回域形成，见 `development/repros/deferred-bitvec-invert.fl` 与 `/tmp/luna-host-examples-qdyqshxz/summary.json`。当前示例用零值验证类型形成，不声称通用符号宽度或所有泛型位运算已完成。完整 sysroot 本轮仍失败于未实现的 String／panic／分配和 I/O hooks，`/tmp/luna-library-wravrt3w/summary.json`。Option 的通用 Clone 还需要非 Copy enum 的指针观察支持，本轮未添加受限的伪通用实现。

最终 workspace 格式检查退出 0，`/tmp/luna-tools-qdqyoa8d/result.json`；git diff --check 通过。改动保留 dev，未提交。

## 声明值捕获的 host 阻塞已修复（2026-10-25）

required invocation 原来只补入转交的 Type 绑定，漏掉替换后来自调用者的值绑定及声明域；补齐后，源码子句读取仍错误地尝试把 AbstractValue 冻结为完整 generator data。现分别保留调用者的域证据，以及 NIR ComptimeValue 的 rigid 声明引用，等待具体实例替换。生成函数继续携带捕获域元数据；不放宽真正的常量冻结规则。

新增 bool／u8 工厂回归验证不同捕获实参的实例身份、状态写入，已有回归检查篡改捕获域被拒绝及合法包恢复。原生 list-clauses 接实际 meta 与 resources 库，状态指针采用配对 Copy／Drop 协议检查。

工作目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test type_lists --test open_functions --test function_type_dependencies --no-fail-fast`：退出 0，26 + 389 + 37 = 452 项通过，`/tmp/luna-tools-c_6uhz32/result.json`。
- `python3 -B development/host_examples.py list-clauses --release-driver --compare-optimization --timeout 900`：退出 0，O0/O3 原生运行均为 `(bool true)`，`/tmp/luna-host-examples-uee6m44i/summary.json`。
- 示例运行器 9 项测试通过，`/tmp/luna-tools-0jm5q2jp/result.json`。
- 最终 workspace 格式检查退出 0，`/tmp/luna-tools-i_zfk7fk/result.json`；git diff --check 通过。

过程证据：`/tmp/luna-tools-eyfavl61/result.json` 显示补域后仍有两个 AbstractValueUnresolved 失败；`/tmp/luna-tools-s1wg1x32/result.json` 是未限定 Value 路径的编译错误，已修正。第一次原生尝试 `/tmp/luna-host-examples-6ms17q9l/summary.json` 因缺配对资源协议而拒绝，接实际库后通过。下节交接时的失败已解除，不代表其余前端缺口完成。dev 未提交。

## 当前交接边界（2026-10-25）

开发重心按作者要求转到 LLVM host 示例交付，前端的未完成项目继续保留，不为后端便利改变语义。当前 dev 有大量未提交实现；本检查点描述工作区，不代表 main 已集成。

最近已验证的前端范围是开放函数值的部分签名上下文推导，证据见下一节。其后工作区还有声明值捕获的改动：open_instance_captures 保留捕获参数的 sort 和声明域，生成函数携带 function_parameters 元数据，并在失败时恢复元数据；对应 type_lists 回归及 host-list-clauses 示例已写入。这部分在切换时尚未完成本轮验证，不能沿用前一轮 493 项通过的结论。

恢复语义前端时，应首先完成声明值捕获的回归与域依赖检查，再继续普通 comptime 输入依赖的函数值、optional／implicit 调用协议、通用属性与 ABI elaboration。程序前置条件尚不能完整附着于普通 callable，当前拒绝擦除；完整 sysroot 的 String、Any、panic、分配与 I/O hooks 仍有缺口。Tuple 的 normal-form 身份与类型列表输入保持既定定义。

host 的当前入口、原生验收与限制集中在 [host-pipeline.md](host-pipeline.md)，不把解释器执行或历史测试结果充作本轮原生执行证据。

交接验证已完成：在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 900 -- test --release -p luna --test type_lists`，退出 101，24 通过、1 失败，报告 `/tmp/luna-tools-jbixamyf/result.json`。失败项为新增的 `open_function_value_factories_capture_declared_values_with_their_domains`，在编译阶段报 `required comptime evaluation failed: interpretation Type`，尚未进入执行和元数据篡改拒绝断言。相同缺口使 host-list-clauses O0 在 elaboration 阶段失败，报告 `/tmp/luna-host-examples-zowjagnm/summary.json`；这不是 LLVM 降码失败。保留实现和失败回归供恢复前端时继续，不将该例列入当前可运行清单。其余三个 host 示例的六次 O0/O3 原生执行均通过。

## 开放函数值的部分签名上下文推导（2026-10-25）

此前 `fn forward(value:T)->defer First<Es>() where T,.Es:List<Type> = [T]` 无法在 fn(bool)->bool 上下文取得函数值，报 declaration is not a supported callable，复现 `/tmp/luna-run-host-38xzrgoh/summary.json`。现在为尚未完整的开放签名建立仅供候选推导使用的形状，记录哪些参数域／结果域已明确。未完成注解用内部 Void 占位，但相应上下文方程被屏蔽；这些占位不发布到真实函数或 NIR。原有候选完成与默认值求解之后，仍走 open_function_value 检查实际完整签名和函数体。

回归覆盖从已知参数域或结果域推导 T，再完成列表默认值；bool／u8 实例和直接回调传参均可运行。参数个数错误、完成后类型不匹配、缺少上下文及企图从 First<Es>() 反向猜测 Es 都拒绝。现有已完成签名的上下文路径不变。

工作目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test type_lists --test function_type_dependencies --test default_calls --test open_functions --no-fail-fast`：退出 0，24 + 37 + 43 + 389 = 493 项通过，`/tmp/luna-tools-enfta8kg/result.json`。
- `python3 -B development/host_examples.py list-clauses --release-driver --compare-optimization --timeout 900`：退出 0，O0/O3 均输出 `(bool true)`，`/tmp/luna-host-examples-qy2tzxt1/summary.json`。
- git diff --check 通过，改动保留 dev 未提交。普通 comptime 输入依赖的函数值、调用协议携带、完整属性／ABI 和 sysroot 仍待继续。
- workspace 格式检查退出 0，`/tmp/luna-tools-9vw5roic/result.json`。

## 默认子句完成开放函数值（2026-10-25）

`let callback:fn(bool)->bool=defaulted` 原先在补齐默认子句前就要求 known_callable_signature，因此含 clause-dependent defer 签名的函数被拒绝（`/tmp/luna-run-host-n0dbn210/summary.json`）。contextual_function_instance_with_fixed_view 现在先执行原有 function_type_plan；若它已经补齐所有输入，再进入函数实例形成。待完成的开放签名使用 open_function_value；原已知签名的接纳路径保持原参数协议处理，随后仍检查 callable view。尚有待推导输入时沿原来的上下文推导路径处理，没有新增默认求值规则或反向猜测列表元素。

新增回归涵盖有／无类型标注、直接传回调、显式覆盖、默认结果与上下文不符、optional 参数协议拒绝，以及失败默认供应在显式覆盖时跳过、在省略时报告 ProgramContractRejected、失败后的合法恢复。host-list-clauses 纳入正常默认／覆盖用例。

工作目录 `/home/fin/luna`：
- 最终版本 `python3 -B development/check.py --timeout 900 -- test --release -p luna --test type_lists --test function_type_dependencies --test default_calls --test open_functions --no-fail-fast` 退出 0，23 + 37 + 43 + 389 = 492 项通过，`/tmp/luna-tools-dwjlu56x/result.json`。第一轮通过记录 `/tmp/luna-tools-1rbgnvje/result.json` 早于最后的已知签名兼容分支调整，以最终重跑为准。
- `python3 -B development/host_examples.py list-clauses --release-driver --compare-optimization --timeout 900` 退出 0，O0/O3 均输出 `(bool true)`，`/tmp/luna-host-examples-avnju72h/summary.json`。
- workspace 格式检查退出 0，`/tmp/luna-tools-pt9qtikk/result.json`；git diff --check 通过。改动留在 dev，未提交。

## 开放函数值的工厂、次数与契约拒绝（2026-10-25）

本轮沿新函数值路径补上行为验收，没有发现需要放宽或修改编译器实现的地方。泛型 factory 返回 identity<[T]>，bool／u8 两个具体实例均保留外层 Type 捕获；callee 表达式与实参分别写入观察位，验证先求 callee、后求实参且各一次。host-list-clauses 已包含此原生场景。

契约验收明确了当前限制：guarded 的独立 bool 参数 gate 带 requires gate，直接 true 调用通过、false 调用拒绝；无论显式标注 fn(bool,bool)->bool 还是让局部变量推断函数类型，取其函数值都报告 erase program preconditions。普通 callable 尚不承载程序前置条件，本项不能称为契约函数值支持；先前“保留契约”包含这一安全拒绝。初始探测报告 `/tmp/luna-run-host-kk6fccsu/summary.json` 保留为拒绝证据。

工作目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test type_lists` 退出 0，21 项通过，`/tmp/luna-tools-a1ao7pox/result.json`。
- `python3 -B development/host_examples.py list-clauses --release-driver --compare-optimization --timeout 900` 退出 0，O0/O3 均输出 `(bool true)`，`/tmp/luna-host-examples-ehgczvb3/summary.json`。
- workspace 格式化退出 0，`/tmp/luna-tools-3lp81fzf/result.json`；git diff --check 通过。dev 未提交。

## 子句已完成的开放函数值（2026-10-25）

`identity<[bool]>` 取函数值原来只查询声明阶段已知签名，遇到 clause-dependent defer 注解报 declaration is not a supported callable（`/tmp/luna-run-host-k19dq46b/summary.json`）。function_instance 现在将这种显式完整实例交给 open_function_value：先检查定义与子句输入，暂时安装 clause substitution，按参数次序完成注解，再复用 instantiate_open_function 生成函数与缓存身份。调用者 substitution 在成功／失败后均恢复；已知签名路径不变，不生成绕过源契约的包装函数。

本路径保持 callable_type 的执行阶段／unsafe 标记、生成函数的程序契约和类型要求，以及已存在的参数协议检查。尚需普通 comptime 输入的开放签名仍明确拒绝函数值形成；没有擦除 optional 等协议。示例中保存 bool／u8 两种实例、传给回调并捕获到闭包，错误类型、unsafe 擦除、optional 擦除及空列表作为拒绝例。

工作目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test type_lists --test function_type_dependencies --test default_calls --test open_functions --no-fail-fast`：退出 0，19 + 37 + 43 + 389 = 488 项通过，`/tmp/luna-tools-7l1ezdog/result.json`。
- `python3 -B development/host_examples.py list-clauses --release-driver --compare-optimization --timeout 900`：退出 0，实际 library/meta 示例在 O0/O3 均输出 `(bool true)`，`/tmp/luna-host-examples-ki3sszbh/summary.json`。
- git diff --check 通过。改动留在 dev，未提交；完整 ABI 属性、sysroot 和剩余函数值参数协议仍待继续。
- workspace 格式检查退出 0，`/tmp/luna-tools-k5xx4j33/result.json`。

## 类型列表子句驱动 runtime 延后签名（2026-10-25）

`comptime fn Forward()->Type where Es:List<Type> {First<Es>()}` 转交已可执行；原缺口在 `fn identity(value:defer First<Es>())->defer First<Es>() where Es:List<Type>` 的签名准备。旧逻辑只检查 PreparedRequired 的普通输入是否为空，误将有符号身份但尚无列表内容的子句当成可立即执行的输入。signature_annotation 现在取得当前可见值子句，复用有预算的 open_computation_uses_clauses 检查实际依赖，再保留 Pending 计算。调用点沿现有 clause substitution／Suspended 路径重新形成签名。仅有未使用的作用域参数不导致延期；普通非 defer 注解的即时计算规则未变。

工作目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test type_lists --test open_functions --test function_type_dependencies --no-fail-fast`：退出 0，18 + 389 + 37 = 444 项通过，`/tmp/luna-tools-iebjxp9n/result.json`。新回归覆盖 bool／u8 实例、错误实参、空列表、失败后的合法编译，以及无关值子句不妨碍 pure 函数的常量签名。
- `python3 -B development/host_examples.py list-clauses staged-default-forwarding --release-driver --compare-optimization --timeout 900`：退出 0，四次原生运行输出 `(bool true)`，`/tmp/luna-host-examples-__346oiu/summary.json`。list-clauses 使用实际 library/meta。
- 初始显式 defer 失败 `/tmp/luna-run-host-j2ebo636/summary.json`；第一版仅依赖捕获准备记录，新增测试仍失败，`/tmp/luna-tools-tmsriqxp/result.json`，不计为整批通过。最终改从声明域取得可见值参数后通过。dev 工作区未提交。
- workspace 格式检查退出 0，`/tmp/luna-tools-06n52j54/result.json`；git diff --check 通过。

## 暂停前端扩展，优先交付 host 示例（2026-10-25）

按作者要求，将开发重心切到 LLVM host pipeline。保留 dev 的现有实现与测试，不回滚未提交工作，也不把当前前端接受范围当作完整语言语义。已经可以运行的入口和本次原生验收见 [host pipeline](host-pipeline.md)。

恢复前端时优先核对以下已知缺口：

- Tuple 已保留唯一类型列表输入与 normal-form 头，并接通整包实现推导及 builtin Once；通用逐元素 Clone／Copy 尚未完成。
- 普通函数值尚不能完整携带 optional／implicit 等参数协议；不能靠删除接纳检查来擦除这些协议。
- 条件 const 默认值和无回边循环的有限性已接通；待形成签名的开放函数值、一般循环终止证明仍待补齐。
- 普通 comptime 函数的 `where Es: meta.List<Type>` 域接纳已在后续 host staging 验收中修复，见下节；先前声明拒绝报告 `/tmp/luna-run-host-_yq4qls2/summary.json` 保留为历史失败。
- 完整 sysroot 的 String、Any、panic、分配／I/O hooks，以及一般函数 ABI 属性 elaboration 尚未接通。小型包与显式协议绑定的原生示例通过，不等于完整库通过。

以下历史条目保留各项实现、验证范围和失败证据。后端不改变 normal-form 身份、Tuple 定义，也不将程序规格并入 PTC。

## 函数类型列表子句与 host staging（2026-10-25）

声明检查现在接纳已绑定的 meta.List<Type> 位置子句和 optional 子句。解释器 validate_function_inputs 和 NIR callable 输入验证分别检查列表域的受信构造器身份、Type 元素及闭合作用域；NIR 元数据查找还拒绝重复头，检查工作计入预算。完整输入向量先验 sort／scope，再替换依赖域，保持捕获参数的原有规则。没有展开 normal-form 头或把程序谓词交给 PTC。

第一轮只修声明检查，随后在 required invocation 报 Type，证据 `/tmp/luna-run-host-m2qxxby8/summary.json`；最终补齐两条输入验证路径。工作目录 `/home/fin/luna`：

- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test type_lists --test function_type_dependencies -p nir --test function_parameters --no-fail-fast`：退出 0，57 项通过，`/tmp/luna-tools-2fc4e12x/result.json`。包含默认列表、显式覆盖、越界、错误元素／域、缺失／歧义元数据和预算失败后恢复。
- `python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --test type_list_inputs --test runtime_function_identities --no-fail-fast`：退出 0，23 项通过，`/tmp/luna-tools-et0y65pi/result.json`。
- `python3 -B development/host_examples.py list-clauses --release-driver --compare-optimization --timeout 900`：真实 library/meta 模块在 O0/O3 均输出 `(bool true)`，`/tmp/luna-host-examples-lsugnxxd/summary.json`。源码 `development/examples/host-list-clauses.fl`。
- 运行器 9 项通过，`/tmp/luna-tools-xlr78al2/result.json`；workspace 格式检查退出 0，`/tmp/luna-tools-a1llpvwn/result.json`；git diff --check 通过。改动留在 dev，未提交。

函数 ABI 属性仍依赖通用 Object 属性求值，未按 attrs.abi 的拼写建立特殊后门。完整 sysroot 的缺口仍在。

## Tuple 整包实现参数推导、列表域证据与 builtin Once（2026-10-25）

为 Tuple 实现使用 `where Es: meta.List<Type>` 时，旧匹配器只拆两份已展开的 TypeListValue，无法为整个 Es 赋候选。matching::decompose 现在沿已绑定 Tuple 的唯一输入继续匹配，可推导整个类型列表，或逐项匹配两个列表；最终仍保留原始类型等式义务。普通不透明头不取得反向推导能力，候选提交前不污染外层推断状态。

随后暴露实现实参准入缺少 TypeList 表示证据。validate_head_arguments_with_domains 按实际 List<Type> 域查找唯一、形状匹配的 TypeList role 元数据，带预算检查列表元素的 Type sort 和作用域，不展开头。实现索引保存 normal-form 元数据，变化时递增 revision；更新名义布局保留这些元数据。source trait/inherent 世界、required invocation、materialization、purity 和驱动执行世界交付同一信息；索引 Display 纳入既有 S-expression dump。匹配、重放、撤销／恢复与无关布局更新均有回归。

库新增 builtin/tuple_resources.fl，在资源 trait 所属的 builtin 包中为 `Tuple<...Es>` 提供 Once。constructor 文件继续可独立加载，不自动引入资源 trait，也没有授予 Clone／Copy 或自定义 Drop。跨独立包放置该实现会被现行归属规则正确拒绝，故没有修改 ownership 规则；原生 Once 测试将实际 Tuple、meta、resources 与扩展模块装入同一个定义包。样例 development/examples/host-library-tuple-once.fl 验证空元组、单元素求值一次、嵌套仿射字段交付及显式清理顺序。

工作目录 /home/fin/luna，dev 基线 944e40ce，改动未提交。最终验证：
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test library_protocols --no-fail-fast` 最终退出 0，17 项通过，包括扩展模块独立子作用域下的新样例 O0/O3 原生执行，`/tmp/luna-tools-h8uy6r8k/result.json`。缓存恢复与布局保留断言也已包括在内；此前定向报告 `/tmp/luna-tools-cwmz75u5/result.json` 同样退出 0。
- 同入口 `test --release -p elaboration --test matching --test implementation --test inherent --test calls --no-fail-fast` 退出 0，共 134 项（19 + 68 + 6 + 41），`/tmp/luna-tools-rshbac1w/result.json`。合计 151 项不同 Rust 测试通过。
- `python3 -B development/host_examples.py library-tuple quote-contract-patterns --release-driver --compare-optimization --timeout 900` 退出 0，两例 O0/O3 均为 (bool true)，`/tmp/luna-host-examples-mofn4g55/summary.json`。
- `python3 -B development/library_check.py --release --timeout 900 --stage native-tuple` 退出 0，四项实际库源码原生测试通过，scope=selected，`/tmp/luna-library-nyh818tc/summary.json`。不代表完整 sysroot 已可用。
- 运行器测试 9 项通过，`/tmp/luna-tools-m70pj5bp/result.json`；全 workspace 格式检查退出 0，`/tmp/luna-tools-no52lp5f/result.json`；git diff --check 通过。

失败证据保留：源码整包推导失败 `/tmp/luna-run-host-48_5vh6w/summary.json`；域证据失败 `/tmp/luna-tools-b63fqpoq/result.json`；拆包归属失败 `/tmp/luna-host-examples-8azzz43j/summary.json` 与 `/tmp/luna-library-gow_caoz/summary.json`。后者清单阶段成功但整体失败，没有记为通过。通用逐元素 Clone／Copy、完整 sysroot 以及任意列表形状推导仍待继续。

## 条件 const 默认值与无回边循环的有限性（2026-10-25）

本轮先检查函数值参数协议：arguments::require_function_value_parameters 的拒绝有实际依据，当前普通 callable type 没有承载 optional／implicit 等路由元数据；没有删除此检查或把参数协议悄悄擦除。该工作仍待类型表示与消费者共同接入。随后修复默认值中的具体阶段遗漏：has_staged_body_work 对 const 同时扫描 initializer、guard 和模式内表达式，避免只有 guard／等值模式依赖 comptime 参数时提前执行选择。

ordinary_conditional_constant_binding 只构造定义侧行为证据。initializer、模式和 guard 分别验证 staged 可用性；成功导出的值与选择条件使用既有 ordinary_behavior_inputs，已知 Type 结构证据单独保留。else 在安装候选绑定前检查，保持外层 runtime 作用域与发散要求。实际调用仍使用 conditional_local_data_binding 完成模式、guard 和常量物化，不发射行为证据。模式可用性复用 check_inline_pattern_availability，保留等值匹配内部的候选局部，避免把观察表达式单独拆出后误报 unstaged local。没有新增 IR 种类。

P3 有限检查新增无回边循环依据：主体类型 Never 排除落穿，带预算的 loop_may_continue 排除指向本循环的 continue，随后仍递归检查所有操作和调用。只在该分析作用域允许对应 break；不解除递归、未知调用、外部副作用或无限子循环的拒绝。普通 P4 循环分析不变。

工作目录 /home/fin/luna，dev 基线 944e40ce，改动保留工作区：
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test default_calls --no-fail-fast` 退出 0，43 项通过，`/tmp/luna-tools-cv9w29pm/result.json`。新增用例覆盖 guard／initializer／等值模式的 staged 依赖、候选绑定、Type 导出、comptime guard 调用、未调用定义，以及 runtime guard、错误作用域、非发散 else、回边、落穿、无限子循环与递归调用拒绝。
- `python3 -B development/host_examples.py staged-default-forwarding --release-driver --compare-optimization --timeout 900` 退出 0，实际 resources 协议下 guard 与等值模式的两种选择均被检查，O0/O3 输出 (bool true)，`/tmp/luna-host-examples-c927twir/summary.json`。
- 同轮 open_functions 389 项、pure_callbacks 80 项在加入有限循环后通过，`/tmp/luna-tools-w4weiwmw/result.json`；该联合运行因候选局部问题整体退出 101，不计作整批成功。之后只调整条件 const 的模式检查并运行上述最终回归。
- workspace 格式检查退出 0，`/tmp/luna-tools-xv5elp7y/result.json`；git diff --check 通过。按授权清理 luna release 可重建产物，未提交。

原复现保存在 development/repros/default-conditional-constant.fl。初始提前求值失败报告 `/tmp/luna-run-host-6onnlq5a/summary.json`；首稿遇到循环有限性拒绝 `/tmp/luna-tools-oj7g0689/result.json`；扩大模式用例后发现候选局部丢失 `/tmp/luna-tools-w4weiwmw/result.json`。这些失败均保留，不替换为最终成功证据。一般带回边循环的终止证明和函数值参数协议仍未完成。

## 带子句的开放函数常量与 host 验收（2026-10-25）

在 dev（基线 944e40ce，保留工作区改动）补齐已知签名开放函数的显式类型子句、函数类型上下文推断及值子句实例化。function_instance 和 contextual_function_instance 共用已知 callable 签名；finish_call 在开放调用期间安装并恢复子句 substitution。required invocation 的参数域元数据加入开放函数，避免把 bool 值子句误按 Type 检查。身份元数据不等于可执行主体，runtime 输入仍不能供应 comptime 形参。

default_calls 回归覆盖同一声明的 bool/Byte 两种实例、上下文推断、值子句，以及错误参数类型、阶段和值域拒绝。Byte 在夹具中显式声明为 .u8'builtin；首轮误用未声明的 u8 导致 HIR 拒绝，报告保留为失败，不计为通过。前端后续继续处理条件 const、待形成签名的开放函数值和一般参数协议；本项不代表这些能力完成。

工作目录 /home/fin/luna，验证记录：
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test default_calls --test open_functions --test pure_callbacks --no-fail-fast` 退出 0，报告 `/tmp/luna-tools-ism86upz/result.json`。
- 此前同一编译器实现下 function_type_dependencies 与 extended_calls 各 37 项通过；联合运行因上述夹具问题退出 101，报告 `/tmp/luna-tools-c794f00j/result.json`，不将整批记为成功。
- `python3 -B development/host_examples.py arithmetic staged-default-forwarding --release-driver --compare-optimization --timeout 900` 退出 0：算术为 (unsigned-integer 42)，泛型／staged 默认值示例为 (bool true)，O0/O3 均实际编译、链接、执行通过；报告 `/tmp/luna-host-examples-ephxttn6/summary.json`。
- workspace 格式检查退出 0，`/tmp/luna-tools-8g7gyg0d/result.json`；git diff --check 通过。改动未提交。

当前 host 入口可运行上述示例，结果打印由开发驱动适配器提供；完整 sysroot 的 String、panic、I/O 接线及通用函数 ABI 属性 elaboration 仍待完善，不能据此宣称 println 或完整语言已实现。

## 已知签名的开放函数 const 身份与调用实例（2026-10-25）

修复 default-open-function-constant.fl：签名已明确、无需子句实例化的开放函数可保存在 const 中，再按普通 comptime 实参形成具体调用。known_callable_signature 同时查询普通签名和开放定义的已知 telescope；定义侧 const 保留函数声明身份，required invocation 快照与最终 NIR 交付相应 runtime identity 元数据。局部 staged 绑定在描述阶段参加调用输入的延后判定，实际调用仍要求具体值。finish_call 将 Function 以及物化所得的空实参 FunctionInstance 路由回开放函数实例化；临时行为签名阻止分析调用重复进入该路径。没有发布未形成的可执行主体，也没有补上尚未支持的 optional 函数值协议。

IIR TemplateStorage 允许从唯一的 runtime identity 元数据验证签名与子句实参，保持逻辑函数类型和标签表示分离。身份仅支持标签和阶段检查，不能执行缺失主体。新增测试覆盖标签准入、缺失主体执行拒绝、身份歧义、签名不匹配与恢复。default_calls 覆盖两种 staged 输入、未调用定义、runtime 输入、非法／递归分支及 optional 函数值拒绝。原复现改为正向回归，host-staged-default-forwarding 加入函数 const。

工作目录 /home/fin/luna，最终证据均退出 0：
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test default_calls --test function_type_dependencies --test extended_calls --no-fail-fast`：40 + 37 + 37 项通过，`/tmp/luna-tools-lhlhqvil/result.json`。
- 同入口 `--test open_functions --test pure_callbacks --no-fail-fast`：两组通过，`/tmp/luna-tools-8y2dkmk7/result.json`。
- `python3 -B development/host_examples.py staged-default-forwarding --release-driver --compare-optimization --timeout 900`：实际 resources 协议下 O0/O3 均为 (bool true)，`/tmp/luna-host-examples-49f24q47/summary.json`。
- `python3 -B development/run_host.py development/repros/default-open-function-constant.fl --release-driver --build-timeout 900`：原复现原生输出 (bool true)，`/tmp/luna-run-host-uv_sq2oo/summary.json`。
- workspace 格式检查通过，`/tmp/luna-tools-zw64lbfb/result.json`；git diff --check 通过。

修复过程依次暴露局部 staged 输入过早求值、函数阶段元数据缺失、template storage 仅查主体、空 FunctionInstance 未路由的问题；最后一项通过 NIR dump 确认，生成报告 `/tmp/luna-tools-zdppu3zu/result.json`。此前失败报告 eff8fx1w、by0xca5d、ws1i8fzg、gdslvrf6、75gn9hkz 均保留，不计为成功。元数据交付首稿出现部分移动的 Rust 错误（jwrh5xju），已改在 checker 状态移交前收集。构建按授权清理可重建缓存；条件 const、待形成签名及带子句实例化的开放函数值仍待继续，dev 未提交。

## 无条件数据 const 的默认行为证据（2026-10-25）

修复下节保存的 default-local-comptime-binding.fl。checking_open_behavior 下，无条件数据 const 通过 ordinary_constant_binding 保留已检查的表达式，不进入 required invocation／常量物化。表达式仍以 staged 阶段检查；普通绑定保留原表达式，解构导出复用 LetPattern 和局部结果的检查用块，已知 Type 信息优先保留。表达式与 pattern 复制都计入预算。实际调用继续执行原始 const 路径，没有把分析表达式发射为 runtime 常量初始化，也没有统一把 let 提升为 comptime。quoted code 仍进入原有独立分支。

新增回归覆盖原复现、无调用点、连续 const、tuple 解构、Type 常量、普通函数声明常量，以及 runtime 输入和非法分支拒绝。工作目录 /home/fin/luna：default_calls 最终 39 项通过（`python3 -B development/check.py --timeout 900 -- test --release -p luna --test default_calls`，`/tmp/luna-tools-5n4ciu33/result.json`）；此前同一最终编译器实现下 inline_control 249、pure_callbacks 80 项通过（`/tmp/luna-tools-mxinnlqm/result.json`，该批整体退出 101 的原因是下述开放函数值用例，不将整批报告为成功）。共 368 项不同测试最终通过。

`python3 -B development/host_examples.py staged-default-forwarding --release-driver --compare-optimization --timeout 900` 退出 0，实际 resources 协议下 O0/O3 均为 (bool true)，`/tmp/luna-host-examples-laa4dkr2/summary.json`。`run_host.py development/repros/default-local-comptime-binding.fl --release-driver --build-timeout 900` 退出 0，输出 (bool true)，`/tmp/luna-run-host-awkvqktd/summary.json`；该文件改为正向回归注释。

新增身份测试发现 const 保存尚未实例化的开放函数声明仍失败。独立保存 `development/repros/default-open-function-constant.fl`，当前原生构建退出 1，诊断 declaration is not a supported callable，`/tmp/luna-run-host-axvgw6o7/summary.json`。改用已有支持的普通函数声明后，常量身份与后续调用测试通过。带 guard／else 的条件 const 仍未接入行为描述，这两项均未宣称完成。参数文档同步边界；dev 未提交。

## 下一处默认值缺口：局部 const 的行为描述（2026-10-25）

新增 `development/repros/default-local-comptime-binding.fl`：默认块先 `const selected = not mode`，再调用 `invert(selected, value)`；两种 mode 的预期合并结果为 true。工作目录 /home/fin/luna，`python3 -B development/run_host.py development/repros/default-local-comptime-binding.fl --release-driver --build-timeout 900` 退出 1，`/tmp/luna-run-host-04xsmdn4/summary.json`，诊断 required comptime input (local 2 1) is not available in this invocation。此项尚未修复，不把前一轮直接实参计算的成功外推到先绑定局部 const。

已定位：check_open_definition 先 describe_open_expression 检查默认块，随后 checking_open_behavior 下再次 expr(default)。H::Let 对 runtime 正文内的 const 调用 cmtt::code_declaration，普通数据分支进入 local_data_binding 并实际求值；该路径尚未区分行为证据与具体调用的常量物化。修复需要保留常量的定义侧类型／阶段依据，并在实际调用才物化，不能通过把普通 runtime let 统一标为 staged 解决。还须处理 Type 身份、代码值、模式／guard 和 callable 行为证据，不能一律替换成无来源的任意值。

对照：将 const 写成普通 `let selected = not mode`，当前在定义描述中报 unstaged local（`/tmp/luna-run-host-ptjmhxn7/summary.json`）；没有据此改变普通局部变量阶段规则。改用显式 const 的临时输入仍在物化时失败（`/tmp/luna-run-host-zc20v4ir/summary.json`）。本轮保留最小源码与新证据，未修改编译器；dev 未提交。

## 默认值中 staged 实参计算的依赖证据（2026-10-25）

复现 `invert(not mode, value)`：mode 是前序 comptime bool，阶段检查接受，但默认值 P3 检查只接受直接 Local 或已规范化常量，因而拒绝（`/tmp/luna-run-host-i8kb7wet/summary.json`）。purity::Analysis 现在复用带预算的 lexical free-binding 分析：非空自由绑定集全部来自已确认 staged 参数时，可保留定义侧可用性依据。该依据不执行表达式、不省略后续 expression/call_candidate 分析；运算／调用仍须稳定且有限，实际 stage 与输入供应仍由阶段检查负责。局部绑定按词法作用域处理，不能用一个 runtime 局部伪装 staged 输入。

新增 default_calls 测试覆盖 not、比较、块内局部绑定，两种 mode、无调用点、runtime 参数拒绝、非法／递归供应分支拒绝。原生示例增加 computed 默认供应，参数文档更新不再宣称仅支持直接引用。工作目录 /home/fin/luna，全部退出 0：
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test default_calls --test pure_callbacks --no-fail-fast`：38 + 80 项通过（`/tmp/luna-tools-y9q3loi5/result.json`）。
- 同入口 `--test extended_calls --test staged_aggregate_contracts --no-fail-fast` 通过（`/tmp/luna-tools-pn09vmia/result.json`）。
- `python3 -B development/host_examples.py staged-default-forwarding --release-driver --compare-optimization --timeout 900`：实际 resources 协议下 O0/O3 均输出 (bool true)，`/tmp/luna-host-examples-68cjy5od/summary.json`。
- workspace 格式检查通过（`/tmp/luna-tools-561ooaya/result.json`），git diff --check 通过。

同时复核上一轮解构转换用于普通函数主体，原生输出 (bool true)（`/tmp/luna-run-host-z8qjesud/summary.json`）。构建前按授权清理 luna release 缓存；未清理源码／历史证据。dev 未提交。

## 解构默认值中的 inline 转换与其余 pattern 入口（2026-10-25）

复现：runtime 默认表达式中的 `let (first and (inline if mode { not first } else { first }) is result) = value`，mode 是前序 comptime 参数。原生构建在定义侧报 required comptime input unavailable（`/tmp/luna-run-host-ywix66fu/summary.json`）。补齐 scan_body_requirement 对 let、for、ErrorHandle pattern 的遍历；match 与 EffectHandle 沿用上轮接线。只补扫描仍失败（`/tmp/luna-tools-5vazt38x/result.json`）：已知类型的解构回到普通检查器后仍试图执行 inline 选择器。describe_known_region 现在为该检查动态安装并恢复既有 contextual inline 描述入口，复用选择器阶段检查与全部分支检查，调用实例仍重新处理原源码。

default_calls 增加解构转换正例，并为全部 inline 默认写法增加去掉 comptime 标记后的拒绝例；未调用定义与非法分支检查保留。host-staged-default-forwarding 增加两种 mode 的解构转换；参数文档补相同语义示例。工作目录 /home/fin/luna，最终证据：
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test default_calls --test inline_control --test extended_calls --no-fail-fast` 退出 0，37 + 249 + 37 项通过（`/tmp/luna-tools-q_buor5a/result.json`）。
- 同入口 `--test pure_callbacks --test error_surface --no-fail-fast` 退出 0，80 + 118 项通过（`/tmp/luna-tools-0d2lh8sn/result.json`）。
- 同入口 `--test open_functions` 退出 0（`/tmp/luna-tools-xcnbo6xi/result.json`），核验通用定义描述路径的既有行为。
- `python3 -B development/host_examples.py staged-default-forwarding --release-driver --compare-optimization --timeout 900` 退出 0，实际 resources 协议下 O0/O3 均为 (bool true)（`/tmp/luna-host-examples-ubz0889o/summary.json`）。
- 全 workspace 格式检查退出 0（`/tmp/luna-tools-0s3yxzt7/result.json`），git diff --check 通过。

构建前按授权清理 luna release 缓存，不清理源码或历史证据。此项修复不等于完成所有开放方法、完整 sysroot 或全部模式协议；dev 未提交。

## inline 默认匹配保留 pattern 中的 staged 依赖（2026-10-25）

发现并修复常量 subject 掩盖 pattern 依赖的缺口：`inline if true is do { 'mode => not value, _ => value }` 中 mode 为前序 comptime 参数，原实现只扫描 subject／arm guard，在定义侧过早索取 mode，复现退出 1（`/tmp/luna-run-host-htd71a1k/summary.json`）。新增带预算的迭代 pattern 表达式遍历，覆盖常量／comptime／splice、嵌套 guard／转换、范围端点、normal-form 与 BitVec 表达式；inline 默认选择器分类和 match／effect handler 的主体依赖扫描共用它。定义侧保留行为检查，调用侧继续取得实际 staged 值，没有提前执行 runtime 分支输入。

default_calls 增加常量引用 pattern、computed pattern、局部别名选择器组合；保留两种 mode、未调用定义、非法分支拒绝。host-staged-default-forwarding 增加 pattern 依赖，参数文档同步示例。工作目录 /home/fin/luna：default_calls 37 + extended_calls 37 通过（`python3 -B development/check.py --timeout 900 -- test --release -p luna --test default_calls --test extended_calls --no-fail-fast`，`/tmp/luna-tools-yk6v48ik/result.json`）；inline_control 与 pure_callbacks 两组通过（同入口 `--test inline_control --test pure_callbacks --no-fail-fast`，`/tmp/luna-tools-fel680qp/result.json`）。原生示例 O0/O3 均输出 (bool true)，`python3 -B development/host_examples.py staged-default-forwarding --release-driver --compare-optimization --timeout 900`，`/tmp/luna-host-examples-3huwi6d5/summary.json`，以上退出均为 0。

首稿 Rust 生命周期不够一般，改为分别约束借用期与 HIR 生命周期；computed pattern 测试首稿缺少 `<` 前的分隔空白，被现有词法规则拒绝，修正用例后通过。未修改词法语义。没有据此宣称所有 pattern 位置或开放方法均已完成；dev 未提交。

## Host 可运行交付与前端续接状态（2026-10-25）

当前工作区已具备源码 → 具体 IIR → LLVM IR 校验／优化 → host 对象 → 链接执行路径。本次用当前源码构建 release driver，算术、数论循环／递归、staged 默认供应三个示例均通过 O0/O3 原生执行：`python3 -B development/host_examples.py arithmetic number-theory staged-default-forwarding --release-driver --compare-optimization --timeout 900`，工作目录 /home/fin/luna，退出 0，报告 `/tmp/luna-host-examples-urd0uy4p/summary.json`。输出分别为 (unsigned-integer 42)、(unsigned-integer 1121)、(bool true)。

前端收尾：runtime 默认表达式中的 inline if／when／match 可保留前序 comptime 参数依赖，在定义侧检查行为、调用侧供应选择器；常量选择器不因出现 inline 就强迫方法进入开放签名路径。default_calls 37 项、extended_calls 37 项通过，报告 `/tmp/luna-tools-g2xuzprq/result.json`；格式化报告 `/tmp/luna-tools-_s3po48v/result.json`。host-staged-default-forwarding.fl 补入直接 inline 默认值、两种选择器与显式覆盖，实际 resources 协议下 O0/O3 均通过，报告 `/tmp/luna-host-examples-3prg7r36/summary.json`。

后续前端继续补齐开放方法／待形成域的组合，以及完整 sysroot 所需的 panic、String、Any、分配与 I/O 接线；已有局部验收不能当作完整 sysroot 通过。结果打印目前来自开发驱动适配器，不代表标准库 println 已就绪。此前的细分检查点保留在下文，旧失败以较新的对应修复记录为准。开发仍在 dev，保留未提交改动。

## 默认值调用含 inline 分支的供应函数已接通（2026-10-25）

上一节保存的 `development/repros/default-forwarded-comptime-input.fl` 已通过原生执行。开放函数依赖传播现在同时扫描 runtime 主体和默认表达式，因而调用含 inline if 供应函数的默认值会进入正确的实例准备路径。定义侧只描述调用并按需生成供应函数的 ordinary behavior witness，覆盖所有 inline 分支，不提前取得某个调用的 comptime 输入；调用侧继续按本次输入形成真正的实例。原 ordinary_pure_functions 更名为 ordinary_behavior_functions，新增 witness 保留原 pure/unsafe 标记。P3 检查临时使用这些描述，随后从可执行函数列表中移除；后续 P4 检查仍使用它们，不把分析节点交给后端。

未调用定义的测试发现阶段依据曾依赖已生成实例。现在每项 pending default stability obligation 携带定义作用域中可见的前序 comptime 绑定，逐项安装并撤销；资源观测分析的独立快照也保留该依据。依据来自已检查声明的 comptime 标记，而非签名构造结束后已释放的 signature_domains 临时表。未被调用的合法默认定义不再误报 P3；显式覆盖不隐藏非法／递归供应分支，runtime 参数仍不能冒充 comptime 输入。原复现保留为正向回归；参数文档已更新。此轮覆盖参数和结果域已明确的开放供应函数，不据此宣称所有待形成域组合均已完成。

工作目录 /home/fin/luna，最终验证：
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test default_calls --test extended_calls --no-fail-fast`：35 + 37 项通过，退出 0（`/tmp/luna-tools-j_7gs26n/result.json`）。新用例包括原复现、无调用点、实际资源协议下无调用点、显式覆盖后的非法／递归分支和 runtime 输入拒绝。
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test pure_callbacks`：80 项通过（`/tmp/luna-tools-vu8j2tx9/result.json`）；`--test inline_control pure_open`：8 项通过（`/tmp/luna-tools-o3lca1qa/result.json`）。共 160 项相关 Rust 测试通过。
- `python3 -B development/host_examples.py staged-default-forwarding --release-driver --compare-optimization --timeout 900`：O0/O3 均输出 (bool true)，默认预算、实际 resources 协议（`/tmp/luna-host-examples-aah107j2/summary.json`），覆盖 inline 供应函数、两种 mode、再次转调、链式默认、显式覆盖、一次 runtime 实参求值。
- `python3 -B development/run_host.py development/repros/default-forwarded-comptime-input.fl --release-driver --build-timeout 900`：退出 0，原始复现输出 (bool true)，`/tmp/luna-run-host-vx3u1smf/summary.json`。无调用点版本也退出 0（`/tmp/luna-run-host-0q_rrt26/summary.json`）。
- 示例运行器测试通过（`/tmp/luna-tools-uqltjj8i/result.json`）；workspace 格式化退出 0（`/tmp/luna-tools-gpfcaja6/result.json`），git diff --check 通过。

初次正例通过后，新增无调用点测试如实暴露阶段依据遗漏；使用临时签名表的首稿也被该测试拒绝，改为声明依据后最终全部通过。重编译前按授权清理 luna release 可重建缓存，没有清理源码或旧证据；本轮无配额失败。dev 未提交。

## 普通 runtime 默认值转发声明中的 comptime 参数（2026-10-25）

完成上一节缺口中的普通调用路径。P3 分析按已检查函数签名收集 staged 形参身份，接受这类形参的直接引用，不再把“定义侧不知道具体值”误当作缺少稳定性证据；普通 runtime 局部仍不能冒充 staged 输入，被调函数仍须所有可能分支满足有限性／副作用要求。首次改动让前端通过后，解释执行暴露 IIR 的下一处遗漏：runtime 默认实参只重建了 staged let，尚未将已完成值传给内部 staged 调用。现在在该默认值的 callee preparation frame 内复用已有闭合局部转发与写入／遮蔽检查，再重建仍有 runtime 用途的绑定；保持源码实参顺序和独立调用实例。

新增 default_calls 正例与 runtime 输入／递归分支拒绝例；新增 `development/examples/host-staged-default-forwarding.fl`，接实际 resources 协议，覆盖不同 staged 值、再次转调、链式默认、显式覆盖和 runtime 实参一次求值，目录增至 63 项。functions/parameters.md 同步说明已实现范围。原 `development/repros/default-forwarded-comptime-input.fl` 中依赖 inline if 的开放主体仍在定义侧过早要求值，未宣称修复：最新重现退出 1，`/tmp/luna-run-host-ibvxmgtr/summary.json`。后续需要把此类默认的行为描述与调用实例准备分开，不能删除定义侧检查或把未知输入伪造为常量。

工作目录 /home/fin/luna：
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test default_calls --test extended_calls --test staged_aggregate_contracts --no-fail-fast` 退出 0，34 + 37 + 31 项通过（`/tmp/luna-tools-3hvwqnl6/result.json`）。
- `python3 -B development/check.py --timeout 900 -- test --release -p iir --test staged_preparation --test dependent_staging` 退出 0，9 项通过（`/tmp/luna-tools-yuw2nk2t/result.json`），含依赖默认值与类型替换。
- `python3 -B development/host_examples.py staged-default-forwarding --release-driver --compare-optimization --timeout 900` 退出 0，O0/O3 原生执行均为 (bool true)，默认预算（`/tmp/luna-host-examples-ebqg02wr/summary.json`）。示例运行器测试通过（`/tmp/luna-tools-qztaprkh/result.json`）。格式化退出 0（`/tmp/luna-tools-j5326_um/result.json`），git diff --check 通过。

首次 Rust 编译因 LocalId 不实现 Hash 改用 BTreeSet；随后定向测试如实暴露 IIR 缺口，补齐后通过。本轮重编译前按授权清理 luna release 缓存 208.3 MiB，后续无配额失败。dev 未提交。

## 默认值转发前序 comptime 输入的未完成路径（2026-10-25）

新增可复现输入 `development/repros/default-forwarded-comptime-input.fl`。take 的 runtime 默认值调用 invert(mode, value)，mode 是 take 的前序 comptime bool 参数；两次调用分别供应 true/false，预期 run 为 true。当前在默认定义准备阶段失败，尚未进入 LLVM：required comptime input (local 2 1) is not available in this invocation。真实构建证据 `/tmp/luna-run-host-6dpdj194/summary.json`（退出 1）。把两次 .next 都显式提供仍失败（`/tmp/luna-run-host-_jqm6bz6/summary.json`）；将 .next 的类型改写为 defer bool 也不解决。未用跳过缺省定义检查掩盖缺口。

区分出两个独立阶段：有 inline if 的 invert 先在 defaults.rs::ensure_parameter_default 的定义作用域表达式检查中触发 required.rs 的缺失输入；把 invert 的控制流改成普通 if 后，调用可形成，但被 purity.rs 的 staged 实参 canonical_input_with_layouts 要求拒绝（`/tmp/luna-run-host-_qy61l1r/summary.json`）。后者已在 functions/parameters.md 记录为实现限制。修复必须保留前序 staged 绑定的阶段依据，并将需要实际值的默认实例准备推迟到调用供应；不能删除 canonical 检查或把尚未供应的值伪造为常量来绕过前者。此轮仅定位、保存复现，尚未修复该组合。

基线：工作目录 /home/fin/luna，`python3 -B development/check.py --timeout 900 -- test --release -p luna --test default_calls --test extended_calls --no-fail-fast` 退出 0，报告 `/tmp/luna-tools-rsqf7nlc/result.json`。这不包含新增拒绝复现，不能作为该功能通过的证据。dev 未提交。

## LLVM ldexp 的异构签名与浮点边界（2026-10-25）

新增 ScaleByPowerOfTwo/ldexp 操作及真实 math.ldexp<T>(value:T, exponent:i32)->T 包装，T 限定 f32/f64。它与 powi 共用整数指数参数角色、generic/concrete 检查和 LLVM 异构参数降码，分别保留操作身份。LLVM 直接生成 llvm.ldexp.f32.i32 / llvm.ldexp.f64.i32，不先计算浮点 2^n 再乘，不额外开启 fast-math 或 flush-to-zero。更新 sysroot 清单、显式 CLI 夹具和示例配置；签名回归复用同一检查矩阵，包含错误指数类型、参数颠倒、pending 类型及篡改已签发证据的拒绝。

新示例 examples/host-math-ldexp.fl 覆盖两种精度、最小次正规数、恰在零／最小次正规数中点的下溢、负零、i32 极值指数、上溢和源码实参各求值一次。0.5 × 2^128 / 0.5 × 2^1024 的有限结果专门排除先生成溢出幂的错误改写。math 源码原生套件增至八项，示例目录现有 61 项。

工作目录 /home/fin/luna：
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test host_intrinsic_protocol` 退出 0，37 项通过（`/tmp/luna-tools-2d166sov/result.json`）。
- `python3 -B development/host_examples.py math-ldexp math-powi --release-driver --compare-optimization --timeout 900` 退出 0，两程序 × O0/O3 全部输出 (bool true)，均用默认编译预算（`/tmp/luna-host-examples-0r7kokax/summary.json`）。
- `python3 -B development/library_check.py --release --timeout 900 --stage native-math` 退出 0，八项实际原生测试通过（`/tmp/luna-library-for3zra9/summary.json`）；仅此选定阶段，不代表完整 sysroot。
- NIR lib 五项通过（`/tmp/luna-tools-isxm5sp4/result.json`）；运行器 61 项通过（`/tmp/luna-tools-g317nrhl/result.json`）；全 workspace 格式检查通过（`/tmp/luna-tools-e4q17ike/result.json`），git diff --check 通过。

构建前按已有授权清理 luna release 可重建缓存约 147.6 MiB，避免继续累计已出现的配额压力；本轮没有配额或链接失败。完整 sysroot 与属性／函数 ABI 等前置缺口保持记录，不因新增数学操作宣称语言已完成。dev 未提交。


## 聚合验收报告在配额失败时保留终态（2026-10-25）

上轮 library_check 的 summary.json 写入因 EDQUOT 抛出异常，无法交付已执行阶段的完整汇总。新增 process_tools.save_summary，采用临时文件与原子替换；写入或替换失败保留旧快照，终端输出 summary_saved=false、wrapper_exit_code=125、错误及完整内存 evidence。library_check、host_examples 和 run_host 共用此机制。汇总无法保存时受控停止，不再启动额外构建／执行；普通测试失败仍遵循原有 keep-going / 独立检查策略。run_host 将失败证据送往 stderr，不污染程序 stdout，finally 仍释放私有驱动。示例执行报告缺失／JSON 截断也记录明确失败，不抛出未处理异常或据子进程 0 宣称通过。

故障注入覆盖写入／rename 失败、旧快照保留及恢复、库阶段停止、keep-going 下停止、运行前／驱动完成后／执行完成后的报告失败、只运行一次、退出码保留、驱动释放、原始 stdout 字节及缺失／截断执行报告。运行器原 60 项通过（`/tmp/luna-tools-5ck8ts_q/result.json`），随后新增缺报告用例后的故障组 5 项通过（`/tmp/luna-tools-ruezxia1/result.json`）；没有把后来新增测试计入先前的 60 项。

工作目录 /home/fin/luna，真实正常路径均退出 0：`python3 -B development/library_check.py --release --timeout 900 --stage native-math` 的 7 项原生测试通过（`/tmp/luna-library-6tagibnu/summary.json`）；`python3 -B development/host_examples.py math-powi --release-driver --compare-optimization --timeout 900` 的 O0/O3 均为 (bool true)（`/tmp/luna-host-examples-insuwywp/summary.json`）；`python3 -B development/run_host.py development/examples/host-arithmetic.fl --release-driver --build-timeout 900` 输出 (unsigned-integer 42)（`/tmp/luna-run-host-4btu612w/summary.json`）。git diff --check 通过。未修改 Rust 或语言语义，不重复编译器全量测试；dev 未提交。


## LLVM powi 与异构 intrinsic 参数签名（2026-10-25）

新增 IntegerPower/powi 操作。NIR 的 operand_type 逐参数描述签名：底数和结果为同一 f32/f64，指数固定 i32。Registry::check、defer 和 Checked::validate_template 共用此规则；generic 类型证据仍由既有 PTC 类型域检查提供。实例化、重写后若指数类型变成浮点、u32 或 i64，旧 binding 不能继续授权。LLVM 先按源码次序求所有实参一次，再生成 llvm.powi.f32.i32 / llvm.powi.f64.i32，不将整数指数转换为浮点，不假定特定乘法／舍入顺序。依据 LLVM LangRef 的 powi 契约，链接见 library/README.md。

真实 math 库新增 pure 泛型 powi，更新 sysroot 默认项、示例运行器操作表及显式绑定夹具。examples/host-math-powi.fl 覆盖负指数、i32 最小指数、零指数、负零符号和实参求值顺序，接实际 resources 协议。math 原生源码套件增至七项；默认示例目录现有 60 项。未将这些 target 操作改为 comptime 可执行。

工作目录 /home/fin/luna：
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test host_intrinsic_protocol`：36 项通过（`/tmp/luna-tools-9rex1_g_/result.json`），包括 concrete/pending 异构签名与重写伪造证据拒绝。旧全操作谓词测试矩阵已按各操作参数角色构造；FMA 原生测试仅对未启动子进程的 ETXTBSY 加入有界重试。
- `python3 -B development/host_examples.py math-powi --release-driver --compare-optimization --timeout 900`：O0/O3 原生执行均为 (bool true)，默认编译预算通过（`/tmp/luna-host-examples-x89kk2bq/summary.json`）。
- `python3 -B development/library_check.py --release --timeout 900 --stage manifest --stage native-math`：清单检查及七项 math 原生测试通过（`/tmp/luna-library-k4oyea1r/summary.json`），scope=selected，不代表完整 sysroot。
- NIR lib 5 项通过（`/tmp/luna-tools-i2wl5sgu/result.json`），luna lib 74 项通过（`/tmp/luna-tools-hc1ydnaz/result.json`）。运行器 56 项通过（`/tmp/luna-tools-2wgzgld2/result.json`），格式检查通过（`/tmp/luna-tools-89a9a9nx/result.json`），git diff --check 通过。

初稿测试把 Rigid 字段写成 id，已改为 declaration。联合 lib 构建先遇到 linker Bus error，后续明确出现磁盘配额及报告落盘失败（`/tmp/luna-library-v1_jg4lo`，最终内存证据为子进程 101、wrapper 125，磁盘旧 running 报告不能代表终态）。按已有授权执行 `python3 -B development/cargo.py clean -p luna --release` 释放约 396 MiB 后，上述清单和 luna lib 验证恢复；未删除源码或旧证据。

本轮改动前还复验 resources 365 项全部通过（`/tmp/luna-tools-3h6srguc/result.json`），历史列出的四项失败已不存在。函数 C ABI 仍需通用属性 Object 的求值与检查，维持已有明确拒绝，不靠名称特判放行。dev 工作区未提交。


## Tuple 仿射字段纳入库原生验收（2026-10-25）

library_check.py 新增 native-tuple，复用 host 示例的真实 tuple/meta/resources 包依赖和类型身份绑定。源码测试覆盖嵌套 comptime 类型列表、空 Tuple、单元素求值一次，以及带 Drop 的非 Copy 字段解构。仿射用例通过两个独立标志检查显式消费次序；Drop 的前置观察拒绝重复析构。没有为 Tuple 或 Tracked 添加隐式 Copy。Math 阶段改用与 host 示例相同的 MATH_OPTIONS，删除重复的操作清单。

入口新增可重复 --stage；按原有顺序去重执行所选阶段，终端和 JSON 都标明 scope=selected 及具体列表。不指定仍跑全九阶段，失败继续执行其它独立项；原生结果必须具有预期非零测试数和零失败。

工作目录 /home/fin/luna。`python3 -B development/library_check.py --release --timeout 900` 执行九阶段，八项通过、完整 sysroot 一项失败，最终退出 1（`/tmp/luna-library-yz5zyzav/summary.json`）；该次 Tuple 为前三项。随后加入仿射用例，`python3 -B development/library_check.py --release --timeout 900 --stage native-tuple` 退出 0，四项实际原生测试通过（`/tmp/luna-library-5mx35agp/summary.json`）。完整 sysroot 仍在 HIR lowering 报未解析 panic、String、Any、集合／I/O hooks，未冒充完成。

运行器单测 7 项通过（`/tmp/luna-tools-e3d0w1k8/result.json`），host 示例运行器 9 项通过（`/tmp/luna-tools-zigs7w11/result.json`），git diff --check 通过。本轮没有 Rust 源码修改，不重复 Rust 全量测试或格式化。dev 未提交。


## Sysroot 发现 math 包并绑定 LLVM scalar 默认项（2026-10-25）

单文件 --sysroot 加载器现在发现可选 math/lib.fl，追加在原包列表之后，保留已有包的身份分配。language-items.conf v1 新增 host-intrinsic.<operation> = package::Enum::variant；仓库清单列出所有当前标量操作及真实 math::target::Scalar 声明。默认项按 operation 合并，显式通用 CLI 选项、fma/sqrt/copysign 专用选项以及驱动已绑定身份优先；显式重复／冲突仍进入原有检查，不被合并消掉。非法操作名、重复项、缺少 enum/variant 的 selector 在加载时拒绝；解析后仍验证 intrinsic enum 身份和签名。AST/HIR 路径及显式 package-set 客户端不会隐式应用这些默认项。

新增真实 math/resources 模块的隔离 sysroot 回归，在 LLVM O0/O3 下执行 fma、sqrt 和 wrapping_abs；覆盖两种 CLI 覆盖写法、错误默认项保留旧可执行文件，以及绑定驱动身份优先。该夹具未加载尚不完整的 String/I/O 等模块，不代表完整 sysroot 已可用。更新使用真实仓库清单的旧 Tuple 测试，使其同时加载清单声明的实际 math 包。

工作目录 /home/fin/luna，`python3 -B development/check.py --timeout 900 -- test --release -p luna --lib --test frontend_cli sysroot --no-fail-fast` 退出 0，2 项配置单测及 18 项 sysroot CLI 测试通过（`/tmp/luna-tools-tttwlrvr/result.json`）。最初夹具缺目录、加载器缺 math 发现已修复；随后真实契约在默认 100000 总预算耗尽（`/tmp/luna-tools-5tlicgmf/result.json`）。原生组合回归显式使用 --fuel 1000000，未提高全局默认预算，不宣称预算问题已修复。最终格式化通过（`/tmp/luna-tools-lwlwuqt4/result.json`），git diff --check 通过。dev 工作区未提交。


## quoted tuple 契约的资源拒绝已查明（2026-10-25）

资源状态对照确认，旧 repro 中的契约解构把调用者 tuple 从完整状态消费为空；根集合、指针和临时值均未变化。这是缺少 Copy 的正确拒绝，不是观察路径泄漏临时绑定的证据。保留 repros/host-quote-contract-patterns.fl 为拒绝例，不削弱资源 gate。临时状态追踪代码已删除。

新增 examples/host-quote-contract-patterns.fl，以应用包本地 trait Duplicate 为真实库 Tuple<bool, bool> 显式提供实现，并由 host 参数绑定 Copy 协议身份。这符合本地 trait 的归属规则；不能在应用包为外部 resources.Copy 和外部 Tuple 声明实现，也不能因元素均 Copy 自动授予 Tuple Copy。标准库 Tuple 的通用复制实现仍待补充。

工作目录 /home/fin/luna，`python3 -B development/host_examples.py quote-contract-patterns --release-driver --compare-optimization` 退出 0，LLVM O0/O3 编译、链接、执行均输出 (bool true)，证据 `/tmp/luna-host-examples-20w7vjsx/summary.json`。目录现有 59 项；不是本轮全目录验收。此案例显式使用 1000000 总预算。新增 host 回归以同一源码删除 Copy 实现，分别验证两档接受／拒绝及拒绝时没有可执行文件。`python3 -B development/check.py --timeout 900 -- test --release -p luna --test host quoted_tuple_contract_requires_explicit_copy_and_preserves_caller` 退出 0，1 项通过、653 项未运行（`/tmp/luna-tools-2z2q5ebz/result.json`）。运行器 9 项通过（`/tmp/luna-tools-e5l69t9u/result.json`），全 workspace 格式检查通过（`/tmp/luna-tools-iao2w2dr/result.json`），git diff --check 通过。dev 未提交。


## host 集成后的资源检查边界（2026-10-25）

host 653 项与库协议 14 项集成检查通过（`/tmp/luna-tools-a_ujwx8k/result.json`）。真实 Tuple/meta/resources 跨包的默认预算原生 O0/O3 示例已交付，见 host-pipeline.md；源码解释层 execution 1368 项通过的结论保持，但不能据此推断全部资源受检调用均已实现。

新增 repros/host-quote-contract-patterns.fl：相同 quoted 计算模式接实际资源协议时，报 guard expression changes caller resources and needs failure-path resource transfer（`/tmp/luna-run-host-2ncqg2lp/summary.json`）。它不是此前的程序证明预算失败。实际 Tuple 未自动取得 Copy；应用包为外部 trait／外部 Tuple 添加实现也被归属检查拒绝，试验实现已删除。后续需核对观察路径及 tuple 复制能力的正确声明／归属，保持资源准入与规范身份；该复现未登记为可运行案例。

## 内联契约局部绑定退出作用域，执行组恢复（2026-10-25）

临时表达式成本计量 `/tmp/luna-tools-rdojbiex/stderr.log` 发现：失败 quoted 计算模式第一次 RequirePredicate 后保留 76 条关系、20 个当前值；第二次展开推到 204 条关系、30 个值，单次检查消耗约 69 万步并耗尽。并非只有最后一条算术操作昂贵，而是契约内部解构绑定和构造临时值留在调用者状态中。计量代码已完全删除。

RequirePredicate／EnsurePredicate 在求值前记录外部根集合，证明及重放成功后调用已有 State::keep 作用域退出逻辑；外部值、仍可访问的内存观察和投影后的关系保留，契约局部绑定不能从 void 义务中逃逸。每次义务仍独立检查，没有跳过重复契约或扩大默认预算。

工作目录 /home/fin/luna。`python3 -B development/check.py --timeout 900 -- test --release -p luna --test execution --test quote_functions --test staged_aggregate_contracts --no-fail-fast` 的 execution 1368 和 staged_aggregate_contracts 31 项全部通过（`/tmp/luna-tools-y772he6_/result.json`）；quote_functions 当时 103 通过、1 项过时拒绝预期失败。该用例的普通消费者和析构主体已可推导内存效果，符合 docs/resource-management/contracts-and-verification.md 的默认推导范围；原先要求 pointer dependencies 拒绝已不符合现行实现。测试改为同时验证普通／quoted 消费后的析构写入恰好发生一次、析构前提为假时仍拒绝，没有修改编译器准入来迎合测试。随后完整 quote_functions 104 项通过（`/tmp/luna-tools-gr9dlxan/result.json`）。本次共 1503 项相关 Rust 测试通过，之前持续失败的 destructuring_method_contracts_survive_trait_parameter_remapping 已恢复，包含全部正例与拒绝变体。

`python3 -B development/host_examples.py memory-result-contracts library-option-callback --release-driver --compare-optimization` 退出 0，四次 O0/O3 原生运行通过（`/tmp/luna-host-examples-izvr3o52/summary.json`）。内存示例仍需显式 1000000 总预算；默认 100000 复核仍耗尽（`/tmp/luna-run-host-e96y92fd/summary.json`）。完整 sysroot、动态 pattern 作用域等未完成项继续保留，不能据此宣布前端或语言全部完成。格式检查通过（`/tmp/luna-tools-z1l4w9_c/result.json`），git diff --check 通过；dev 未提交。

## 复制证据快照共享与精确批次去重（2026-10-25）

解构回归现在在失败时打印具体 Flurry 夹具，定位为 quote fn keep 的 requires 内含 `let (first,second) and (first,second) is (left,right)=pair`。首次诊断辅助函数名被局部变量遮蔽，编译失败（`/tmp/luna-tools-21ug5kc5/result.json`）；重命名后取得实际夹具（`/tmp/luna-tools-v5wjwad3/result.json`）。未改变任何接受／拒绝预期。

PreparedCopy／InstalledCopy 共享 Arc<Facts> 的冻结输入；同一次 prepare_copy 中尚无既有证据的组件惰性创建一次快照。预先携带证据的组件保留各自原快照，不同准备时刻仍独立冻结。安装仅共享只读证明输入，目标身份及表达式仍分别生成。finish_copy_batch 对同一 Arc 快照和完全相同的完整 substitution 只导入一次关系／条件；不同映射不合并，每个结果表达式的关系仍单独处理。所有遍历与映射比较继续计入预算，缓存只存在于当前批次。

工作目录 /home/fin/luna，最终 `python3 -B development/check.py --timeout 900 -- test --release -p luna --test execution --no-fail-fast` 退出 101（`/tmp/luna-tools-i7cn1j0r/result.json`）：1367 通过、1 失败，仍为上述 quoted 计算模式，报错范围变为 249..308。只加入共享时的中间检查亦为同一失败（`/tmp/luna-tools-0c6ge84v/result.json`）。这次没有解除该阻塞；后续需直接计量此夹具各阶段，不能继续仅凭最后耗尽位置估计主成本。

`python3 -B development/host_examples.py memory-result-contracts library-option-callback --release-driver --compare-optimization` 退出 0，四次 O0/O3 原生运行通过（`/tmp/luna-host-examples-pjp9txn5/summary.json`）。内存例仍使用目录显式 1000000 总预算；另以默认 100000 复核仍耗尽（`/tmp/luna-run-host-93upy6bj/summary.json`），不标记默认预算问题已修复。格式检查通过（`/tmp/luna-tools-88i8akv0/result.json`），git diff --check 通过；dev 未提交。

## 完整布尔观察树与原生预算区分（2026-10-25）

flow/operators.rs 将只含已绑定标量比较、布尔局部值／字面量、Not／And／Or 的总观察树组装成谓词，不再每层执行状态 fork/join。内部组合节点不分别证明区间，由 enclosing inner 统一判定；叶节点保留原有检查、深度和预算限制。调用、内存读取、算术及其他可能失败或改变状态的节点仍走原有短路分支路径，未修改执行器／LLVM 的求值顺序。

新增源码测试检查嵌套布尔树带来的安全加法精化、错误界限拒绝，以及右侧调用的执行／跳过和可观察写入。首次夹具把 p.* = true 写成 p.*=true，引起词法歧义，已修正；初次报告 `/tmp/luna-tools-95ty9iht/result.json` 不计为通过。修正后定向测试通过（`/tmp/luna-tools-6_1cwgrx/result.json`），最终 `python3 -B development/check.py --timeout 900 -- test --release -p luna --test execution --no-fail-fast` 退出 101：1367 通过、1 失败（`/tmp/luna-tools-ek1a2l0x/result.json`），仍为 destructuring_method_contracts_survive_trait_parameter_remapping 的 250..303 BudgetExhausted。

原生内存复现的默认总预算为 CLI 的 100000，而非各源码验证入口常用的独立 1000000。默认仍在 27179..27258 耗尽（`/tmp/luna-run-host-mqindtf7/summary.json`）；显式 --fuel 1000000 后 LLVM O0 编译／链接／执行通过（`/tmp/luna-run-host-eucr7ims/summary.json`）。未修改默认预算，不能记作默认预算不足已修复。完整源码已移到 examples/host-memory-result-contracts.fl，原 repro 为相对软链接；目录显式登记该预算和真实 resources 协议。O0/O3 均通过（`/tmp/luna-host-examples-t5dqiep4/summary.json`），详见 host-pipeline.md。

工作目录 /home/fin/luna。运行器八项测试通过（`/tmp/luna-tools-5j99b2_a/result.json`），格式检查通过（`/tmp/luna-tools-rm_3hzcm/result.json`）。dev 未提交。

## 稀疏差分最短路闭包（2026-10-25）

DifferenceBounds 使用稀疏边求完整最短路矩阵：Bellman-Ford 的虚拟零成本源覆盖全部节点，检查不连通负环并计算势函数；非负重标权后使用带预算比较的最小堆执行各源 Dijkstra，最后恢复原始权重。没有裁剪查询之外的图，没有改变最终矩阵或 S-expression 接口。矩阵大小限制、系数大小限制与共享工作预算继续生效。

新增 128 个确定性图与独立 i128 Floyd 参考实现逐项比较，包含负边、负自环、不连通负环、不可达节点。原有 24 变量独立等式对的 30000 步预算测试仍通过，并检查跨零节点的区间路径、零预算拒绝及新预算恢复。

工作目录 /home/fin/luna，`python3 -B development/check.py --timeout 900 -- test --release -p elaboration -p luna --lib --test verification --test execution --no-fail-fast` 退出 101（`/tmp/luna-tools-pr2zp37_/result.json`）：256+85+73 项通过，execution 1366 通过、1 失败，仍为解构方法 BudgetExhausted。原始内存契约示例经真实 resources 协议及 LLVM O0 复核，仍在 residual run 的 27179..27210 耗尽（`/tmp/luna-run-host-jsas9rhk/summary.json`）；未执行原生程序，未加入可运行目录。格式检查通过（`/tmp/luna-tools-ru4gy6jc/result.json`），git diff --check 通过；dev 未提交。

闭包算法替换没有单独解除完整复现。后续关注 flow/operators.rs::logical_expression：左结合 And／Or 每层先 fork/join，再对累积条件重新 assume 两种真值；simple_comparison 分支也主动进行 enclosure。应检查这些累积路径上的重复推导和快照搬运次数，避免继续只优化最终耗尽的算术操作。

## 合取假设直接顺序传播（2026-10-25）

Facts::assume 不再为尚未判定的 And=true／Or=false 假设先执行整树 enclosure；assume_inner_mode 直接依次加入两个操作数，保留 replay 模式，每个新的原子关系由原有路径传播。需要分支的 And=false／Or=true 仍使用原有分支／合并规则。外层 fork 后才发布结果的事务边界不变。

工作目录 /home/fin/luna，`python3 -B development/check.py --timeout 900 -- test --release -p elaboration -p luna --lib --test verification --test execution --no-fail-fast` 退出 101（`/tmp/luna-tools-s74kg62q/result.json`）：255+85+73 项通过，包含小整数域布尔组合穷举、矛盾分支与失败事务回滚；execution 1366 通过、1 失败，仍为解构方法的 BudgetExhausted。随后原始内存契约示例经 run_host.py、真实 resources 协议、LLVM O0 复核，仍在残余契约 27179..27210 耗尽（`/tmp/luna-run-host-h613sag0/summary.json`），未执行程序。

此改动消除了合取假设的重复整树证明，但没有解除完整示例的阻塞。后续应处理局部查询反复进行全图差分闭包的成本，保留跨中间变量的传递关系及矛盾检测；不要继续把单个快速路径通过等同于完整恢复。格式化通过（`/tmp/luna-tools-j4mz655m/result.json`）；dev 未提交。

## 关系推导的单点提前返回（2026-10-25）

完整 host-memory-result-contracts.fl 在真实 resources Copy／Drop 绑定下仍被残余契约检查拒绝（`/tmp/luna-run-host-8awx8l32/summary.json`）。临时耗尽回溯 `/tmp/luna-run-host-u0qu5yp7/build/stderr.log` 指向 Facts::bound → DifferenceBounds::build_with_nodes → Polynomial::enclosure → Condition::truth_by_enclosure → logical_expression；回溯代码已删除。

Polynomial::enclosure 在单行关系推导已经得到单点区间时立即返回，保留已使用关系的全部值依赖，不再继续扫描或构建差分闭包。新增两条不等式证明 a-b=0、附加 64 个关联值的 1000 步测试，同时检查 a/b 依赖及重写 b 后旧查询失效。没有改变默认预算或把程序命题移入 PTC。

工作目录 /home/fin/luna，`python3 -B development/check.py --timeout 900 -- test --release -p elaboration -p luna --lib --test verification --test execution --no-fail-fast` 退出 101（`/tmp/luna-tools-jn6j1l2v/result.json`）：255 项 elaboration 库、85 项 verification、73 项 luna 库全通过；execution 1366 通过、1 失败，仍是解构方法的 250..303 BudgetExhausted。格式检查通过（`/tmp/luna-tools-vyoj_9i2/result.json`）。随后通过 run_host.py 使用真实 resources 包、Copy／Drop 绑定和 LLVM O0 复跑原始内存示例，仍在残余契约 run 的 27179..27210 报 BudgetExhausted（`/tmp/luna-run-host-zxvr1dqe/summary.json`），没有产生或执行原生程序，未加入可运行目录。局部单点优化未解除完整复现阻塞；后续需要继续减少非单点查询的重复差分图构建。dev 未提交。

## host 交付前的前端停靠状态（2026-10-25）

批量 copy 的关系搬运现在借用源关系，避免先克隆整个多项式集合；同一 lineage 中不涉及映射、版本仍有效且目标已具有的精确关系不再重命名和重复导入。受影响关系仍搬运到新身份，跨帧与失效快照保留 projection。add_row 使用 RelationStore 的单次去重及容量检查，没有扩大默认预算。

工作目录 `/home/fin/luna`。定向 execution 两项检查退出 101（`/tmp/luna-tools-p_omg52p/result.json`）：写指针联合后置条件通过，解构方法仍在 250..303 耗尽预算。随后通过 `python3 -B development/check.py --timeout 900 -- test --release -p luna --test execution --no-fail-fast` 复核受影响执行组：1366 通过、1 失败（`/tmp/luna-tools-9apgnsfj/result.json`），失败仍仅为 destructuring_method_contracts_survive_trait_parameter_remapping。这项优化不等于解除该阻塞。

后续前端工作继续从该解构契约、host-memory-result-contracts.fl 的残余资源契约及下文动态 pattern 作用域接线恢复。当前不扩大语义修复范围，先交付可运行 host 示例。全 workspace 格式检查通过（`/tmp/luna-tools-ltas_fjx/result.json`）。dev 工作区保留，未提交。

## 关系索引与查询相关行，源码执行仅余一项失败（2026-10-25）

实际耗尽回溯见 `/tmp/luna-tools-hgvbev08/result.json`（退出 101）。写指针案例停在 logical_expression → assume_inner_mode → Polynomial::enclosure → interval multiplication；解构方法案例停在 pattern → finish_copy_batch → add_row → contains_relation。临时 Unknown 转换回溯已移除。

Facts 的关系存储改为保留插入顺序的 RelationStore，附带预算化指纹索引。哈希只筛选候选，碰撞仍进行完整预算化结构比较；副本、重命名和作用域裁剪重建索引，clear 同时清空两部分。证明重放查询同一存储，证明对象仍保存原有多项式行；Facts 的 S-expression 顺序不依赖哈希。新增 512 行低预算查询、强制碰撞、重复插入、耗尽不改变存储和清空后恢复测试。首次编译因 PlaceId／ValueId 缺少 Hash 失败（`/tmp/luna-tools-t28vsimu/result.json`），已补齐。

Polynomial::enclosure 的单行加减尝试只访问与查询共享非恒定单项式的行；无交集时区间运算不能通过该已知 <=0 行收紧查询。完整行集不删除，后续差分闭包仍能使用传递关系。新增 64 条无关行下 5000 步区间查询，核对结果和必要值依赖。

工作目录 /home/fin/luna，`python3 -B development/check.py --timeout 900 -- test --release -p elaboration -p luna --lib --test verification --test execution --no-fail-fast` 最终退出 101（`/tmp/luna-tools-qa61znyv/result.json`）：elaboration 库 254、verification 85、luna 库 73 项全部通过；execution 1366 通过、1 失败。writing_pointer_postconditions_export_joint_result_memory_relations 已通过，包含其错误契约／修改后使用旧关系等拒绝分支。剩余 destructuring_method_contracts_survive_trait_parameter_remapping 仍在 250..303 报 BudgetExhausted。

repros/host-memory-result-contracts.fl 进一步尝试 LLVM O0：无协议绑定依法拒绝 nominal storage（`/tmp/luna-host-examples-jgcsfgsy/summary.json`）；绑定实际 resources Copy／Drop 后，在资源受检残余契约 run 中报 BudgetExhausted（`/tmp/luna-host-examples-loqes6la/summary.json`）。尚未执行原生程序或 O3，未登记为可运行示例；保留源码复现。后续需继续处理批量关系搬运成本及 residual program contract 检查，而非放宽资源准入。dev 未提交。

## 调用预算追踪与区间条件快速查询（2026-10-25）

对两个失败用例临时追踪 call_expression 各阶段。`/tmp/luna-tools-w8uq56v4/result.json` 退出 101；构建曾触发配额错误，受管入口自动清理可重建缓存并重试后实际执行。stderr 中写指针案例的 867..883 调用结束仍余 940828 步，951..962 调用开始仅余 1445 步，说明大量消耗发生在调用之间的条件处理，不能把最终报错位置直接当作主要消耗点。临时环境开关及 trace 调用已全部移除。首次移除遗漏一处 trace 的检查编译失败（`/tmp/luna-tools-rf_b2n3n/result.json`），随后清除。

Condition::truth_by_interval 只用当前区间判定；Facts::known_condition 先验证整个谓词的所有值版本，再尝试区间结果，未决时继续原有保留条件／别名搜索。此路径不会递归进入符号或保留条件推理。remember_condition 的内部去重路径不采用该捷径，避免丢失需要跨 join 保留的 disequality。新增六种比较在 32 条无关保留条件下各以 256 步判定，以及旧值 x==x 不得通过消去绕过版本检查。

最终工作目录 /home/fin/luna，`python3 -B development/check.py --timeout 900 -- test --release -p elaboration -p luna --lib --test verification --test execution --no-fail-fast` 退出 101（`/tmp/luna-tools-12902w6z/result.json`）：252 项 elaboration 库、85 项 verification、73 项 luna 库测试通过，execution 1365 通过、2 失败。两个契约仍未修复，写指针失败位置现为 901..962，解构方法仍为 250..303；不把局部快速路径通过称为解决源码预算问题。早期仅在 truth_by_enclosure 中加捷径的尝试亦未解决（`/tmp/luna-tools-6jdb0rfl/result.json`），最终实现已改为上述入口。

写指针源码单独保存为 repros/host-memory-result-contracts.fl，尚未纳入可运行 host 示例目录，未执行原生验收。继续排查应定位关系搬运／布尔条件传播内部的实际耗尽调用栈，不能只优化报错的最后一次调用。格式化退出 0（`/tmp/luna-tools-6z6kx148/result.json`），dev 未提交。

## 稀疏差分闭包的预算优化（2026-10-25）

DifferenceBounds 的 Floyd 闭包把常量零节点放到最后作为 pivot，避免过早用区间边连接所有独立关系分量；缺失 a→k 路径时跳过整行 b 扫描。所有 pivot 仍被遍历，最终仍包含经由零节点的路径，矛盾对角线检查保留；没有增加默认预算、降低算术精度或引入 PTC。

新增 24 个变量、12 组独立等式的测试，在 30,000 步以内完成闭包并逐对核验：同组差分上界 0，跨组上界 255。还检查零预算失败及新预算恢复。工作目录 /home/fin/luna，`python3 -B development/check.py --timeout 900 -- test --release -p elaboration -p luna --lib --test verification --test execution --no-fail-fast` 退出 101（`/tmp/luna-tools-pnd6mjus/result.json`）：elaboration 库 251、verification 85、luna 库 73 项全部通过；execution 1365 通过、2 失败，仍为下文两项程序验证 BudgetExhausted，位置也未改变。此优化降低了稀疏闭包开销，但不能据此宣称解决这两项源码失败，后续仍需定位调用契约传播的其他消耗。格式化退出 0（`/tmp/luna-tools-85h28f0f/result.json`），git diff --check 通过。dev 未提交。

## 执行测试对齐已实现行为，剩余两项验证失败（2026-10-25）

核对后确认两项旧失败来自过时的拒绝预期，不修改编译器语义来迎合测试。comptime_interpretor 的 eval_not 已使用 heap continuation，线性 Not 链按节点扣步数而不递增原生递归深度；原测试仍要求 384 层 Not 报 DepthExhausted。现保留 2 MiB 栈、递归函数与嵌套 pattern 的深度拒绝及恢复检查，另验证 Not 链成功、至少消费 384 步、仅余 128 步时 StepsExhausted、独立正常实例仍可执行。

受检执行入口已支持 direct-passing 的复制计划和清理；Config 的测试不再要求这个已实现入口报 cleanup protocol 错误，而是核对 Bool(true)，即两次交付的内联字段不互相别名。原始无协议执行的 EffectValueConsumed 拒绝与撤销 Copy 实现后的 affine effect slot 拒绝均保留。本轮只修正和加强测试，没有放宽执行准入。

工作目录 /home/fin/luna，`python3 -B development/check.py --timeout 900 -- test --release -p luna --test execution --no-fail-fast` 退出 101（`/tmp/luna-tools-6wxkjtqc/result.json`）：1365 通过、2 失败。剩余为 writing_pointer_postconditions_export_joint_result_memory_relations 与 destructuring_method_contracts_survive_trait_parameter_remapping，均为程序验证 BudgetExhausted；仍需修复，不能记为整个 execution 组通过。格式化退出 0（`/tmp/luna-tools-f2y5uymd/result.json`），git diff --check 通过。dev 未提交。

## 命题查询计量与剩余四项执行失败（2026-10-25）

World 增加 decide_trait_metered，trait 命题调用按返回的剩余额度收费，不再为每个原子无条件耗掉整份预留预算。输入形成失败也回写预算；已有约束查找、实现选择和 PTC 证明分别按实际计量扣费。未提供计量的结构视图／闭合性检查仍保守扣除其额度，传入额度受当前剩余预算限制，不在耗尽后获得新预算。证明仍属于当前不可变实现世界；未增加全局 trait 事实或跨世界缓存。

新增连续四次复合布尔契约检查共用预算、零预算拒绝、失败查询收费与全局事实不泄漏回归。首次新增测试错误要求复合命题必须返回 NotEntailed 反例模型，实际并无此接口保证；改为检查缺失 trait 的明确 Excluded 判定，其他预算与证据要求保留。该首次运行退出 101（`/tmp/luna-tools-lh3ndyxd/result.json`），不计为通过；其中源码布尔相关 30 项通过。

工作目录 /home/fin/luna。最终 `python3 -B development/check.py --timeout 900 -- test --release -p elaboration -p luna --test capabilities --test execution --no-fail-fast` 退出 101（`/tmp/luna-tools-zbnh5i0d/result.json`）：capabilities 41 项全通过；execution 1363 通过、4 失败，没有新增失败。boolean_method_requirements_use_the_selected_implementation_world 已通过，本轮最先单独复核亦通过（`/tmp/luna-tools-m1wmxiu9/result.json`），不将恢复仅归因于本轮计量改动。

剩余失败为 direct_passing_copy_plan_repeats_nominal_results_without_aliasing_inline_fields、interpreter_depth_budget_fires_on_a_two_mebibyte_host_stack_and_recovers（预期失败却得到 Bool(true)），以及 writing_pointer_postconditions_export_joint_result_memory_relations、destructuring_method_contracts_survive_trait_parameter_remapping（程序验证 BudgetExhausted）。这些仍待修复，不把整个 execution 组标绿。格式化退出 0（`/tmp/luna-tools-ddj89027/result.json`），dev 未提交。

## 条件实现预算修复（2026-10-25）

候选搜索只扣实际消耗，剩余搜索额度不再被第二次从候选预算中扣除；候选条件的直接／补充 PTC 证明改为按 prove_metered 返回的实际消耗收费。保留每次查询上限、深度限制、循环检测和局部实现世界证据。

`conditional_impl_requirements_compose_nested_boolean_queries` 与 `conditional_inherent_impl_executes_in_both_phases` 已恢复；前者同时覆盖 runtime 和显式 comptime invocation。工作目录 /home/fin/luna，`python3 -B development/check.py --timeout 900 -- test --release -p luna --test execution conditional_ -- --test-threads=1` 退出 0，20 项通过（`/tmp/luna-tools-rtl4lygi/result.json`）。底层 `python3 -B development/check.py --timeout 900 -- test --release -p elaboration --test capabilities --test implementation --no-fail-fast` 退出 0（`/tmp/luna-tools-0rbr8w22/result.json`），包括新增的共享预算重复查询、零预算失败后恢复与证据重放。格式化退出 0（`/tmp/luna-tools-b8s1yeuu/result.json`），git diff --check 通过；dev 未提交。

下文七项失败为修复前检查点，其中这两项已解除，其他五项不能据本次过滤测试判定修复。

## 当前执行回归缺口（2026-10-25）

本次转入 host 示例交付时，execution 组串行复核仍为 1360 通过、7 失败，不能标记语义前端整体通过。工作目录 /home/fin/luna，命令 `python3 -B development/check.py --timeout 900 -- test --release -p luna --test execution -- --test-threads=1`，退出 101，证据 `/tmp/luna-tools-hyulw7s8/result.json`；同七项也出现在默认并发运行中，不能归因于测试并发。尚未完成相对修改前版本的归因。

- boolean_method_requirements_use_the_selected_implementation_world：comptime invocation 的 TypeContractUnresolved。
- conditional_impl_requirements_compose_nested_boolean_queries、conditional_inherent_impl_executes_in_both_phases：实现条件未决。
- destructuring_method_contracts_survive_trait_parameter_remapping、writing_pointer_postconditions_export_joint_result_memory_relations：验证 BudgetExhausted。
- direct_passing_copy_plan_repeats_nominal_results_without_aliasing_inline_fields、interpreter_depth_budget_fires_on_a_two_mebibyte_host_stack_and_recovers：预期拒绝／预算失败却返回 Bool(true)，需要核对实际行为和预算契约，不直接放宽测试预期。

这些失败留作恢复前端的明确入口；可运行示例的证据另见 host-pipeline.md，不代表上述缺口已修复。

## 恢复语义前端时的入口（2026-10-25）

当前优先交付可运行的 LLVM host 示例。已有前端实现与下文阶段证据保留；未完成能力不通过放宽类型身份或资源准入来绕过。恢复时优先接续 `repros/comptime-factory-pattern-binding.fl`：resolve 的可恢复名称计划尚未接入 HIR lowering／elaboration 的动态 pattern 输出作用域，需同时处理 Pending 遮蔽、作用域退出、输出类别与调用环境，不能展开 Type 工厂的 normal-form 头猜测输出名字。

未知普通参数下任意工厂结果的部分字段推导、一般浮点数值契约证明仍未完成。host 所需的资源修复及原生验收单独记入 [host-pipeline.md](host-pipeline.md)；完整 sysroot 的 panic、String／Any、分配和 I/O 接线不能由独立示例通过推定完成。直接运行入口见 [README.md](README.md)。

## 当前接续边界（2026-10-24）

2026-10-25：Ast::validate_structure 在节点布局检查之后校验直接节点引用、列表句柄及列表元素；standalone frontend 和 VFS scanner 均在扫描前调用。错误用 S-expression 定位拥有者节点、槽位和列表元素。Id 原始符号字及 FnType flags 不解释为节点，可选直接槽的 0 保留；列表元素的 0／越界编号与损坏列表范围拒绝。该检查不证明无环，也不检查各语法位置是否必须有节点。

工作目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 900 -- test --release -p ast -p parse -p resolve -p ast_lowering` 退出 0，123 项通过（`/tmp/luna-tools-lfj1v_6f/result.json`）。同一入口执行 `test --release -p luna --test host --test cmtt --test library_protocols --no-fail-fast` 退出 0：611 + 523 + 11 项通过（`/tmp/luna-tools-vdsdh3qk/result.json`）。格式化通过（`/tmp/luna-tools-6ckp8gzn/result.json`）。语义前端后续工作继续按下列边界接续；dev 未提交。

2026-10-25：结构审查发现 AtomicBlock 的 parser 写入两个槽位（列表、主体），NodeType 却声明三个（两个节点、列表）。现统一为 SingleWithMultiChildren 的主体、资源列表布局，通用 dump／visitor 因而不再误读列表句柄或越界。解析回归覆盖零／一／多资源，并验证节点名称、主体以及两种 dump 路径。此改动只修复语法树，未开放 atomic 的 HIR、资源协议或运行时实现；一般引用／列表完整性校验仍是后续工作。`python3 -B development/check.py --timeout 900 -- test --release -p ast -p parse -p resolve -p ast_lowering` 退出 0，121 项通过（`/tmp/luna-tools-10qeu4pj/result.json`）。格式化退出 0（`/tmp/luna-tools-2lldt3_3/result.json`），git diff --check 通过；工作目录 `/home/fin/luna`，dev 未提交。

2026-10-25：新增 Ast::validate_node_layouts 与可 Display 的 LayoutError。解析结果在 standalone AST dump／名称扫描前检查节点的实际槽位数、存储边界和 span 元数据；错误显示为 S-expression，并在文件扫描路径附带节点 span。Id 的两个原始符号字段、Symbol 的单个 token 句柄、FnType 的 flags 均按存储布局处理，不冒充 visitor 子节点。检查不证明图无环或多子节点列表内部语义。回归覆盖缺失 predicate 槽、后续分配不能掩盖错误、原始 flags 和符号 payload。

前端四 crate 最终命令 `python3 -B development/check.py --timeout 900 -- test --release -p ast -p parse -p resolve -p ast_lowering`：120 项通过（`/tmp/luna-tools-70ttlq0v/result.json`）。host／真实库集成命令 `python3 -B development/check.py --timeout 900 -- test --release -p luna --test host --test library_protocols --no-fail-fast`：611 + 11 项通过（`/tmp/luna-tools-6o98orax/result.json`）。初稿误将 visitor NoChild 当作零存储槽导致 Id 被拒绝，已按实际 token 布局修正；修正前启动的集成运行 `/tmp/luna-tools-9xfpn58a/result.json` 失败，不计为最终证据。工作目录 `/home/fin/luna`；格式化退出 0（`/tmp/luna-tools-4jezqy1q/result.json`），git diff --check 通过；dev 未提交。

2026-10-25：AST 现在为每个节点记录 children_len，get_children/get_node 只暴露该节点直接存储槽，multi-child 仍通过独立列表句柄取得，FnType flags 仍是原始值。此前尾切片会把后续 arena 分配混入旧节点，已经移除这一约定（取代下段“保留尾切片”的阶段说明）。回归覆盖后续单／多子节点分配、空叶节点、clone 和损坏长度。`python3 -B development/check.py --timeout 900 -- test --release -p ast -p parse -p resolve -p ast_lowering` 最终退出 0，117 项通过（`/tmp/luna-tools-p24cxd_o/result.json`）。初次执行链接时配额耗尽且日志截断（`/tmp/luna-tools-2_q4t07_/result.json`），不计为通过。经授权的 release luna/elaboration 包产物清理释放约 430 MiB 后重试成功，没有删除源码或下载缓存。Rust 格式化退出 0（`/tmp/luna-tools-rgonqv_q/result.json`），git diff --check 通过。dev 未提交。

AST 查询修复了 node_index == nodes.len() 的越界判断，并对公开 arena 的缺失 span／children_start 元数据、越界 child 起点及多子节点列表声明长度进行检查。无效查询返回 None／空子节点切片；未改变有效节点的布局或尾切片约定。新增 invalid handle、最大编号、空列表、截断／超长列表与缺失元数据回归。`python3 -B development/check.py --timeout 900 -- test --release -p ast -p parse` 退出 0（`/tmp/luna-tools-6oa73nfl/result.json`）；格式化退出 0（`/tmp/luna-tools-gcu7h2up/result.json`），git diff --check 通过。工作目录 `/home/fin/luna`；dev 未提交。

跨调用平方／立方后置条件的 `'old` 关系已恢复：导出 callee 快照等式时优先连接 caller 的 immutable entry 观察值，保留当前版本检查和显式输入／结果映射。resources 全组 357 项通过，新增正反回归及 LLVM O0/O3 示例通过；此前记录的四项资源失败均已解除。验证和限制见 host-pipeline.md 最新段落。

验证语句的 AST 存储已统一为 `[label, predicate]`，语句位置保留空 label；此前 parser 写入单子节点，而 NodeType 声明双子节点，作用域扫描因此可能误读相邻 arena 存储并重新扫描无关函数体。`assumes` lowering 同步读取 predicate 槽。Tuple 指针跨调用原回归已通过，未修改 Tuple 定义或资源预期。新增解析布局／缺失谓词测试通过；最终集成证据见 host-pipeline.md。

非原生类型的六个普通比较运算符已通过显式 Eq／Ord 身份绑定进入 selected_method／finish_call，分别调用 eq/ne/lt/le/gt/ge，返回 bool，保留左右各一次的求值顺序和 Never 控制流。绑定由普通 CLI、多包入口及 sysroot manifest 提供，支持泛型前提、真实库默认方法和覆盖的 ne。原生标量／BitVec 相等与标量有序比较保持原有节点，包括 IEEE 行为；TypeEqual 保持独立，Identity<T> 与 T 没有关系依据时仍为 Unknown，不展开或据此断言不等。模式常量观察与编译期查询键协议没有由本轮扩展。跨包 Version 示例覆盖六个运算符，验证见 host-pipeline.md。

平台整数 intrinsic 已接通位操作、usize 位计数／反转／funnel shift／旋转、usize/isize 饱和加减与整数 min/max，以及 isize wrapping_abs。byte_swap 也接受 usize，但具体目标位宽必须是 16 的正整数倍；未指定布局和未实例化的模板保留延迟检查。源码准入、NIR phase 校验、程序验证与 LLVM 发射共用布局限制，更换包目标后重新检查，不缓存旧布局证据。验证证据见 host-pipeline.md。

泛型平台整数的加减乘、无符号除余和有符号取负已接通。类型身份与布局分离：未消费布局的声明可保持 target=None，具体执行和 LLVM 降码使用显式 package.target，不将平台类型改写为同宽固定整数。机器运算验证入口中的泛型平台类型实例，在目标未定时使用私有证明副本检查所有合法位宽（8 到 128、每 8 位一档），不选择宿主布局或改写源码文字。普通非泛型平台文字仍需显式布局。纯度分析不以布局可用性判定平台算术是否有副作用；溢出义务保持独立。有限类型案例规划使用带预算的 primitive 条件化简，未知关系仍交给既有类型求解，normal-form 头不展开。下面各段保留阶段记录，平台算术的旧限制以本段为准。

泛型相等与有序比较已纳入 usize／isize，P4 的有限标量案例也保持这两个独立类型。比较声明不需要预选位宽；Package.target 可保持 None，而具体执行仍需要显式目标布局。源码回归对同一调用在无目标、32 位、64 位环境作对照，并分别检查各目标边界；host LLVM 仅执行与实际工具链布局一致的案例。generic-order 示例加入平台计数／偏移值。此处只接通比较及行为分类，未把一般泛型平台整数算术视为完成。证据见 host-pipeline.md。

比较表达式现与算术／一元操作保持一致：两侧完成 elaboration 后，左侧若为 Never，直接保留左侧的控制流，不依据不会执行的右侧猜测相等／排序域。右侧仍接受名称和类型检查。右侧提前退出时保留既有相等节点以执行左侧，但 != 不再包装不会发生的 not。新增回归覆盖六种比较、普通／泛型／pure 函数、comptime invocation、左右副作用顺序，以及 return／break／continue 的析构次数。不会放宽可达左侧写入的纯度检查；详见 host-pipeline.md。

泛型有序比较现可接受固定宽度整数、f32／f64、char 的联合类型域，统一证明其属于共享 ScalarLess／LessEqual／Greater／GreaterEqual 操作的域，具体实例仍决定符号与 IEEE 无序行为。此前分别证明“全是整数”或“全是浮点”会误拒绝这些联合域。没有把 bool／str／Symbol／Type 或 opaque normal-form 头加入原生有序域，也没有扩大混合整数／浮点算术的准入；用户 Ord 协议的运算符接线现已由显式绑定完成，见本页开头。13 种标量及动态边界检查的 host 回归见 host-pipeline.md。

机器整数验证现可遍历同一函数中的 FloatBinary／FloatNegate：按源顺序检查操作数、保留写入与控制流，浮点结果仅获得未知值描述，不产生精确整数／多项式事实。原先含整数计数器的泛型浮点循环会报 unsupported-construct，现可运行带默认估计值、+=／*= 和 u8 计数器的 Newton 迭代。浮点操作数内的溢出、除零，以及写入使旧非零依据失效仍拒绝；未实现一般浮点数值契约证明。案例与证据见 host-pipeline.md。

一元取负与 not 的操作数若已检查为 Never，现直接保留该操作数，不为不会发生的运算猜测 Integer／bool 域或生成额外运算节点。返回类型、操作数中的名称与副作用仍在此前的正常检查中核验，程序的 return／break／continue 及清理边界不被抹去。新增源码回归覆盖普通与泛型调用、纯函数、显式协议绑定、comptime invocation、LLVM O0/O3 和每条退出路径的析构次数，详见 host-pipeline.md。

普通泛型浮点操作现接受 requires 已证明的 f32／f64 域：加减乘除、取负、有序比较及上下文浮点文字都保留泛型类型，待实例化后走已有 IEEE 运算。协议绑定与无绑定使用同一内建域判定，浮点 remainder 仍需协议；未知域、混合整数／浮点域和 opaque normal-form 头不据此放行。程序行为分析同时修复标量二元操作的 Never 结果判定：不要求复制不存在的结果，但仍检查两个操作数的行为，提前返回不能掩盖外部写入。普通乘加保持两次舍入，与 math.fma 分开；验证记录见 host-pipeline.md。

显式绑定 Neg 协议后，已证明为有限有符号整数域的泛型取负现与未绑定时采用同一原生操作；不会误查用户 trait 实现。泛型取负的溢出义务、内建 bool／BitVec 的 not、自定义非内建协议结果继续分别检查。builtin/ops 的 Type 注解已改为显式 builtin 引用，使该模块可以作为独立协议包加载；不改变协议身份或输出类型规则。完整 sysroot 仍有未实现依赖：本轮实际加载发现 Option／Result 的 panic、String 与 std 的分配／集合／格式化／I/O 占位入口未解析，不能将独立 ops 包运行成功视为完整 sysroot 已可用。证据见 host-pipeline.md。

泛型固定宽度整数的 `+`／`-`／`*` 和有序比较已接通，除法／余数仅在已证明无符号域时走原生操作。有符号除法仍要求显式舍入协议。泛型上下文整数文字在取得具体域后完成 formation；有限域程序验证检查函数体、前后置条件与默认实参，调用方的已替换契约也能取得文字数值事实。NIR 对泛型二元操作保留 Never 提前返回，拒绝仅伪造 Never 结果注解的节点。`examples/host-generic-integers.fl` 的泛型 GCD 与差值已在 LLVM O0/O3 执行通过；详细证据见 host-pipeline.md。该支持限于已证明的有限内建整数域，不代表任意数值协议、循环归纳证明或所有待定文字定义都已完成。

PTC 的强制原子优先顺序现只在单次 Search 中构建一次，回溯时仍查询当前 assignment；仅保存原子索引顺序，不保存证明事实或活动变量。构建所需数组与工作栈扩展先计费，中途失败不发布部分顺序。完整证明格式和独立检查器未变，低预算失败／恢复以及不同查询间隔离均有回归。本次相关单元、PTC 与泛型源码 389 项通过，见 host-pipeline.md；未重跑全 workspace。

有限类型域的关联组合现按前缀排除：只有类型求解证明某个前缀不可能时才删除，Unknown 保留；64 项上限作用于仍可能的组合，不再先截断完整笛卡尔积。PTC 搜索优先处理布尔结构要求的原子，但仍输出原有 Split 证书，由同一检查器复核。P4 纯度分析同步处理关联类型参数，并复用正向子句中的直接相等前提；or／not 内的等式不成为无条件别名。八个二选一且要求相等的参数只需验证两个组合。源码复现 `repros/correlated-generic-purity.fl` 已通过 NIR 检查（`/tmp/luna-tools-ynabe78k/result.json`），相关纯函数的 LLVM O0/O3 与错误契约拒绝见 host-pipeline.md；原 wrapping-abs 资源证明的较高 CLI fuel 成本不据此宣称已解决。

此前两项前端基线失败均已处理。未知 Source.Missing 的方法路由先确认成员存在，再查实现，报告 unknown trait member；缺少证据的关联投影本身继续正常拒绝。handler 算术的旧测试则错误地把 var 声明等同于捕获值会改变：现有验证器保留从未写入或暴露地址的词法标量事实，对实际写入／暴露者恢复声明域。测试已按此边界修正并补充主体写入、handler 写入和指针别名写入的拒绝例，以及重复请求的解释／LLVM O0/O3 正例；未放宽验证器。当前 elaboration 449、host 585 项全通过（`/tmp/luna-tools-msr7l6b5/result.json`）。这仅解除已知基线失败，不代表下述未完成能力已经实现。

泛型相等／不等比较现在可使用 requires 已证明的有限内建标量域：在 T 仍是 rigid 时，用类型命题验证其属于共享 ScalarEqual 的 bool、char、str、Symbol、固定宽度整数及 f32/f64 域，再保留泛型操作待实例化。不会根据已见调用猜 T，不展开 normal-form 头；Type／Integer／BitVec 等使用其他操作种类的域不在此入口放行。无约束泛型和混入未知名义类型的域仍需相应协议或后续分派支持。

math.wrapping_abs 的公共泛型后置条件已接通：`result == value'old or result == -value'old`，先处理最小整数保持原值的情况。泛型取负仅在 requires 证明有符号固定宽度域时走现有 IntegerNegate；NIR 对当前函数拥有、且操作数与结果一致的开放类型参数保留操作义务。程序验证对所有类型参数均可证明属于有限固定宽度整数域的函数，枚举全部可能组合后逐一验证，最多 64 组并计费；类型关系不可能的组合仅在 PTC 证明后排除，值／内存规格仍不进入 PTC。未知域保持原分析路径，普通 runtime 依赖、任意协议数值操作和无限域的通用符号证明尚未由此实现。`repros/generic-wrapping-abs-contract.fl` 已生成 LLVM（`/tmp/luna-tools-joeeq8_p/result.json`）；五位宽原生回归及跨包调用见 host-pipeline.md。跨包资源检查示例需显式较大的共享 --fuel，默认预算未改；证明成本优化仍待继续。

host-number-theory 暴露的复合条件非零事实丢失现已修复：短路合流后分别检查两侧谓词的版本，只用新鲜未知 Bool 替代不可保留的一侧；另一侧仍可约束 and 成功／or 失败路径，不把单侧条件当成整个结果。右侧写入使观察过期时继续丢弃对应事实，原运行时求值次数和短路行为不变。示例已恢复复合 while 条件。仍未从 `divisor <= value / divisor` 自动证明递增不溢出，累加／递归结果的界也显式给出 checked assumes；不宣称循环不变量或归纳证明已完成。这些属于程序验证层，不交给 PTC。验证记录见 host-pipeline.md。

本次切换回 LLVM host 示例交付，暂不继续扩展语义前端。resolve 已补充可恢复的 NameResolutionPlan：待定 pattern 输出必须区分 Pending 与 Missing，并按由内到外的词法顺序查询；Pending 不能直接回退外层同名变量。计划保留原有权限／未解析错误，失败后可重试，且有 S-expression Display。该机制尚未接入 HIR lowering 与 elaboration 的动态输出作用域，不能据此声称任意 Type 工厂的无注解 pattern 输出已支持。`repros/comptime-factory-pattern-binding.fl` 是续接用例；恢复时须同时处理遮蔽、作用域退出、输出类别和调用环境，不能扫描工厂函数体或展开 normal-form 头猜名字。本次 `test -p resolve --no-fail-fast` 的 21 项测试通过，报告 `/tmp/luna-tools-fzxlu55j/result.json`。

无注解 pattern 方法结果现在可从源码中结构明确的容器结果导出名字：块、if／when／match 及显式泛型容器。lowering 对每个可能容器逐一查使用位置的可访问成员，并要求输出名称集合相同；仅导出词法名字，不假定输出类型相同。CMTT 类别提示同样要求可能成员的代码类别一致，invocation 仍重新选择／验证实际方法。原 `repros/comptime-computed-pattern-binding.fl` 已转为 bool true。任意 Type 工厂调用的结果仍不在该结构分析内，新的 `repros/comptime-factory-pattern-binding.fl` 仍在 lowering 报 value 未解析；不展开函数体或 normal-form 头来猜名字。

计算 Type 接收者上的 CMTT 代码方法现可 invocation：名义方法及显式 trait 方法先检查主语域，Type 主语不再当成值 receiver 传入。表达式代码、带显式输出契约的 pattern 代码，以及 quote fn 普通实参恰好一次已有解释与 LLVM 回归。仍未完成的具体缺口是无注解 pattern 输出绑定：`repros/comptime-computed-pattern-binding.fl` 在 AST lowering 的名称建立阶段无法导出 value；`pattern_expression_leaf_outputs` 目前只从已解析成员／显式 trait 契约取得输出名字。不能把显式契约用例通过等同于该推导已完成。

计算得到的 Type 接收者现可选择并调用静态函数：块／if／when／match、Type 工厂及返回记录的 Type 字段先通过域检查与完整类型求值确定主语，再走原有 selected_method、权限和调用检查，不把 Type 值当作 runtime self 传入。泛型函数、函数值、comptime 静态函数、不同容器实例和普通实参恰好一次的回归见 host-pipeline.md。`repros/comptime-computed-static-method.fl` 保存原缺口；不据此宣称全部 quote 方法或动态 Type 调用已完成。

块／if／when／match 直接返回 Type 时，现可选择嵌套泛型类型和成员常量，不必先包进记录字段。分类阶段仅取得 Type 域；类型位置完成身份，成员表达式也先完成整个 comptime Type 计算，再取得名义容器实参，不要求最终 NIR 已经是 Type 字面值。成员常量进入 runtime 仍要求绑定 Splice。docs/type/index.md 的阶段规则保持有效：runtime 局部状态不能形成 Type；接收者内部的 comptime 局部赋值和分支则可参与计算。`repros/comptime-composite-type-member.fl` 及正反例验证见 host-pipeline.md。

块／if／when／match 的结果字段现可继续选择嵌套类型成员，例如 `do {packet()}.selected.Inner<bool>.Payload`。域探测复用不提交的 checking_attempt；临时局部、捕获、控制流和观察证据恢复原上下文，接收者的执行仍交给最终表达式处理。普通 runtime 数据投影在 comptime 探测不适用时继续原有检查路径，副作用顺序已有解释和 LLVM 对照。runtime 分支条件即使两臂返回同一容器也不能因此进入 required 类型计算。详见 host-pipeline.md；这不等于未知运行值的 Type 身份已经可用。

泛型 comptime 工厂的返回字段域现先完成调用实例化：`packet(value:T)->Packet<T>` 的调用 `packet(Outer)` 能取得 `Packet<Type>`，再从 selected 字段选择 `Inner<bool>.Payload`。返回域含 rigid 参数时复用 required 准备过程，使用现有实参／默认值推断所得 result_type；准备时不 invocation 工厂，实际 Type 身份仍由完整 required computation 产生。显式泛型、扩展位置调用、optional 默认及命名覆盖、函数实例别名、不同调用的容器身份有回归。此修复不等于未知 ordinary 参数下任意返回对象的部分值分析完成。

完整调用返回的 Type 字段现可继续选择嵌套泛型类型成员，例如 `packet().selected.Inner<bool>.Payload`。调用签名仅提供字段路径的声明域；实际 Type 身份仍通过 required computation 取得，并沿用使用位置的成员候选／可见性和类型实参检查。普通 runtime 函数不因返回域含 Type 字段而变成 comptime 函数。`repros/comptime-returned-type-member.fl` 保存原问题；详见 host-pipeline.md。本轮不宣称一般未知调用结果、任意块根或活动存储的部分 Type 身份已完成。

计算接收者上的 `vars.Closure` 已按结果域消歧：先准备 required computation；数据结果留给完整字段路径求值，只有 Type 结果才在此 invocation 并检查是否为真正的 Type metatype。`packet().vars.Closure` 不再被强制要求 packet() 返回 Type；`Meta().vars.Closure`、块结果和泛型嵌套字段均有解释及 LLVM O0/O3 回归。这里不把 result_type 等于 Type 当作返回值就是 Type，也不展开 Identity<Type>。先前记录的非局部计算接收者歧义已由本轮修复；未知普通参数下工厂结果的部分字段推导仍未完成。

普通字段投影现可作为 required comptime 类型表达式：`packet().selected`、块／条件结果及嵌套泛型记录字段由完整计算取得 Type 身份，而非只依据字段声明为 Type。`repros/comptime-returned-type-field.fl` 从原失败转为解释与 LLVM O0/O3 通过。运行时函数／输入、非 Type 字段和错误 normal-form 身份继续拒绝。状态闭包在同一 required 计算中的连续调用保持环境变化。此路径要求计算能够完成；未补齐未知普通参数下任意工厂返回值的部分字段推导，特殊 `vars.Closure` 路径对非局部计算接收者的消歧也仍需继续处理。

普通数据字段 `packet.vars.Closure` 不再仅因拼写被误认成 metatype 的上下文类型。先以局部绑定及字段的声明域区分数据路径与 Type 路径，实际投影仍执行原有类型和成员检查；泛型字段、别名及 Type 字段上的真正 `vars.Closure` 均有回归。`repros/comptime-record-meta-field-names.fl` 已通过解释与 LLVM O0/O3。当前限定为局部根的数据字段路径，并未宣称任意调用结果的 metatype 消歧已完成；验证见 host-pipeline.md。

部分聚合的字段投影现保留已知信息，包括子记录别名、嵌套字段和直接类型注解 `saved.packet.selected.Inner<bool>.Payload`。数据路径先走普通表达式投影检查，再按已检查的声明／字段位置读取观察描述；只探查已观察局部值的字段路径，不调用工厂或读取指针。对一个副本的 Type 字段写入后，该副本的旧观察失效，先前保存的 Type 和另一份值副本保持原值。`repros/comptime-partial-field-projection.fl` 已解释／LLVM 回归通过，见 host-pipeline.md；一般调用结果、未知内存和自定义 view 的推导仍未完成。

tuple_enumerate 现保留已知元素字段的 Type 身份，支持直接输入及部分已知 const 别名。观察信息与普通元素存储分离：不加入 specialization_inputs，不替换可执行局部读取；取址、写入或捕获使观察失效，required computation 中也执行该失效规则而不改变原有符号参数规则。直接 T 注解、经 const packet 恢复 Selected、两轮不同 Type、输入恰好一次、Type 存储写入和按值闭包捕获均有解释／LLVM 回归。`repros/comptime-type-packet-enumeration.fl` 已由原失败转为 bool true，见 host-pipeline.md。一般工厂返回值与未知存储读取的字段身份仍未由这条结构观察路径补齐。

部分已知 const 聚合现在可经过简单别名链、as 整体绑定及中间子聚合解构，保留已知字段与构造标签。专用分析描述只含已知 Term、聚合结构和 Unknown；不保存运行中存储、闭包环境或重放初始化表达式。块与 inline 循环清除离域描述且保留失效，开放函数／候选事务／generator 恢复独立上下文。`repros/comptime-partial-packet-alias.fl` 已由原失败转为解释与 LLVM 回归通过；原变量／指针写入后读取旧 packet、有状态 initializer、范围循环中变化的 Type 身份均有检查。当时另存的 `repros/comptime-type-packet-enumeration.fl` 后续修复见上段；普通工厂返回值、一般控制流及自定义 pattern view 的部分字段分析仍未完成。

部分已知字段证据现覆盖具名 struct 和 enum 的直接构造：字段声明身份、枚举标签及已知字面量用于辨别可能成功的模式候选，未知字段与未知 guard 不用于排除候选。or 对可能候选的证据取交集，原构造与匹配仍执行。泛型字段默认值、嵌套枚举、改变字段书写顺序及有状态初始化已有解释／LLVM O0/O3 回归，详见 host-pipeline.md 的记录与枚举小节。任意工厂调用结果及自定义 pattern view 仍未据此完成；独立部分聚合别名的后续支持见上段。

普通 comptime 调用中的部分已知 const Tuple 现在逐字段保留证据。`const (Selected,value)=(Outer,flag)` 中，即使普通参数 flag 尚未求值，Selected 仍可用于类型注解及成员选择；flag 保持调用期值，不进入静态实例缓存。嵌套 Tuple、as、guard 和 or 模式有解释／LLVM O0/O3 回归；or 只保留各候选共有的身份，未知整个 packet 不被补成虚构常量。具名局部函数的普通参数捕获仍需要独立调用期环境，未在此实现。见 `repros/comptime-partial-type-packet.fl` 及 host-pipeline.md 的部分已知 Tuple 小节。

带 guard／发散 else 的分支内 Tuple 别名现在能保留成功路径的成员来源。原模式及 guard 仍执行：状态闭包回归验证恰好一次调用，反转 guard 的复现进入 `while true {}` 的失败分支并耗尽预算。类型错误、非发散 else 和失败分支访问成功绑定仍拒绝。`repros/staged-member-guarded-tuple-alias.fl` 已通过解释和 LLVM O0/O3；嵌套 const initializer 中的词法 return 涉及尚待作者确定的跨阶段控制契约，见 `repros/staged-member-guarded-return.fl`。当前诊断已从误导性的 return outside a function 改为 return across a required comptime boundary is not supported；拒绝行为未改变，也不据此规定语言禁止跨阶段返回。这不同于已支持的 runtime 正文中直接 const 绑定失败后 return；本轮没有把该拒绝改为语言规定。完整 33 项目录基线来自前一轮，本轮仅重验受影响测试与 type-packets 示例。

分支内的 const Tuple 解构现在保留已知类型身份：支持嵌套 Tuple、容器别名链及 as 绑定，从解构结果继续选择泛型成员。描述阶段沿已检查 Tuple 语法恢复来源，具体阶段通过结构检查已有的常量数据记录绑定值；不构造替代 Tuple normal-form 的结构类型。该记录只用于 const：普通 let 在 comptime 中仍有可取址、可捕获的存储身份。未知字段不能被伪造为已知值，任意自定义 pattern view／可变数据流也未据此完成。新示例 type-packets 与回归证据见 host-pipeline.md；最新集成验收已覆盖完整 33 项目录：O0/O3 共 66 次构建及原生执行通过，开发工具 37 项通过。命令与原始日志复核见 host-pipeline.md 当前基线，替代此前 32 项目录证据。

条件结果的成员来源已接通：if／when／match、结果块及分支内不可变简单别名可以组成待定成员别名，具体成员结果与类型注解仍在实例化时检查。分析要求所有结果分支都有来源，不能只因条件来自 comptime 就允许无关 Any 的字段访问。分支内别名通过 HIR 绑定身份回溯初始化式，兄弟作用域不按名字合并；共享别名子图按节点和 selection 状态去重并计费。可变绑定的来源传播尚未据此扩展；Tuple 解构及 guard／else 成功路径的后续支持见上段。原始失败与 LLVM／解释回归见 host-pipeline.md。

开放描述的 Rust 栈溢出已修复：绑定检查拆成独立栈帧，连续一元运算描述使用迭代过程。直接 NIR 的 64 层 not 也已修复：原失败来自 Interpreter::new 对宿主递归深度的 min(64) 保护，因而传入 depth=256 仍会失败。现将连续 Not 的执行改为显式待处理栈，保留每节点步数、BitVec 域检查、控制转移和 delivery／cleanup 边界；不增加宿主递归上限或默认预算。源码 32／63／64 层直接解释通过，低层测试覆盖 96 层、预算耗尽后的恢复及中间域错误。复现保留在 repros/deep-open-unary-interpreter.fl，历史拒绝断言已改为实际结果断言。其他算子／调用的递归限制仍保留；这不是整个解释器已经无递归限制的承诺。验证证据见 host-pipeline.md 的“连续 Not 的直接 NIR 执行”。

待定容器的参数化成员支持直接 `Container.Inner<T>.Payload`、命名参数覆盖、混合 Type／值参数和中间别名。未注解中间别名缺口现已通过描述层来源记录修复：来源沿不可变别名传递，不能据此假定结果是 Type；作用域、分支、开放函数和 inline 循环分别保存／恢复。`repros/staged-generic-member-inferred-alias.fl` 已解释执行及 LLVM O0/O3 通过。错误域、非 Type 结果、私有访问和局部遮蔽有拒绝对照。

阶段函数即使没有局部 const／inline 控制流，组合注解或值表达式中的显式 comptime Type 参数成员访问也会触发开放函数检查。扫描不再只识别 let 的顶层成员注解。指针、回调参数／结果类型及经显式 Splice 的成员常量均有 O0/O3 验证，错误回调域、跨模块私有成员、缺失 Splice 仍拒绝。复现 `repros/staged-parameter-composite-annotation.fl` 已通过；四组回归 1,654 项通过，见 host-pipeline.md。

待定成员／别名现可组成指针与 Tuple 局部注解：定义端扫描构造内部的待定输入，检查构造表达式的域，但保留完整源注解供实例化核对；初值已有的指针／Tuple 结构不会被待定注解抹去。参数化指针读取仅描述结果域，具体读取、Copy 与来源检查仍在实例完成。非 Copy 名义实例的拒绝有可复制实例对照；四组回归 1,652 项通过，证据见 host-pipeline.md。尚不据此宣称所有任意形态的待定类型表达式都已覆盖。

待定 Type 成员经过普通局部 const 别名、别名链或显式 `const Alias: Type` 后，也能用作局部类型注解。定义端保留待定描述，不能把显式 Type 域误作已完成类型身份；具体实例继续检查成员结果确为 Type、注解与初值匹配，以及 normal-form 身份不展开。`repros/staged-type-factory-alias.fl` 已由失败转为解释执行和 LLVM O0/O3 通过；本次四组回归 1,650 项通过，证据见 host-pipeline.md。

阶段 Type factory 的成员现可用于局部变量类型注解，包括嵌套成员链与显式 comptime Type 参数接收者。局部注解保留源表达式，在实例给出容器身份后完成检查；函数签名虽已有稳定符号域、但体内成员注解仍依赖阶段参数时，也进入开放函数检查。bool/u32 实例、错误类型、跨模块私有成员、非 Type 成员及 normal-form 身份拒绝已有回归。`repros/staged-type-factory-annotation.fl` 已解释执行与 LLVM O0/O3 通过，详细证据见 host-pipeline.md。

阶段 Type factory 的成员选择已接通：`const Container=Make(flag); const result=Container.FLAG;` 在定义端仅有 Type 域、尚无容器身份时保留普通未知描述，具体实例重新执行成员、可见性、域与阶段检查。明确 Type 根的嵌套链也可延迟；接收者域本身尚未建立的任意字段访问仍拒绝。`repros/staged-type-factory-member.fl` 从原字段投影错误转为解释执行及 LLVM O0/O3 通过（本轮解释器 `/tmp/luna-tools-03rf2gzs/result.json`）；四组回归 1,645 项通过，详细证据见 host-pipeline.md。没有推测 Make 的函数体或展开 normal-form 头。

局部 Type 的成员路径已支持嵌套类型和内部自有参数，注解与 Type 常量表达式均有 O0/O3 验证。每层沿用使用位置可见性，普通数据字段链不进入类型成员路径。相关四组回归 1,159 项通过；命令和报告见 host-pipeline.md 的“局部 Type 的嵌套成员”。这未补齐未知 Type 主语的通用推导。

嵌套成员分类求出的接收者类型现在在同一次成员选择中复用，避免再次递归检查同一前缀。原先 20 层局部成员链在默认预算报 `constant copy budget exhausted`；修复不增加预算、不跨环境缓存，也不省略表达式容器的求值准备代码。复现与回归证据见 host-pipeline.md 的深层成员链小节。

此前阶段成员、组合注解和函数体依赖识别改动后的历史 host 示例基线：32 个示例 O0/O3 共 64 次构建与原生执行通过，开发工具 37 项通过（`/tmp/luna-tools-jpm_ec8s/result.json`、`/tmp/luna-tools-sj0minxe/result.json`）。独立核对当前目录的完整性、协议选项、所有构建／执行退出状态与精确 stdout，结果见 `/tmp/luna-host-examples-swmownrw/audit.json`。这替代此前完整目录集成证据，下述前端与 ABI 缺口仍未完成。

runtime 数据 const 的模式／guard／else 已接通 parser、AST 四子节点布局、lowering、编译期匹配和 host 控制流。成功绑定由一次匹配交付；失败返回原 runtime else，保留 return／break／continue 及清理。定义端不提前匹配尚未提供的阶段输入。quoted code 与 generator 数据绑定仍走原来的独立路径，本次没有放宽其条件绑定限制。详细证据见 host-pipeline.md 的局部 const 小节。 阶段函数的条件 const 现在还会从输入结构保留已知 Type／类型列表证据，供 guard 和后续注解使用；没有执行类型函数或把未知的 normal-form 关系变成不等。 已知 normal-form 主语的条件 const 还会传递头与参数结构，解构出前序 Type 和依赖该 Type 的值；定义探测保留同一已检查输入，嵌套 normal-form／Tuple 类型模式的 Type 输出也可用于后续注解。未知主语的通用依赖模式推导仍待补齐。

递归节点的局部循环已可安全遍历和反转链接，读取单个字段的具名 helper 可推导有限内存契约，见 `examples/host-linked-list.fl`。递归函数 `enable(node)` 读取 `node.next` 并递归调用时，仍在契约准备报 `every callable candidate requires a memory contract: memory target set budget exhausted`；复现 `repros/host-recursive-node-memory-contract.fl`，最初报告 `/tmp/luna-tools-0t0ltze0/result.json`。补充诊断后的复验 `/tmp/luna-tools-85rd205f/result.json` 明确定位到 `enable` 的同一个实例，失败前摘要已累计 80 个访问项（discovered instances: 1）；当前表示持续展开有限路径列表，尚没有递归访问摘要。这不是 LLVM 递归布局失败，也不是语言禁止递归遍历；后续需要能够表达和求解沿递归结构的内存访问摘要，不能靠增加预算或省略资源检查宣称修复。

LLVM 已支持经数据指针形成的递归布局：指针表示不再展开其目标类型，读写／字段投影在访问点从已检查 IIR 类型获取布局。`examples/host-recursive-data.fl` 展示 Node 与 Link 互相引用，以及从枚举载荷取出指针后写回；复制节点与递归节点析构也有 O0/O3 验收。逻辑类型与 Copy/Drop、来源检查保持原契约，按值递归仍不能获得有限布局。详见 host-pipeline.md。

普通 const 的 Type 别名在阶段函数定义端也已保留已知证据；局部 Type 接收者可从原 use-site 候选表选择名义容器的成员类型与常量，私有访问不扩大。头已知但含尚未提供值输入的容器保留检查后的构造证据，复制时只改内部绑定，外部阶段输入保持词法身份。runtime 输入仍不能据此进入 comptime；struct／enum／newtype 及 normal-form 身份、成员类型、值参数成员常量和拒绝路径已有 O0/O3 与相关回归。没有新增源级名义类型模式语法，未知 Type 主语的通用依赖模式问题仍在。

函数 ABI 属性仍待通用 comptime Object elaboration／invocation 接通。本轮核查确认依赖顺序仍是 Object 的值与类型支持、按词法绑定求值并合并属性、ABI 进入函数类型／调用与间接函数值检查，之后才是 LLVM 外部符号与调用约定。C 边界还须检查空效应行；不能只改 LLVM 调用指令或识别 attrs.abi 拼写。定义、参数、字段、variant 与 configuration 现在保留实际 HIR 表达式，已知名称依普通词法作用域绑定，未知名称保留待处理节点；保留原始文本用于诊断，不根据 `attrs.abi` 拼写直接授权 C ABI。成员属性目标使用真实 HirId，增加表达式节点后仍保持目标与应用次序。该层尚未完成 Object 类型检查和属性消费，生成 checked NIR 的拒绝边界保留。2026-10-24 在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 600 -- test -p luna --test frontend_cli`，197 项通过；报告 `/tmp/luna-tools-c9c8jev6/result.json`。覆盖已知函数与前置参数绑定、未知名称保留、成员目标、顺序及转义稳定输出。

块内定义的属性链也已接入上述保留路径：只在分类时穿透包装，实际 lowering 使用完整包装链，防止丢失属性或把嵌套属性误作表达式。回归覆盖内外属性次序、真实定义目标、外层函数参数引用和被修饰函数的同名参数不会遮蔽属性中的外层函数名。非定义语句的属性求值尚未因此接通。2026-10-24 在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 600 -- test -p luna --test frontend_cli`，198 项通过，退出 0（`/tmp/luna-tools-6wpi8fo0/result.json`）；全 workspace 格式化退出 0（`/tmp/luna-tools-cbf_5j8h/result.json`）。dev 未提交。

块内声明已进一步接通 resolver→HIR→NIR→LLVM：普通函数和源级 test 主体的各源码块拥有独立静态作用域，声明身份与顺序变量的使用解析分别处理。内层声明、顺序绑定按词法层级查找；已支持函数修饰符不再触发统一的 block pending 标记。声明本身生成 unit，实际定义仍由包级检查验证，未使用错误函数也拒绝。局部函数、类型别名、递归及兄弟块遮蔽已有解释器/O0/O3 验收，详见 host-pipeline.md。quote 表达式内的生成声明及 template 实例生成作用域仍需独立处理；通用属性 Object 求值缺口不变。

局部函数现在隐式继承外层子句参数，固定前缀进入 NIR 函数实例身份；多层声明、函数值返回、自己的显式／默认参数以及外层类型要求已验证。外层普通参数列表中显式 `comptime` 参数也已接入：resolver 分配稳定声明身份，HIR 保留绑定映射，局部实例固定继承类型和值输入。父函数的类型要求和 trait bound 重新绑定到局部输入；`const` 初始化的阶段上下文与函数体特化分开处理。形成域依赖于前序阶段参数的情况已支持；尚未形成的域、依赖 runtime 输入的域，以及 comptime 函数中没有显式 comptime 标记的普通参数捕获仍需后续处理。完整验证与 guard/as AST 元数据修复见 host-pipeline.md。

局部阶段环境在递归和兄弟调用中已改为引用当前声明的固定输入身份，避免复用父函数 Bound 槽位；相关函数值与泛型递归已有解释器/O0/O3 回归。普通 comptime 函数的未显式标记参数捕获仍失败，保存为 `repros/comptime-local-ordinary-input.fl`（`/tmp/luna-tools-3lhkd_7_/result.json`，`local binding is unavailable in this scope`）。需继续补调用期环境表示；不能据此改写普通参数的阶段分类。

普通 comptime 参数捕获的进一步定位：NIR `Function` 没有具名词法环境字段；解释器 `Value::Function` 只保留声明身份，`FunctionInstance` 的缓存键只由声明和静态参数构成。普通调用值不能作为该缓存的隐含依赖，也不能通过把普通参数的 Sort 改成 ComptimeValue 来修复。现已在实际局部查找失败之后，沿 HIR lexical parent 和原 HirId 识别这种缺口，明确诊断 `capturing an ordinary comptime function parameter in a local declaration requires an invocation environment, which is not yet implemented`。已有 locals/lambda 环境优先，未改变普通参数分类或签名。

后续实现需贯通具名函数的环境需求、函数值构造、递归／兄弟引用及调用绑定；静态函数实例和每次 invocation 的环境必须分离。可复用闭包的实际环境值与快照检查，但必须同步处理参数域、前后置条件、环境共享、指针生命周期和资源责任。仅把外层 locals 加进 checker 会留下无绑定的 NIR；仅把环境塞进解释器会漏掉 phase/resource 检查；只内联局部函数则不能交付递归和返回函数值的完整行为。这些是实现约束，尚未据此宣称具名捕获可执行。

本轮回归用例明确标为临时实现缺口：直接捕获、多层捕获、未使用局部函数均检查诊断和失败产物保护；lambda 捕获与同名显式参数转发各在 LLVM O0/O3 验证，返回结果同时覆盖 true/false。工作目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 600 -- test -p luna --test host ordinary_comptime_` 2 项通过，退出 0（`/tmp/luna-tools-81fz5clu/result.json`）。这些通过证明诊断和既有路径，不能证明缺失特性已经实现。 `test -p luna --test host lexical` 经同一入口执行也全部通过（`/tmp/luna-tools-gst5ddkb/result.json`）。全 workspace 格式化通过（`/tmp/luna-tools-6oijcob1/result.json`），`git diff --check` 通过；dev 未提交。

整数 `min/max` intrinsic 已接通十种固定位宽 signed/unsigned 类型，泛型输入域仍须由类型约束证明。程序流分析分别对区间上下端取 min/max，单点结果形成常量事实，并为 min 保留结果不大于两个实参、为 max 保留结果不小于两个实参的关系，同时保留“结果等于其中一个实参”的析取。实参可能被后续求值写入时先取得不可变值快照；常量与确定不变的局部值复用身份。对外契约用 `a'old` 表达调用入口值，关系可经现有复制和返回协议导出。条件匹配只使用当前版本的精确相等证据，不用区间巧合替换值；分支重放保留不等事实以排除不可能选项。这些是程序验证事实，不进入 PTC。预算回归已有缓解：相等关系按变量对索引双向证据，残余契约在清理足迹与依赖收敛后统一检查；关系调用组合恢复到 1M 通过，析取／分支组合在 2M 通过、1M 仍耗尽。编译器默认预算未提高，尚不能说性能问题全部解除。

固定位宽 signed/unsigned 饱和加减已通过泛型 math 包接入 host。注册与实例化检查保留符号／位宽；程序 flow 使用实参求值时取得的区间快照，在数学整数域计算加减端点，再钳制到结果类型边界。单点区间可形成常量事实，支持完全饱和及未饱和结果的范围契约。该分析属于程序验证层，不进入 PTC；尚不建立结果与输入之间的一般关系式，不能将饱和加减直接等同于普通加减。缺失范围时保留类型范围，预算耗尽仍报告 Unknown。

浮点分类 intrinsic 已支持 f32/f64 输入、bool 输出：类型证据改为分别核验操作数域和结果，泛型输入域须由类型约束证明。isnan/isinf/isfinite 可通过泛型 math 包原生执行；这不补齐一般浮点前后置条件的静态证明。

捕获式效应的入口尚未接通：`compiler/elaboration/src/program/effects/handlers.rs` 的 handler lowering 只接受 Direct／DirectPassing，NIR 的 EffectResume 目前表达直接 handler 边界，未提供可拥有的 continuation 值及其栈池来源。后续需要同时补充捕获、单次恢复／放弃和资源清理接口，才能对接 host 栈运行时；不能仅取消 LLVM ABI 拒绝分支。

混合契约链的冗余证明成本已降低：`repros/host-intrinsic-mixed-contract-chain.fl` 在旧实现的 1M fuel 下源码分析失败（`/tmp/luna-tools-ou77chx7/result.json`），当前同预算可 O0/O3 构建并运行（`/tmp/luna-tools-7wmojrdy/result.json`）。同 lineage 且所有源版本仍有效的复制不再执行冗余差分投影；临时差分图也不再纳入已被区间蕴含的边。原始关系仍保留供 join／区间放宽后使用。默认预算没有提高；该大例在默认 100k 总预算下仍于残余契约分析耗尽（`/tmp/luna-tools-sfzz7u88/result.json`），不能将本次改进说成预算问题全部解除。

整数 intrinsic 的静态范围／常量契约已有支持：位计数可推得 0..width，非零输入可加强边界；bitreverse、bswap、ctpop、ctlz、cttz、fshl、fshr 的已知整数输入可得精确结果。这些事实进入普通程序流分析，用于前后置条件，不调用目标解释器、不进入 PTC。`examples/host-intrinsic-contract.fl` 已原生执行。一般 intrinsic 重复计算的跨调用等价证据尚未实现，浮点 FMA 条件仍属于下面所述缺口。

预算观察：把 `native_integer_intrinsic_contract_ranges_and_exact_results` 的常量、范围及非零输入用例串成同一 run 时，残余／源码契约分析分别发生 BudgetExhausted（`/tmp/luna-tools-f_zk5exp/result.json`、`/tmp/luna-tools-lxzgxs5u/result.json`）。独立 test 入口逐项验证通过，未扩大编译器默认预算。简化后的 20 个常量计数调用链 `repros/host-intrinsic-contract-chain.fl` 能生成 LLVM（`/tmp/luna-tools-2ash9d7s/result.json`），所以它是缩减对照，不是仍失败的复现；混合范围契约的冗余投影／闭包成本已按上文修复，默认总预算限制仍保留。

规格表达式已接通具有目标证据的标量 intrinsic：checkable_predicate 复核 tag、类型、元数和所有实参；资源观察复核具体证据，unsafe scope 也按原观察规则检查其内容。模板证据只准入结构，不授权未特化执行。真实 assumes 示例见 `examples/host-intrinsic-predicate.fl`。这不解除下面记录的一般泛型 math 调用限制。

含 intrinsic 的 requires/ensures 仍需流分析表达能力：直接 FMA 前置条件通过结构检查后，函数分析仍以 `unsupported-construct` 拒绝（`/tmp/luna-tools-3mzcrcb6/result.json` 中 preconditions 测试）。当前入口 preconditions 要求可转换为流分析 Condition；目标数学结果只有 unknown value，尚无可保留、替换和重放的条件证据。不能将 unknown 当真、跳过调用前证明，或将它送入类型 PTC。本轮没有放宽该层检查。

math 原生库示例仍需先调用再检查结果：直接将泛型 `math.bit_reverse` 等调用嵌入 assumes，会在多包 elaboration 报 `assumes requires a supported effect-free program predicate; calls and type propositions need further contract elaboration`。失败报告 `/tmp/luna-tools-zwd7elb3/result.json`；示例现先将调用结果绑定到局部，再保留原值断言，6 项原生测试通过 `/tmp/luna-tools-z4ztssqk/result.json`。这没有实现任意 pure 调用在程序规格中的展开，相关前端契约工作仍待补齐。

computed pattern 观察临时值的可失败清理仍缺失败路径资源转移：嵌套 newtype 的底层 Item 析构若包含 `assumes self.ready.*`，源码阶段拒绝 `observation cleanup needs failure-path resource transfer`。这不是 LLVM newtype 布局缺口；普通局部／闭包／入口错误载荷的嵌套析构已经可执行。复现及拒绝保护见 `compiler/luna/tests/host.rs::native_computed_newtype_candidates_retire_nested_payloads`，其中无失败析构版本对照解释器和 LLVM O0/O3，并检查三层析构调用生成；可失败版本保留旧产物。本项仍待前端补齐，不能通过跳过观察清理或放松权限检查解决。

开发重心转向可执行 host 示例；本文件后面的记录按历史阶段阅读，旧的失败或 gate 描述可能已被后续条目解除。当前源码到具体 IIR、LLVM 降码、原生产物生成与执行已有贯通入口，不再以完成整个语义前端作为运行示例的前置条件。

保留到后续前端工作的具体缺口：operation 参数的私有借用根在 handler 词法返回中尚需请求侧来源替换；不能把它当成安装方捕获来源直接导出。结果含指针的 match 后置条件并非都能自动证明，亦不能通过把 assumes 嵌进不支持的 predicate 片段绕过检查。已有安装方捕获指针返回、条件载荷来源与析构的支持不代表上述两项完成。

操作参数返回的源码复现：`repros/host-handler-parameter-return.fl`。未解引用的参数不一定进入 direct_handler_accesses 的访问集合，因而旧 handler 分析可能为其词法返回发布空 origins，`--analyze-resources forward` 曾成功；根调用者直到解引用返回值才拒绝。现新增返回布局完整性检查：所有可能存在的指针叶必须有非空来源，已由 variant domains 排除的载荷不要求来源。缺失时在 handler 返回处报 `handler pointer return lacks request-side provenance evidence`，不再发布误导性的空证据。

完整支持仍需要把操作参数返回与正常 completion 分开建模，在请求实参完成求值后映射来源，并经具名 callee 的非局部退出摘要传回 installer；不能仅移除私有借用根检查。当前底层解释器可执行该复现，但 LLVM 数据指针要求 Copy/Drop 检查，不能通过省略协议测试其原生 ABI。`test -p luna --test host handler` 40 项通过（`/tmp/luna-tools-e_wl6gum/result.json`）；扩充后的 `checked_handler_parameter_pointer_return` 矩阵覆盖直接／跨函数请求、裸指针／record 载荷，定向退出 0（`/tmp/luna-tools-_nomhl0k/result.json`）。测试明确记录当前能力缺口，不将其固化为语言拒绝规则。

后端待扩展的独立边界包括 capturing/async ABI、仿射 handler 捕获，以及完整 sysroot 的字符串、分配与 IO 接线。继续保持错误结果与代数效应 ABI 分离、normal-form 头不展开、PTC 仅处理类型命题；不以放宽这些契约换取示例运行。

本轮在 `/home/fin/luna` 运行 `python3 -B development/host_examples.py --opt-level 0`，退出 0，17 个示例均完成 LLVM 构建、原生执行与输出核对，报告 `/tmp/luna-host-examples-9tdj9537/summary.json`。这验证可执行子集，不表示完整语言实现；未重复全部前端测试。运行方法见 [README.md](README.md)，实现边界见 [host-pipeline.md](host-pipeline.md)。工作区继续保留在 dev，未提交。

## Trait contract 实例化回归已修复

`invoke_instance` 在调用及失败清理结束后，通过 IIR expansion source 表还原对外错误的源码位置；内部事件和 cleanup site 保留生成节点身份。先前 call(255) 的两条执行路径均正确拒绝，但对外暴露的位置一个为生成身份、一个为源码范围，现已一致。实例化回归同时核验拒绝事件原始身份能够追溯到对外错误位置。

`cmtt::merge_entry_contract` 不再因接口谓词与实现中的继承谓词相同而删除接口检查。普通方法调用有接口及实现两个边界；内联保留同样的入口／退出检查，已有实现的弱化入口保护与入口快照保持不变。原检查次数 4/5 不一致的用例已通过，覆盖单／双前提、参数在函数体内修改以及显式 return。

工作目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 300 -- test -p luna --test instantiation`：56 项全部通过，退出 0（`/tmp/luna-tools-phzn4eo7/result.json`）。此前列出的两项 trait contract 失败均已解除。 `test -p luna --test host contract` 6 项通过（`/tmp/luna-tools-fggwukc5/result.json`），`test -p luna --test inline_returns` 全组通过（`/tmp/luna-tools-pt5shqvy/result.json`），均退出 0。格式化退出 0（`/tmp/luna-tools-buf_xr86/result.json`），git diff --check 通过；dev 未提交。

## Staging 递归栈溢出已修复

`fold_input` 中 pending integer/float 的形成代码拥有较大的临时值，增加了所有递归分发层的宿主栈帧。将它提取到禁止内联的 `fold_pending_literal` 后，原来的深层 scalar comparison/block/discard 测试可正常到达逻辑深度检查。没有降低 128 的逻辑上限、增大测试线程栈或过滤失败用例。回归现覆盖深度 16、64、127 的成功，128、160 的诊断，以及失败后的正常求值恢复。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p iir`：80 项通过，退出 0，无过滤（`/tmp/luna-tools-dbgf0eal/result.json`）。`test -p luna --test host staged` 的原生集成检查也退出 0（`/tmp/luna-tools-yhdh4_j8/result.json`）。此前记录的该 staging 栈溢出已解除；两项 trait contract 实例化测试失败仍待处理。格式化退出 0（`/tmp/luna-tools-rb_u_8gl/result.json`），git diff --check 通过。dev 未提交。

## Host 集成检查暴露的待修复项

本轮执行 `test -p luna --test effect_contracts --test instantiation`：效应契约 5 项通过，实例化 54 项通过、2 项失败，报告 `/tmp/luna-tools-65l1a1ko/result.json`。`concrete_contract_execution_matches_normalized_execution` 在 call(255) 两侧均拒绝，但 IIR 错误源范围为占位范围，NIR 为 657..677；`inline_weakened_trait_contracts_preserve_entry_guards_and_check_counts` 的 predicate 次数 4 与 5 不一致。没有修改期望来消除失败，也未确认其引入版本。

`test -p iir` 的 `staging::tests::nested_scalar_comparisons_and_blocks_reach_logical_depth_limits` 发生宿主栈溢出（`/tmp/luna-tools-bpy46a2y/result.json`），应继续检查递归求值在逻辑深度拒绝前耗尽物理栈的问题。显式过滤该测试后的其他 79 项通过，仅用于隔离影响，不视作修复。

## Scalar direct passing 请求的独立内存足迹（已解除间接调用阻塞）

进一步分析确认上一条候选调用阻塞并非必须复制 handler 捕获前提：scalar direct passing 请求只交付已安装的标量，不在请求处调用 provider。memory/effects.rs 的 `require_effect` 现在优先使用显式前提；没有前提时，只对声明 ABI 为 DirectPassing、实例化后签名为零参数且无指针标量／void 结果的操作，推导请求侧空访问足迹。操作类型实参和签名有预算检查，request 继续核对实际 ABI、签名和行槽。普通 direct、指针结果及 owning payload 仍需要原有前提。

这不是 provider 纯度或 Copy/Drop 证明。安装分支仍正常遍历并记录读写、拒绝和清理；delivery 仍由资源分析单独检查。现有拒绝分析继续保守标记请求 may-reject，本次不从空内存足迹推导无失败保证。

`native_direct_passing_indirect_abi_with_resource_admission` 已替换旧的阻塞预期：两个 runtime 候选、caller/callee 不同行、实参请求都通过 Copy/Drop 检查，checked 解释执行与 LLVM O0/O3 一致。内存推导回归覆盖 bool/u128/f64 泛型实例、零预算失败后重试、provider 指针读写与 assumes 拒绝保留、普通 direct／pointer direct passing／伪造请求 ABI 拒绝。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test effect_memory_transfers --test effect_spreads` 退出 0，88+21 项通过（`/tmp/luna-tools-8qy9l5ll/result.json`）；`test -p luna --test host direct_passing` 退出 0，9 项通过（`/tmp/luna-tools-1baqvzvc/result.json`）。最初新测错误地期待 may-reject=false，已改为核验独立拒绝分析保持保守，未修改该分析规则。全量格式化退出 0（`/tmp/luna-tools-8rvtfgw3/result.json`），git diff --check 通过。dev 未提交。

## Effectful 间接候选的 memory premise 尚未接通

host 的 scalar direct-passing 隐藏参数 ABI 已可执行间接函数调用，但 checked 路径在候选内存摘要处被拒绝。复现见 `compiler/luna/tests/host.rs::native_direct_passing_indirect_abi_and_resource_admission_boundary`：caller 行 `[enabled,ready]`，runtime 在两个 `#ready` 函数间选择，用 caller 的 `enabled` 请求作为实参。无资源协议时解释执行及 LLVM O0/O3 一致；启用 Copy/Drop 时错误为 `every callable candidate requires a memory contract: effect request requires an installed handler memory premise`。测试保留该拒绝和旧产物保护，不把未检查 ABI 通过当作 checked 支持。

调查入口为 moves.rs 的 `memory_candidate_summary_with_enums`、`memory_summary_with_enum_context`，后者把 `self.effect_memory_inputs` 交给 `infer_callable_memory_with_borrowed`；memory/effects.rs 的 `require_effect` 找不到 premise。后续应追踪条件分析 frame 对安装 premise 的取得／映射／传递及每个候选的验证，不能只为消除错误而推定 handler 无副作用或移除候选检查。普通 named direct-passing 调用、递归、拥有参数析构已 checked 执行通过。本次未修改该前端边界。

## Handler 清理摘要与观察权限的集成边界

新增源码反例：普通 touch 函数内部通过 handler 参数构造带指针 Watch 并在退出时写回；调用 touch 的 guard 仍以 `guard pointer observation cannot write caller memory or run unverified cleanup` 拒绝，旧原生产物不变。参数访问前提让正常运行可验证，但不能被转用为只读观察权限。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host handler_cleanup_summary` 退出 0（`/tmp/luna-tools-0z91aqkd/result.json`）；`test -p luna --test default_calls --test method_memory_contracts --test staged_aggregate_contracts` 退出 0，33+8+31 项通过（`/tmp/luna-tools-nyv2mvwk/result.json`）。格式化退出 0（`/tmp/luna-tools-m2t4zpm2/result.json`），git diff --check 通过。未改动语言契约；dev 未提交。

## 条件枚举指针参数的清理验收

新增 effect 参数为 `Choice {none,left(*bool),right(*bool)}` 的源码回归。运行时选择 none／left／right，两个载荷指向不同目标；普通完成与 resume 后只允许实际载荷的 Drop 写回。再加入部分 handler 先接收 none、其余标签转发的组合，验证条件参数访问前提在转发后仍保持标签与来源关联。空载荷不要求为不存在的指针建立来源。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host native_checked_handler_cleanup_respects_enum` 退出 0，解释执行与 LLVM O0/O3 全部通过（`/tmp/luna-tools-mzu241jz/result.json`）。格式化退出 0（`/tmp/luna-tools-pf5yq6gn/result.json`），git diff --check 通过。本轮扩展条件载荷验收，未改变借用规则，未重复全量测试；dev 未提交。

## 指针参数析构的字段、别名与存活边界

新增源码回归覆盖 Copy 聚合参数内的两个指针字段、两个独立指针参数、两个参数别名同一标量。前两项各自写回两个目标，别名项两次翻转同一目标；资源检查后的解释执行与 LLVM O0/O3 结果一致。它们验证不同字段来源不覆盖，以及实际别名不因虚拟借用根分开而在原生执行中丢失。

另验证参数表达式返回块内局部地址时仍拒绝：`pointer target unavailable outside its local scope (escaping provenance)`，旧可执行文件保持原样。建立 handler 的清理前提不会延长实参目标的生命周期。本轮只扩展已实现路径的边界验收，没有修改借用／清理规则。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host checked_handler_cleanup` 退出 0，2 项通过（`/tmp/luna-tools-ibgjb111/result.json`）。格式化退出 0（`/tmp/luna-tools-kdb4551w/result.json`），git diff --check 通过。dev 未提交。

## Effect 指针参数用于局部析构的来源交付

已定位并修复 `direct_handler_accesses` 因 may_run_cleanup 整体丢弃参数摘要的问题。存在隐式清理时，按每个输入指针叶建立保守的读／写前提，保留变体限定路径；moves 后续仍检查参数与目标的 Copy／借用要求，并以实际参数来源验证所选 Drop。memory 调用方同时传播 may_run_cleanup，不能把未知隐式清理误当成已经精确验证的访问摘要。结果指针／指针字段逃逸检查保持。

原“effect 参数传入指针，在 handler 中构造 Watch 后析构”拒绝例现通过，已转为实际写回正例；正常完成／resume、直接匹配／部分 handler 转发四种组合均对照资源检查后的解释执行并通过 LLVM O0/O3。Handler 内对词法捕获新取址仍是独立拒绝边界。保守全叶读写前提可能要求比实际 Drop 更多的输入能力；精确 Drop 摘要及更一般借用协议仍待完善。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host --test effect_memory_transfers --test effect_spreads` 退出 0，385+85+21 项通过（`/tmp/luna-tools-zhuof7bp/result.json`）。定向新正例报告 `/tmp/luna-tools-96v5rg7t/result.json`。格式化退出 0（`/tmp/luna-tools-t9j2k6k4/result.json`），git diff --check 通过。dev 未提交。

## 稳定指针捕获的 handler 析构写回

补充已支持内存路径的源码证据：安装前 `let target=seen.ref`，handler 捕获稳定 target 并构造 Watch，其 Drop 经该指针写回。零／一／两层未匹配 handler 转发与普通完成／resume 六种组合均通过资源检查后的解释执行及 LLVM O0/O3。这里没有在 handler 内对词法捕获新取址，也没有通过 effect 参数传递指针，不改变前述两项拒绝边界。

`host-direct-effect-cleanup.fl` 已改成这个可观察清理示例：输出 true 同时依赖 resume 结果和析构写回，避免只以没有 trap 作为资源示例的验收。拥有型 effect 参数交付仍由其它测试覆盖，本项不宣称带指针 effect 参数的来源协议已完成。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host native_checked_handler_stable_pointer_capture` 退出 0（`/tmp/luna-tools-i54x01t5/result.json`）。`python3 -B development/host_examples.py direct-effect-cleanup --opt-level 3` 退出 0，实际构建／链接／执行通过（`/tmp/luna-host-examples-ujdz048g/summary.json`）。格式化退出 0（`/tmp/luna-tools-noeb1x4x/result.json`），git diff --check 通过。dev 未提交。

## Handler pattern 计算的写入范围

修复 handler 捕获写入扫描仅包含 guard／body、遗漏 pattern 计算根的问题。新增 `flow::patterns::handler_arm_writes`，以 pattern_observations 加 guard／body 为根执行有预算的完整遍历；其结果参与请求后事实失效及 handler 稳定性依据。现有观察规则仍独立检查 pattern 中的外部写入；本项不授权之前被拒绝的写入，也不依赖该拒绝来省略失效分析。

结构级回归直接构造三个根的写入，验证目标集合完整、无写入返回空集合、预算耗尽后可重试。它隔离扫描范围，不把未定型的结构 fixture 当作合法 Flurry 程序。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p elaboration --lib handler_writes_include` 退出 0（`/tmp/luna-tools-ymjqxgfk/result.json`）；`test -p luna --test host` 退出 0，383 项通过（`/tmp/luna-tools-9e8n_5no/result.json`）。格式化退出 0（`/tmp/luna-tools-f456eo33/result.json`），git diff --check 通过。dev 未提交。

## Direct-passing 安装后的稳定事实

安装时 provider 运行后，flow 不再无条件清除所有标量事实，而是复用完整函数的 handler_unstable_locals（写入与地址暴露并集），保留稳定的按值标量。被修改的捕获、内存读取和别名相关事实仍失效。此处与请求阶段不同：即使计算体短路、不请求 effect，安装期写入也必须生效。

新增用例先复现稳定 flag 的后置条件失败（`/tmp/luna-tools-q3yqoxh7/result.json`），修复后正常请求／短路请求两种函数均通过资源检查及 LLVM O0/O3。两个负例让 provider 分别直接改写和通过指针改写捕获，计算体跳过请求；两者仍被后置条件证明拒绝，不能沿用安装前真值。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host direct_passing` 退出 0，4 项通过（`/tmp/luna-tools-kojs5j68/result.json`）；新增测试单独通过（`/tmp/luna-tools-1x54p2oi/result.json`）。格式化退出 0（`/tmp/luna-tools-99wh58xq/result.json`），git diff --check 通过。dev 未提交。

## Handler 稳定标量事实与拥有型返回结果

修复 flow::effect_handler_arms 对所有普通参数事实的整体清除。完整函数已有的写入／取址扫描现在给 Walker 提供 handler_unstable_locals（两者并集）；handler 在任意请求时刻分析时，仍清除可能写入、取址暴露的局部事实和内存观察，只保留未写入、未暴露的词法标量事实。没有扫描依据的内部 Walker 保持全量 havoc。Handler 分支结果仍不成为安装后的无条件事实。

初始源码 `requires flag, ensures flag` 即便 flag 从不改变，handler return 也无法证明后置条件（`/tmp/luna-tools-fjeitruq/result.json`）。修复后单层／嵌套 handler 的拥有型 Watch 返回、具名／无名结果都通过资源检查后的解释执行及 LLVM O0/O3；并保留直接写入和通过指针写入捕获的拒绝例，防止使用安装前旧事实。

验证目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 300 -- test -p luna --test host native_checked_handler_return_delivers` 退出 0（`/tmp/luna-tools-wqdf6abk/result.json`）；`test -p luna --test host handler_postcondition` 退出 0（`/tmp/luna-tools-y1wgg19m/result.json`）。`test -p luna --test effect_memory_transfers --test effect_spreads` 退出 0，85+21 项通过（`/tmp/luna-tools-1tv07abh/result.json`）。格式化退出 0（`/tmp/luna-tools-pj2xinam/result.json`），git diff --check 通过。负例初稿 `pointer.*=false` 被 lexer 识别为复合符号，已改为 `pointer.* = false` 并确认命中后置条件证明诊断。dev 未提交。

## Guard 清理：已支持路径与候选事务边界

核对 `docs/control-flow/pattern-matching/index.md` 的观察块契约：块内私有无指针资源当前要求调用与清理具有不拒绝依据。因此前述 fallible Drop 拒绝符合现行契约，不能直接删除 `check_observation_cleanup` 的 may_reject 检查。一般候选事务的失败路径支持仍待扩展，但不是当前 handler 后端遗漏一次清理所致。

新增源码验收证明无失败 Drop 在 guard 为真、guard 为假后转发、多个 guard 依次尝试时均可执行。检查未优化 LLVM 中每个私有对象对应的析构调用数（排除 flurry_check_failure 辅助调用），并在 O0/O3 对照资源检查后的解释执行结果。观察清理仍保持在 arm 的独立资源帧内。

工作目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 300 -- test -p luna --test host handler_guard` 退出 0，2 项通过（`/tmp/luna-tools-hxqutszc/result.json`）；格式化报告 `/tmp/luna-tools-vg_l7z23/result.json`。仅增加契约边界验收，没有放宽前端观察规则；dev 未提交。

## Checked handler 组合验收与 observation 清理缺口

源码析构矩阵从五种退出扩大到嵌套 handler 正常退出／跨帧 return、部分匹配向外转发、拥有 Drop 的参数交付、direct-passing 安装时的 resume／return。各场景的析构谓词成功与失败均对照资源检查后的解释执行，在 LLVM O0/O3 实际执行。另以可变捕获翻转验证 direct-passing 在两次请求前只安装一次。

Guard 内创建带 fallible Drop 的局部对象仍在前端资源准备阶段报 `observation cleanup needs failure-path resource transfer`，即便该实例字段使谓词为真，也不能因此假定一般析构不失败。本轮增加拒绝／旧产物保护用例，未放宽检查；后续须为 observation 的析构失败补完整清理转移。该项与指针 provenance 缺口独立。

验证工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host checked` 退出 0，61 项通过（`/tmp/luna-tools-1owsvo4c/result.json`）。正例矩阵定向报告 `/tmp/luna-tools-85bhygx8/result.json`，direct-passing 次数报告 `/tmp/luna-tools-bea3afg9/result.json`。Guard 初次正例尝试失败报告 `/tmp/luna-tools-nyjvog4h/result.json`；该语义未实现，现明确保留为拒绝。格式化退出 0（`/tmp/luna-tools-2lu6v431/result.json`），git diff --check 通过。未重复全量检查，dev 未提交。

## Checked direct handler 源码准入已接通

新增 suspension 来源核对覆盖 direct request 与 direct-passing 安装点，接通 EffectEscape／EffectReturn／EffectBreak／EffectContinue 以及返回结果 postcondition origin 的清理准入；去除生产路径对所有 checked handler 的整体拒绝。具体上下文与每个清理站点仍先验证，跨函数 ABI 和跨默认求值的非局部退出仍明确拒绝。

真实源码的普通完成、resume、return、break、continue 现均通过带 Drop 谓词的成功／失败验收及 LLVM O0/O3。宿主回归 48+376 项通过，新增仓库示例定向通过，详见 host-pipeline.md 开头。

本轮进一步定位两个前端缺口（均在解释执行准备时即可复现）：effect 参数传递的指针用于聚合析构时缺 stored pointer provenance；handler 内对词法捕获取址缺 memory contract。未放宽安全检查，新增两项拒绝／旧产物保护案例。下一步应补参数与捕获的真实内存协议，并扩大 nested/partial/direct-passing 的 checked 源码验收。此前“生产 gate 保留”的条目是历史状态。


## Handler 清理来源定位与正常退出准入

`admitted_direct_program` 以最近 Function／Closure 为机器函数基底，再逐个进入 Default／EffectHandler。handler 通过 installation/handler/arm 唯一定位；pattern 的计算根与 guard 作为该 arm 的 observations 独立遍历，不伪装为函数前置条件，也不跨入更深层资源帧。进入子帧清除父函数的 result binding、快照与契约。HandlerEnd 只接纳当前 arm 正文根；EffectResume 对应实际 resume 节点。EffectRequest 的 prepared 参数按参数位置和 value source 检查，请求／安装来源可承载 PredicateFailure。

扩展来源准入回归覆盖父帧拒绝 arm 内 discard、正确 arm／guard 接纳、handler→default 与 default→handler、错误 arm 编号、HandlerEnd 的正确来源与父帧拒绝。本测试隔离结构来源；一般 checked handler 的生产 gate 仍保持。剩余为 EffectEscape／EffectReturn／EffectBreak／EffectContinue 与目标函数 postcondition origin 的对应检查，以及真实源码的资源清理验收；不得凭底层来源检查通过就宣称完整 ABI 接通。

验证工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host` 退出 0，48+374 项通过（`/tmp/luna-tools-oymuyt1z/result.json`）；定向来源测试 `/tmp/luna-tools-fc2n4xbu/result.json` 退出 0。格式化退出 0（`/tmp/luna-tools-pzb_gjxr/result.json`），git diff --check 通过。dev 未提交。

## 清理来源验证的资源帧边界

`admitted_direct_program` 的来源匹配改用 `expressions_in_scope`：验证当前帧的 discard、prepared/temporary 责任和退出站点时，不再穿越默认参数、闭包正文及 handler arm。默认参数上下文逐层进入，要求 construction/parameter 在当前帧中唯一匹配，重复来源不再由最后遍历的节点覆盖。该修复为后续 handler 准入建立来源边界；尚未放开 EffectHandler 上下文与生产 checked handler gate。

新测试直接验证：父帧不能使用 default 内的 discard；正确子帧可接纳；同帧重复 construction 拒绝；嵌套同来源只通过显式追加 context key 进入；缺失子帧拒绝；预算耗尽后的充足预算重试成功。测试只隔离来源准入，空清理程序不用于声称析构执行正确。

验证工作目录 `/home/fin/luna`：实现修改后 `python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host` 退出 0，47+374 项通过（`/tmp/luna-tools-v3z437al/result.json`）；随后新增来源回归，`test -p luna --lib cleanup_admission_requires` 退出 0，1 项通过（`/tmp/luna-tools-x5ynewo9/result.json`）。格式化退出 0（`/tmp/luna-tools-tk7u8sk2/result.json`），git diff --check 通过。dev 未提交。

## Handler 清理准入续接：机器函数共享检查

`host/llvm/cleanup.rs::inline_scopes` 现在将 Default 与 EffectHandler 都视为当前机器函数的内联资源上下文，按完整相对路径参与 `same_frame` 比较。Function／Closure 仍终止本机范围，不能把其后代资源计划误算进外层函数。handler 的安装来源、handler 编号和 arm 编号均参与身份；即使析构列表相同，资源转移差异也禁止共享代码。

新增回归覆盖 handler、handler→default、default→handler、嵌套 handler 四种路径，逐项检查来源／编号变化、转移差异、重复 scope、预算耗尽后重试，以及独立函数／闭包边界。本项解除共享检查对 handler 上下文的整体拒绝，但生产 checked handler gate 尚未开放。下一步仍需在 `admitted_direct_program` 中从最近 Function／Closure 精确定位内联 Default／EffectHandler 源码范围，逐项验证 handler 边界与 prepared 责任，再用真实源码接通；不能直接删掉 gate。

验证工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib cleanup` 退出 0，7 项通过（包括实际 O0/O3 handler 清理回归），报告 `/tmp/luna-tools-5rqftsnp/result.json`；`test -p luna --test host effect` 退出 0，报告 `/tmp/luna-tools-2xl4gbto/result.json`。格式化退出 0（`/tmp/luna-tools-dd3oz6b1/result.json`），git diff --check 通过。dev 未提交。

## 2026-10-24 续接摘要

在 dev 基线 `944e40ce` 的已有未提交工作区继续，保留全部改动。当前优先交付 LLVM host 示例入口；以下历史条目按时间累积，已修复项以较新的记录为准。

- 浮点延迟形成已完成具体域检查、实例化、comptime 与 staged forwarding 接入，已有测试证据见末节。
- 整数延迟形成已完成 `PendingInteger`、形成函数和相关消费者接入，并补验范围边界、实例失败恢复、目标字长和阶段准入；具体证据见下文“泛型整数字面量形成”和“整数形成的 staged／平台目标组合”。泛型 BitVec 不属于此形成路径。
- 前端后续重点仍为泛型模板／CMTT 缺口、完整库与 sysroot 接线、字符串 view 的分配来源和生命周期，以及一般效应的资源协议。PTC 继续只处理类型命题。
- Host pipeline 已存在并经本轮复验；保留 IIR 类型身份与资源检查，不借后端实现放宽上述语义。新示例运行入口与本轮证据见 `host-pipeline.md` 开头。

当前 dev 工作区保留全部实现和测试，未提交。按作者要求暂时停止扩展语义前端，优先让已支持的示例生成并执行本机产物。

- 括号 tuple 值与 Tuple 类型身份已分离；`Tuple` 保留 opaque normal-form 头。
- 函数签名模板已接通普通/inline/quote 实例执行，支持参数、默认值、契约、普通 comptime 参数依赖、闭包环境和嵌套默认表达式；声明侧类型身份及各实例绑定独立。
- 带显式参数列表的模板实例只允许 quote fn（docs/grammar/syntax:360）；普通实例继承签名。本次收尾模板/解析测试共 73+7 项通过，报告 `/tmp/luna-tools-71h1s2yr/result.json`。
- 泛型模板子句实参、显式 quote 实例参数匹配、case 参数/quoted arm、共享 CMTT 正文与 contextual quote 输入仍待实现。不要把目前未实现的分支误当成语言限制。
- PTC 只负责类型命题。默认值 P3 证明与资源协议独立：有合法 Copy/Drop 证据时局部 nominal 解构可执行；有外部副作用的析构仍拒绝。
- 其它前端缺口包括一般 meta.List<T> 数据、部分迭代协议及标准库完整接线；更详细过程和已有检查证据见 implementation-plan.md。不能用有限测试通过宣称语言完成。

Host 后端应消费已实例化 IIR，保留 opaque 类型身份，单独取得表示；不得重用停用 MIR/ty 的语义或悄悄改变前端规则。不支持的 runtime 操作必须带来源拒绝，失败产物不得覆盖已有文件。

- Host 宽整数验收再次确认：有符号源码 `/` 的舍入协议仍未实现（program.rs 的既有诊断）；不能为运行示例把它默认解释成向零截断。无符号除法/余数已可生成本机产物。

- Host 回调验收：携带前置谓词的函数转换为无契约函数类型，当前前端明确拒绝擦除 program preconditions；callable program contract 表示仍待接入。callee 为 `do {if true {return true;}unused}` 时，整个值域是 Never，不能从不可达尾部虚构调用签名；现明确诊断为 diverging callee has no callable signature for checking its arguments，保留拒绝。另已修复普通 runtime 退出块把外层结果类型强加给独立定型尾部的问题（见 host-pipeline.md）；comptime/类型输入、展开包及结构输入的形成检查仍保留。

- 已核实 Host 的 void 指针写回可用；旧的 non-Copy pointer writes 限制记录已过时。新增带资源检查的参考执行及 LLVM O0/O3 回归确认：左侧地址计算先于右侧值求值且均只执行一次，右侧提前返回跳过后续 producer；零大小局部存储的指针仍因 local memory target cannot escape 被拒绝，失败构建不产生可执行文件。证据：`/tmp/luna-tools-r8pgib6_/result.json`。

- 已修复 Host BitVec 的 Never 左操作数缺口：`let result:Bits=(do {return true;}) and (do {7});` 可从外层取得 BitVec 字面量上下文；已有类型的右侧自行确定逻辑操作域，不再被强制为 bool，整个表达式仍为 Never。无外层 BitVec 标注、连续 and/or、双方提前退出与布尔短路均已通过参考执行和 LLVM O0/O3；右侧函数没有被执行，不合法的操作数域仍拒绝。证据见 `/tmp/luna-tools-k0iq2exs/result.json`，具体变更见 host-pipeline.md 的 Never 逻辑操作一节。

- 固定编码 enum 的零宽载荷在现有 elaboration/encoding 验证中被拒绝：`enum payload has zero width or duplicate name`。普通 `BitVec<0>` 的 host 路径已接通，但不能据此宣称枚举零宽载荷可用；本轮保持前端验证与 host 拒绝，待恢复该主题时核对设计契约。

- Native Drop 逆序测试中，析构方法的 `assumes self.second.*.*` 被现有资源准备拒绝为 observed pointer slot contents provenance is not established；相同已存储指针用于普通 RHS 读取及写回可通过。已改用普通读取观察析构顺序，未放宽谓词观察规则；恢复验证层开发时需核对 Drop 入口的指针槽来源事实。

- Host 资源交叉验收：普通参数期望 record 内为 `![Other,Fault] bool`，实参 record 内为 `!Fault bool` 时，当前 elaboration 报 unresolved type equality (UnvalidatedModel)，不能自动提升内嵌错误行（`/tmp/luna-tools-sfcp7tf7/result.json`）。改用已有泛型行匹配入口可接受，但发布的 IIR 已统一行顺序，没有留下 Product 节点（`/tmp/luna-tools-xirmxfqc/result.json`）。后续需分别检查聚合内错误行扩大能力与 Product 降码的直接测试覆盖，不能把来源名称含 product 的用例视为实际执行 Product 的证据。

- Product 覆盖续接：已通过内部 LLVM 原生测试直接执行嵌套 record 的物理行重排（成功/两类错误/非法标签，O0/O3，`/tmp/luna-tools-7jo8s8k2/result.json`）。这补足后端降码分支证据；源级聚合内错误行扩大及何时插入 Product 的前端问题仍待续接。

- Host staged callback 资源清理来源冲突（已修复，见下项）：`development/repros/host-staged-callback-resource-boundary.fl` 中，同一声明的 `<true>` / `<false>` 实例经过可变 callback、alias 和重新赋值，在启用 Duplicate/Retire 时被 `cleanup site has no unique runtime boundary in trial at 4294967295..35 (TemporaryEnd, 2 matches)` 拒绝。`python3 -B development/check.py --timeout 300 -- run -p luna -- development/repros/host-staged-callback-resource-boundary.fl --build run --copy-protocol Duplicate --drop-protocol Retire --output /tmp/flurry-staged-resource` 退出 1，报告 `/tmp/luna-tools-mbsmj1tm/result.json`。不启用资源协议的标量路径可原生执行；另一个有指针副作用的 staged callback 用例可通过资源准备，不能把此问题概括为所有 staged callback 的资源层都不可用。恢复前端时核对合成 source 与临时清理边界身份，不通过丢弃 schedule 绕过冲突。

- Trait callback 资源准备的历史阻塞（struct receiver 的绑定语句问题已修复，见下项）。本轮普通 trait 方法回调启用 Duplicate/Retire 后报 `executable cleanup contains unresolved callable obligations`（定向测试报告 `/tmp/luna-tools-muibzytz/result.json`）；带 struct receiver 的 staged 方法回调报 `call preparation requires binding statements`。后者复现文件为 `development/repros/host-trait-callback-resource-receiver.fl`，通过 `python3 -B development/check.py --timeout 300 -- run -p luna -- development/repros/host-trait-callback-resource-receiver.fl --build run --copy-protocol Duplicate --drop-protocol Retire --output /tmp/flurry-trait-receiver` 实测退出 1（`/tmp/luna-tools-s7hi2hzg/result.json`）。Host 已接通方法选择与标量 runtime receiver，但这不能作为 nominal receiver 的资源交付已完成的证据；保留原拒绝，后续修复合成调用准备与 callable obligation 的交接。

- 已修复上述 `call preparation requires binding statements`：staged callable 别名展开后留下的 void Unit 在最终 CallPreparation 规范化中移除；资源检查仍要求绑定语句，没有放宽接纳。原 struct receiver 复现已成功生成本机产物（`/tmp/luna-tools-c2e_eipk/result.json`），并已纳入 host 回归。额外拥有 Drop 的 receiver 用外部布尔翻转验证实际清理，O0/O3 通过。普通方法回调的 unresolved callable obligations 和可变回调的重复清理来源仍分别保留待查，不能由本项推定解决。

- 已修复可变 staged callback 的重复清理来源：降码为 Match 时，读取标签的子节点取得独立 generated source，复制的候选表达式也重新分配展开来源。原 `host-staged-callback-resource-boundary.fl` 已能通过 paired Copy/Drop 准备并生成本机产物（`/tmp/luna-tools-uprpiq1j/result.json`）；保持站点唯一性检查，不通过合并重复 hooks 放行。普通 trait 回调 unresolved callable obligations 仍是独立待查项。

- 已补齐普通 trait 方法值的 residual callable 来源：在具有 runtime instance 图且签名无 staged 输入时，资源分析通过保留接口检查的 MethodCallees::select 取得完整目标，再沿普通函数的值来源路径传递。普通方法回调的 unresolved callable obligations 复现现已通过；带 trait 值实参的运行时选择及指针写入也通过完整资源验证和 O0/O3 原生执行。此前该项待查标记作历史记录，接口 program contract 的一般组合能力仍未因此实现；携带接口谓词的方法值仍明确拒绝擦除契约。

- 有状态闭包的指针调用尚缺借用 callable-place 契约：`development/repros/host-borrowed-closure-pointer.fl` 用 `pointer.*()` 连续调用同一环境，当前前端拒绝 non-intrinsic pointer read，需要 Copy 依据或 memory transfer contract（最初报告 `/tmp/luna-tools-o_skhvus/result.json`）。不能给闭包随意添加 Copy 实现，也不能把这次调用解释为先复制环境再调用；后端当前 closure_place 仅恢复 lexical local/field 位置，恢复此路径时须同时让指针 callee 恰好求值一次、保存原环境地址，并处理实参求值期间的别名改写。此项尚未实现，拒绝回归只保护现有边界，不代表功能完成。

  本轮 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host native_closure_pointer_call_requires_borrowed_callable_contract`，退出 0，1 项拒绝／旧产物保护回归通过（`/tmp/luna-tools-zf_3lf1d/result.json`）；格式化退出 0（`/tmp/luna-tools-kyw65786/result.json`）。未运行全量测试，dev 未提交。

- Host 字符串接线确认：内建 `str` 可在未启用资源协议的 host 路径中以不可变字面量存储传递和匹配，但 checked 路径仍由 `verification/memory.rs` 拒绝：`string-view memory provenance is not implemented`。`str` 是借用的字符视图；下一步应建模 backing-region 来源、有效期与传递，不能因为 LLVM 字面量存储静态存在，就把所有 str 当成无指针标量或通过 Copy 实现绕过检查。本轮只增加后端支持及保留拒绝的回归，没有更改该前端边界。

- Intrinsic host 接线第一阶段：AST lowering 将 `'intrinsic` 保存为专用 HIR `TakeIntrinsic`；普通 application 保留操作数，紧接 `.as(T)` 的 cast 结构也保留。HIR Display 为 `(take-intrinsic (tag ...))`，表达式遍历和函数模板克隆已接通。这里只完成源结构交付，不代表 intrinsic 已定型／可执行；具体与开放函数 elaboration 继续明确拒绝，unsafe block 的 pending semantics 不被消除。
- 后续 intrinsic 所需工作：按枚举声明与变体身份建立操作标签；落实 unsafe 作用域；将显式输出请求及按源序求值的实参交付 NIR/IIR；在纯度、资源与目标检查中保留未知行为；最后由 host 通过显式操作协议选择 LLVM 实现。不能按 `fma` 等拼写冒认任意枚举变体，也不能从 `.as(T)` 推出无副作用或内存安全。基础 math 包装需要这条实际链路，不应添加假的 builtin 函数来掩盖缺口。

- Unsafe 源结构续接：unsafe block 不再降为无法区分的普通 HIR Block；现在保留 `UnsafeBlock` 节点，函数声明的 `FnModifiers.is_unsafe` 也独立于纯度和阶段保存。S-expression dump 分别为 unsafe-block-expr 与 modifiers 的 unsafe 字段。模板克隆与结构遍历保留边界。原有 pending semantics 仍保留，普通与开放表达式 elaboration 也明确拒绝未实现的 unsafe admission；这不是 unsafe 操作、函数调用契约或审计传播已经可执行。下一步需要落实这些检查，并在后续 IR 保存相应审计证据，不能直接把新节点删成普通 Block。

- Unsafe 普通块执行续接：上述普通块的整体拒绝已解除，HIR `UnsafeBlock` 降为 NIR/IIR `UnsafeScope`，dump 保留 `(unsafe-scope ...)`。块的结果类型必须等于主体类型；阶段、纯度、资源来源、程序规格和算术检查继续进入主体，LLVM 在验证后执行主体。普通／泛型函数、早退、拥有值清理与转移、指针和 callable 来源、可选内联及 inline effect/resume 已有原生回归。此处仅接通现有合法操作组成的块，不授予尚未实现的 unsafe 操作权限；unsafe 函数的调用责任与 intrinsic 目标协议仍拒绝，后续必须单独实现。

- 高阶 unsafe 接口的前置结构已补齐：AST 中带 pure/inline/unsafe/spec/verified 的函数类型此前只设置 pending semantics，HIR 则丢失这些修饰。现在使用 `TyModifiedFn(FnTypeModifiers, parameters)`，独立保存 pure、comptime、inline、unsafe、spec、verified；Display 为 `(ty-modified-fn (modifiers (fn-type-modifiers ...)) (params ...))`。外层 `TyFnArrow` 继续保存返回类型，嵌套签名各自保留修饰和参数作用域。模板签名复制和输入依赖遍历已接通；普通及仅 comptime 的既有类型表示保持其已实现路径。带这些修饰的接口仍有 pending admission，elaboration 本身也明确拒绝新节点，不允许丢弃 unsafe 责任后退化为普通函数类型。下一步仍需在语义 callable 类型中表达责任，定义调用边界检查及高阶兼容，然后接通 unsafe 函数执行；本项不宣称这些功能完成。

  工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- check -p luna` 退出 0（`/tmp/luna-tools-vf4d755u/result.json`）。`test -p hir --test display` 经同一入口退出 0，13 项通过（`/tmp/luna-tools-pg17qu5d/result.json`）。`test -p luna --test frontend_cli --test test_templates --test function_type_dependencies` 退出 0，分别 181、74、33 项通过（`/tmp/luna-tools-sowwu023/result.json`）；覆盖修饰组合、嵌套类型、依赖参数作用域、未知类型名拒绝和模板克隆后仍执行 admission。最初定向测试因输入未绑定 Type 失败，改用 `.Type'builtin` 后通过上述回归。格式检查退出 0（`/tmp/luna-tools-x44pl27v/result.json`）。未运行全 workspace 测试，dev 未提交。

- Unsafe callable 的语义类型已加入 `Term::UnsafeFunction`，Display 为 `(unsafe-function signature)`。它作为独立构造器参与类型身份、PTC 同余／构造器区分、匹配和 implementation 结构键，不与普通 Function 相等。内部规范表示为 unsafe 包装 ordinary Function 或 ComptimeFunction；闭包环境位于包装外层。该选择是内部表示，不新增源语法。类型重写、绑定特化、返回类型替换、associated-type 归一化、completed-value／运行时存储遍历、类型预算与迭代释放均继续访问内部签名；comptime 阶段与 unsafe 责任各自可查询。测试覆盖环境包装、错误嵌套、自由绑定与 comptime-only 存储不能被包装隐藏。

  源级 `TyModifiedFn` 与 unsafe 函数仍保留 pending admission。本轮没有将它们提前放行；NIR 阶段检查与 IIR 校验明确拒绝 unsafe callable 的执行入口和调用，包括手工构造的 IR。下一步需把 HIR 签名接入这个类型，并用可传播的词法责任上下文处理直接／间接调用、函数体、闭包与默认实参；这些检查完成前不能只删除 pending 标记。

  在 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p nir -p iir` 433 项通过（`/tmp/luna-tools-jqe3hmut/result.json`）；同一入口执行 `test -p elaboration --test ptc --test matching --test implementation --test ptc_kinds` 149 项通过（`/tmp/luna-tools-dya38vm4/result.json`）；`test -p luna --test host --test function_type_dependencies` 342 项通过，包括 309 项 host（`/tmp/luna-tools-6smq5rby/result.json`）。均退出 0。首次 NIR 测试编译因新断言误用无 Display 的 Error 失败，改为读取 message 后已通过上述测试。全 workspace 格式化退出 0（`/tmp/luna-tools-t_3h9gaq/result.json`）。未执行全 workspace 测试，dev 改动未提交。

- 源级 unsafe callable **类型形成**已接通：`unsafe fn(...) -> R` 与 `comptime unsafe fn(...) -> R` 可用于 typealias、参数／返回类型和签名模板；依赖参数按同一个 telescope 绑定，嵌套返回接口保留各自责任。环境限定 `%{...} unsafe fn(...) -> R` 保留环境及 unsafe 签名。Type 值比较已验证 unsafe 类型与自身相等、与普通 fn 不相等。pure/inline/spec/verified 函数类型的独立承诺仍待实现，没有随本项放行。unsafe 函数定义及调用也仍待实现；即使位于 unsafe block，现有 NIR 拒绝仍明确报告 call responsibility 未实现。

  后续实现注意：当前 NIR `Function.signature` 保存普通参数／结果 telescope，comptime 定义另用 `Function.phase`；`ComptimeFunction` 是函数**值**的类型包装。unsafe 定义应同样保留声明责任 metadata，并在产生函数值类型时附加 UnsafeFunction，不能只把所有定义签名直接替换成包装而破坏已有实例化／ABI 消费者。还需按明确的 unsafe 边界检查调用、默认实参和生成代码。

  工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test function_type_dependencies --test frontend_cli --test test_templates` 退出 0，36、181、74 项共 291 项通过（`/tmp/luna-tools-t2zlqi4r/result.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-lg3i0mj2/result.json`）。未运行全 workspace 测试，dev 未提交。

- Unsafe **声明责任 metadata** 已接通：NIR `Function` 与 `MethodContract` 增加独立的 `is_unsafe`，Display 输出 `(unsafe true/false)`。普通／开放函数、required comptime 函数和 trait 方法契约从 HIR 保留该字段；类型特化与方法 program summary 继续保留。`Function.signature` 仍是普通 telescope，新增 `Function::callable_type()` 统一生成带阶段和调用责任的值类型。staged 调用重建、函数实例与方法值的解释器类型重建已接通责任包装；仅设置新的 metadata 不能绕过 NIR/IIR 现有执行拒绝。合成的安全包装默认不获得 unsafe 权限。源级 unsafe 函数定义与调用仍待词法边界 admission，不宣称已可执行。

  在 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p nir -p iir` 434 项通过（`/tmp/luna-tools-95wkea7a/result.json`）；补充方法摘要责任传播后 `test -p nir --test code_interfaces` 13 项通过（`/tmp/luna-tools-es3d04f1/result.json`）。`test -p luna --test host --test function_type_dependencies --test runtime_purity --test trait_purity` 428 项通过（`/tmp/luna-tools-eik1b9wa/result.json`）。`check -p comptime_interpretor -p elaboration --tests` 退出 0（`/tmp/luna-tools-7oal9lww/result.json`），这里只做测试目标编译，未声称运行这些 crate 的全部测试。全 workspace 格式化退出 0（`/tmp/luna-tools-rt80jggi/result.json`）。未运行全 workspace 测试，dev 未提交。

- Runtime unsafe **调用 admission 已接通**，取代前面普通 runtime unsafe 定义／调用的整体拒绝。`nir::call_safety` 用有预算的词法遍历检查实际调用，NIR phase traversal 完成后检查责任，避免覆盖更早的结构诊断；IIR 独立重检，并校验函数／方法引用的责任与目标定义一致。值取得与调用分离，普通 helper／default 不继承调用方的边界。泛型、trait、函数值、默认实参、闭包与可选内联已实测原生执行。资源层按完整 callable 类型核对候选，unsafe 函数值仍是无环境函数类别，不丢弃责任比较。unsafe comptime／quote 定义和 intrinsic 继续拒绝；跨阶段 runtime function identity 的通用 Splice 输入冻结仍待补，已保存独立复现及对照拒绝测试。详见 host-pipeline 的对应小节。

  本轮验证证据见 host-pipeline 最后一节；668 项前端／host 集成回归、434 项 NIR/IIR 测试及资源／解释器定向测试均通过，后续补充检查通过。未提交。

- 跨阶段 runtime callback **具名身份冻结缺口已修复**：可在 invocation-only 输入中保留 RuntimeFunction 或无环境具名实例的声明／实参，重建时核对 registry 或实际定义的阶段、完整签名与 unsafe 责任；不执行目标主体。IIR staged constant 也支持该具名引用。原 host-unsafe-staged-callback 复现现可生成并运行 LLVM 产物，新增 host-staged-unsafe-callback 示例。安全和 unsafe 形式、资源协议、O0/O3、不同泛型值实参均已验证；运行时函数在 comptime 中执行、借 Splice 擦除 unsafe、匿名／捕获闭包身份序列化仍不被授权。验证证据见 host-pipeline 最后一节：922 项前端／host、99 项解释器、70 项 IIR 测试通过。unsafe comptime／quote 定义与通用 intrinsic 仍是独立后续任务。

- Intrinsic 标签声明已保留：`^intrinsic_enum` 仅接受 enum，HIR/NIR 的枚举定义和 S-expression dump 保存 `is_intrinsic`；声明身份沿原有 nominal identity 传递。同名 variant 不合并标签族，未知叠加属性仍阻止 admission。尚未绑定 LLVM 标签族或实现 intrinsic 操作节点，源级调用仍明确拒绝；下一步需按实际声明身份接入目标协议、参数/结果及行为契约检查，不能按 variant 拼写直接映射 LLVM。

- Intrinsic 专用 NIR 节点已建立（标签、源序实参、外围输出/来源），支持遍历、类型替换、预算和 dump。NIR 检查标签身份、runtime 阶段及词法 unsafe；未知目标行为在验证、解释器、IIR 发布中独立拒绝。后续需接通 source call/as 的 elaboration 与目标行为证据，解除拒绝时同步纯度、内存、所有权和程序验证，不能只放行后端。测试证据见 host-pipeline 的对应小节。

- 源码 intrinsic 的直接位置实参调用与 immediate as 已接通专用 NIR，覆盖常规 where 泛型；无 as 只能输出 void，不从外围上下文推导。NIR 可发布结构，IIR 仍拒绝缺少目标行为的节点。下一步为驱动绑定及已验证操作行为接入，随后 LLVM 发射；输入展开/隐式/命名协议与标签族实例仍待实现。645 项相关回归通过，见 host-pipeline 对应小节。

- 已绑定的 f32/f64 融合乘加可通过 host API 原生执行。节点携带具体协议检查结果，NIR/IIR/程序/内存/所有权层与后端复核签名；未知操作保留拒绝。仍需在驱动接线时提供源码 pure 检查所需的提前目标行为，以及泛型实例化后的宽度特化。当前解释器不执行 target intrinsic。测试与具体限制见 host-pipeline 最新小节。

- Host FMA 选择器已在 elaboration 前解析为声明身份，具体 f32/f64 pure 包装可形成带协议证据的 NIR，CLI build/emit-llvm 均已接通（含多包与资源检查）。泛型输出的协议特化仍待实现；类型未确定时不能提前制造具体宽度证据。示例与证据见 host-pipeline 最新小节。

- 已补 pure 泛型在 paired Copy/Drop 下的有限标量类型域覆盖检查：PTC 只证明类型候选覆盖，实际程序/内存/清理行为逐实例交给已有验证层；不使用程序谓词证明类型覆盖、不从 impl 数量推断穷尽。预算失败/未知仍保守拒绝。该修复支撑 library math 泛型 fma 的资源验收，尚不代表一般抽象类型 intrinsic 输出已实现。

直接泛型 host FMA 已贯通：受 f32/f64 类型域约束的 rigid 输出在 NIR 保留 pending-type 绑定，类型实例化后取得具体位宽证据；纯度分析对有限类型域分别检查行为，LLVM 与资源检查拒绝尚未特化的节点。math 库已迁移到单个泛型 intrinsic 函数体。具体边界及验证报告见 development/host-pipeline.md 的“直接泛型 FMA 的待特化绑定”。

void 指针读写的历史形成限制已修复：`intrinsic_pointer_value` 与内建 Copy 分类对齐，新增 void/usize/isize；void 的原生资源检查读写和局部逃逸拒绝均有验收。usize/isize 的 host 数值表示尚未完成。资源协议现独立于 formatting/String；完整 sysroot 仍有原有未实现依赖。自定义 Clone 写入 pointee 后返回带指针聚合的字段来源仍丢失，复现与验证报告见 host-pipeline.md“真实 library 资源协议与 native 验收”。

Clone 返回聚合字段来源丢失已修复：moves 的 aggregate-return 分支现接受 TraitMethod，先做完整静态选择，再传递原有返回 origins/guards。实际 repro、整体复制/重建字段两种 Clone 和嵌套泛型聚合已原生执行通过；返回局部指针及 owner 移出后访问仍拒绝。程序谓词接口的组合内存契约仍未实现。详见 host-pipeline.md“Trait 方法的聚合返回来源已接通”。

接口/实现程序契约的参数、命名结果和 old-value 快照对齐已提取为 NIR BindingAlignment，并接回 staged materialization。补充严格形状/冲突/预算检查；各实例化 pre/postcondition 副本现在有独立执行位置，修复重复 ScopeEnd 导致 checked cleanup 无法定位的问题。普通 runtime trait 方法的组合内存摘要仍待实现，未解除拒绝。验收与设计边界见 host-pipeline.md“契约绑定对齐与 staged 契约边界”。


组合方法内存摘要已有独立查询 API `memory::infer_method_in_package`：保留完整方法身份、接口谓词足迹、实现返回来源及已准备 cleanup 足迹，使用绑定对齐与完整子调用固定点。尚未接入普通 runtime 方法调用准入；失败路径的所有权/结果清理及入口观察仍须一起接通，不能只移除拒绝。当前支持 runtime-only 参数帧；详见 host-pipeline.md“方法接口与实现的组合内存摘要查询”。

高阶方法组合摘要已通过 `infer_method_with_code_inputs` 接收 named 多候选与 prepared closure/capture 前提，摘要保留前提和组合读写足迹。无前提入口不能推断不透明回调；未知身份、签名/布局错误保持拒绝。普通调用接线仍须处理完整方法身份及失败清理，未以行为选择绕过。100 项相关测试通过，见 host-pipeline.md“高阶方法组合摘要的回调前提”。

LLVM 直接 TraitMethod 调用现在从完整方法身份取得接口前置条件，在实参准备之后、进入实现之前检查，并连接 InterfacePreconditionFailure 边界。原生 handoff 用例独立于实现继承前提验证 O0/O3；源码用例验证参数只求值一次与 optional 缺省读取。checked memory/moves 准入及接口后置条件结果清理尚未完成，拒绝未解除。见 host-pipeline.md“LLVM 直接方法调用的接口入口检查”。

非资源检查 host 路径已补齐直接方法调用的接口后置条件：独立入口快照帧、实际返回绑定、正常返回后检查及调用者环境恢复。checked 路径明确保留 result-cleanup boundary 缺口，未复用入口失败边界。350 项相关检查通过，详见 host-pipeline.md“LLVM 接口后置条件与入口观察帧”。

接口后提失败已有独立 IIR cleanup boundary 与返回值 prepared 槽；checked LLVM 可执行显式提供的唯一清理 site，O0/O3 验证成功不清理、失败精确一次。moves 尚未为普通接口调用生成该 site，源码准入仍拒绝；不能把后端 handoff 测试当成源码端到端完成。下一步为完整方法身份下的调用报告、返回结果所有权及 site 发布。见 host-pipeline.md“接口后提失败的返回值清理边界”。

组合方法内存摘要已改为独立接口帧包围真实实现调用，避免把接口后提拼进实现局部帧。新增 p=q 的源码回归证实接口对原输入的读取不再随实现参数重绑定丢失；高阶/返回来源/cleanup overlay 继续通过。101 项相关检查通过。该修复是 moves 生成调用结果清理计划的前提，普通方法准入尚未解除。见 host-pipeline.md“组合内存摘要保留真实实现调用边界”。

NIR/IIR 参考执行器现保留并执行接口出口契约，接口观察值独立于实现 Storage；checked 模式使用唯一 InterfacePostconditionFailure site 清理待交付返回值。共享测试同时验证 LLVM/参考执行成功不清理、失败一次及缺失/重复计划拒绝，另覆盖 NIR program 实例化。moves 的普通接口准入/site 生成尚未接通。见 host-pipeline.md“参考执行器的接口出口与清理一致性”。

已提取完整 MethodInvocation，并接入 protocols::analyze_method_cleanup，使用真实 Copy/Drop/moves 生成接口调用帧的清理报告。带自定义 Drop 的 Token 用例验证入口清理输入、出口失败清理返回值、已转交输入不重复清理。395 项相关检查通过。尚需将报告映射到实际调用点的 prepared 输入/结果槽和失败边界，才能发布 schedule、放开普通方法准入。见 host-pipeline.md“接口调用帧的真实资源报告”。

接口资源报告已有调用点映射 API `cleanup::lower_method_call_cleanup`，保留完整方法身份，将输入/返回清理映射到 prepared 槽，并合并动作相同的谓词失败边界。真实报告形成的片段经参考执行器和 LLVM O0/O3 验证，717 项相关检查通过。尚需与调用者局部资源、外围构造、输入来源及调用上下文组合，才能自动发布完整 Schedule；当前片段不能独立授权普通方法调用。见 host-pipeline.md“接口资源报告映射到实际调用槽”。

高阶接口资源查询已通过 `analyze_method_cleanup_with_context` 接收并保留调用者提供的代码、枚举、效应和借用前提。测试确认回调身份传至实际实现，并在后续实现资源分析中拒绝重复消费仿射捕获。顶层报告仍可能含延期/子调用义务，调用槽映射拒绝尚未展开的上下文；完整调用者组合未完成。397 项相关检查通过，见 host-pipeline.md“高阶接口帧的资源前提传递”。

普通直接方法的非组合内存路径现自动生成接口后提失败清理：返回 prepared 槽、caller 局部及外围构造由 moves 统一规划，scalar/void 也发布显式空边界；LLVM 准入已接线。checked 源码构建覆盖 owned Token 返回与 caller/pending owner，生产 schedule 经参考执行和 LLVM O0/O3 验证失败路径。724 项相关检查通过。组合内存与高阶上下文的普通方法准入仍待接通，详见 host-pipeline.md“普通调用路径自动发布接口出口清理”。

普通直接接口调用现在为选中实现保留已准备高阶输入并生成资源子上下文，同时保留原接口 entry/result 边界。命名/无捕获回调已 checked 原生执行；消费仿射捕获一次通过生产资源计划和参考执行，重复消费拒绝。LLVM 持久捕获状态写回仍未实现。726 项相关检查通过，详见 host-pipeline.md“普通接口调用传递实现的高阶资源上下文”。

LLVM 现把闭包捕获的 product 构造标记存入环境，调用体消费与外层条件清理共享状态；已解除 capture-transfer 的统一后端拒绝。接口输入的单次仿射捕获消费、嵌套闭包及条件部分消费已原生执行，析构计数经 O0/O3 验证；重复消费/部分拥有值整体转移仍拒绝。719 项相关检查通过，见 host-pipeline.md“LLVM 闭包环境持久构造状态”。

临时拥有型闭包调用的 LLVM 环境现与残余 cleanup 共用保留槽，消费状态不再留在独立副本。接口工厂返回 owned Task 的绑定/立即调用均已 checked 原生执行；临时调用体及实参求值失败的析构计数通过参考执行和 O0/O3 验证。721 项相关检查通过，见 host-pipeline.md“临时闭包调用与残余环境共用存储”。

- 错误结果开发入口验收发现：`typealias Symbol=.Symbol'builtin; typealias Void=.void'builtin; fn run()->!Symbol Void {do {}}` 的 void 成功值注入被 elaboration 以 unresolved type equality 拒绝（期望错误结果、实际 void，报告 `/tmp/luna-tools-c0__erm6/result.json`）。同一结果类型的 `error .ready` 可编译执行。后端没有绕过此检查；恢复前端时检查零大小成功值的注入路径。

- 已修复上项 void 成功值注入缺口：省略 block 尾值与裸 return 生成显式 Success ErrorConvert；无 else 的 if 先保留其 void/丢弃语义，再注入结果。Never 路径不虚构成功返回。错误表面、CLI、host 共 672 项通过，含 !E bool / !E Never 拒绝例、参考执行与 O0/O3 资源清理；详见 host-pipeline.md“void 成功值与错误结果”。一般聚合内错误行扩宽等其它续接项未因此解除。

- 字符串视图续接边界：`str` 不能按普通 pointer-free scalar 接入。正式契约 `docs/resource-management/moves-and-initialization.md` 要求 view 依赖字符分配，移动 String 描述符不能改变该依赖，而只返回 view、清理剩余 owner 必须拒绝。当前 `verification/memory.rs::owned_fields` 明确拒绝 str 布局，`PointerInputLeaf` 只承载指针叶的 pointee/字段路径，没有字符串范围的底层分配来源。后续需同步内存摘要、调用输入/结果来源传递、资源清理与运行时视图表示；不能只将 Str 添入 intrinsic_pointer_value 或由 Copy 消除依赖。本轮保留拒绝行为，没有发布 str Eq。

- Unsafe 签名模板与 quote 展开续接：实例现继承模板声明的 unsafe 责任，不能因实例只写 fn/inline fn/quote fn 而消失。直接 unsafe quote fn 已接通，在展开删除 Call 节点之前，对调用者与保留的 quote 主体执行词法责任检查；此检查也用于后续实例选择后的 retained quote 展开。unsafe comptime fn 的 staged admission 仍未实现。IIR 参考执行器构造 callable 时统一使用 Function::callable_type()，同时保留阶段与 unsafe 包装，不再自行拼装不完整的类型。模板、quote、纯度、CLI 与 callable identity 验证证据见 host-pipeline.md 的对应收尾记录。

- 泛型浮点字面量续接：`T == f32 or T == f64` 约束下，调用 copysign(value, 0.0) 中的字面量仍被 float_proxy 拒绝为 float literal requires a Float, f32 or f64 target；真实 math 包初次 abs 尝试报告 `/tmp/luna-tools-kr48mdds/result.json`。该限制不能通过把 T 选成某个固定浮点宽度来绕过；后续需保留待特化的字面量形成证据。本轮 abs 改为直接包装 LLVM fabs，无字面量形成，不宣称该缺口已解决。

- 本轮重验完整 sysroot，仍因 panic、String、格式化／I/O 等未接通名称失败，报告 `/tmp/luna-tools-ipph24hj/result.json`。未删除库声明或用空实现掩盖问题。

### 泛型浮点字面量：精确形成对象已抽出，定义侧接入未完成

新增 nir::float_formation::FloatFormation，保留 DecimalLiteral 的精确十进制和独立符号；parse 与每次 form 使用调用方预算。form 只接受具体 Float/f32/f64，不推断或授权泛型域，不改变 normal-form 类型身份。Display 为 `(float-formation (decimal-literal ...))`。现有 program::float_proxy 已改为调用该对象，避免后续延迟路径与立即路径各自实现转换。对象本身不是 Expr、completed value 或可执行字面量。

后续仍需：在泛型定义检查中证明允许的具体浮点域；在残余表达式中保留形成对象与期望类型的关联；实例化完成类型替换后形成对应 Literal 并再次检查；IIR admission 拒绝任何未完成形成。不能把精确 Float literal 直接标成 T，不能先形成 f64 再转换 f32，也不能从 opaque normal-form 的函数体推测其类型域。本轮没有解除泛型源码的现有拒绝。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p nir --test float_formation` 退出 0，3 项通过（`/tmp/luna-tools-rhpvw7r6/result.json`）；覆盖独立宽度选择的二次舍入反例、负零、错误域、f32 溢出后仍能形成 f64、预算失败后的恢复、Display 和 clone 后的结果。初次测试使用了缺少小数点的 1e40 字形，解析拒绝；改为现行 1.0e40 后通过。`python3 -B development/check.py --timeout 300 -- test -p luna --test exact_floats --test float_ranges --test host_intrinsic_protocol` 退出 0，16+16+24 项通过（`/tmp/luna-tools-wdvolzl_/result.json`）。格式化退出 0（`/tmp/luna-tools-oyrr_mhh/result.json`），git diff --check 通过。dev 未提交。

### 泛型浮点字面量：f32/f64 域已接通

上一节的残余接入已完成。NIR Literal::PendingFloat 保存经过词法解析的十进制 spelling 与独立负号，Display 为 `(pending-float (negative ...) (spelling "..."))`；复制预算计入文本长度。定义侧只在 PTC 证明目标类型等于 f32 或 f64 后构造残余字面量，不给无约束 T 或 opaque normal-form 头指定表示。

materialization 在类型重写后调用 FloatFormation::complete，以当前预算解析并直接形成目标 Literal；失败只影响当前私有候选。默认值行为检查识别其必为标量的形成行为，不要求从泛型 T 获得用户 Copy 证据，实际域检查／形成仍不可跳过。comptime interpreter 在具体 f32/f64 类型下使用相同形成逻辑与共享预算。IIR validation 拒绝全部 PendingFloat，即使其注解伪造为具体类型；未形成值不能因此进入 LLVM。

回归涵盖一个定义的 f32/f64 独立实例、二次舍入反例、两种负零、泛型默认值、comptime invocation、f32 溢出后 f64 恢复、低预算失败后的恢复、无约束域／包含 bool 的域／opaque 目标拒绝，以及手工残留到 IIR 的节点拒绝。当前泛型域只开放 f32/f64；包含精确 comptime Float 的泛型联合域尚未开放。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test exact_floats --test float_ranges --test default_calls --test pure_callbacks --test host_intrinsic_protocol` 退出 0，174 项通过（`/tmp/luna-tools-siop4vgv/result.json`）。其后接通解释器形成并新增 comptime 用例，再运行 `test -p luna --test exact_floats` 退出 0，22 项通过（`/tmp/luna-tools-sdg3isvk/result.json`）。原生舍入／负零正例实际通过 O0/O3；默认值正例另经过原生执行。首次源码负零紧贴左括号触发块注释词法，修正空格后通过；首次默认值检查因泛型 Copy 域不明而失败，随后增加仅限 PendingFloat 的标量形成分支，未放宽其它值的 Copy 检查。格式化退出 0（`/tmp/luna-tools-xe1xuqhd/result.json`），git diff --check 通过。dev 未提交。

### 精确 Float 泛型域的 comptime 形成

上一节保留的 Float 联合域限制已解除：定义侧允许证明 T 属于 Float/f32/f64 的类型命题，解释器在具体 Float 类型下也经 FloatFormation 形成精确 Literal。仍不把 Float 形成规则变成 runtime 存储许可。新增混合域 comptime 函数回归验证超出机器精度的十进制保持精确，同一定义的 f32 实例直接舍入，Float 负零的符号保留；有效 runtime 包装实际经过 LLVM 执行。

拒绝回归现在断言具体阶段诊断 `comptime-only value cannot inhabit runtime storage`，避免仅因其它错误编译失败就宣称边界受保护。docs/comptime/index.md 增加对应泛型示例与形成／阶段说明。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test exact_floats --test float_ranges --test default_calls` 退出 0，24+16+33 项通过（`/tmp/luna-tools-r1bepdrj/result.json`）。格式化退出 0（`/tmp/luna-tools-eibps1rj/result.json`），git diff --check 通过。未重复全 workspace 测试，dev 未提交。

### 泛型浮点形成与闭包的局部常量转发

新增组合用例暴露：泛型闭包内 `let value:T= -0.0` 可先进入 staged closed-local forwarding，再到普通 annotation normalization；原先 canonical constant 入口因此拒绝 PendingFloat（`/tmp/luna-tools-alqwkoh4/result.json`）。现在 fold_input 在具体 Float/f32/f64 域下执行有预算的形成，返回真正的 Literal 后再供 staged 转发使用。canonical_input_with_layouts 继续拒绝未形成节点；IIR 的禁止残余规则不变，不以扩大 canonical 输入种类来绕过形成。

四种源码边界已验证：泛型 quote 调用、返回无捕获闭包且其主体有局部浮点常量、泛型默认回调、捕获泛型浮点局部值的返回闭包。各自的 f32/f64 负零经参考执行及 O0/O3 LLVM 原生执行保持符号。IIR 局部测试验证形成前拒绝 canonical admission、形成后接纳、来源范围保留、无效／未完成类型拒绝，以及预算失败后恢复。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p iir --test float_formation` 退出 0，2 项通过（`/tmp/luna-tools-95t4w448/result.json`）。`python3 -B development/check.py --timeout 300 -- test -p luna --test exact_floats --test quote_functions --test default_calls` 退出 0，25+104+33 项通过（`/tmp/luna-tools-uddbf_f9/result.json`）。捕获用例加入后另行运行 `--test exact_floats generic_float_formation_survives`，退出 0（`/tmp/luna-tools-bheh5gms/result.json`）。格式化退出 0（`/tmp/luna-tools-iu8rj7qd/result.json`），git diff --check 通过。未执行全 workspace 测试，dev 未提交。

### 浮点形成失败诊断与失败节点保留

新增 FormationError，保存 SourceRange、具体目标 Term 和底层原因，Display 为 `(float-formation-error (source ...) (target ...) (reason (string ...)))`。普通残余形成与 staged fold 使用相同诊断；浮点范围失败不再显示底层整数转换的 integer does not fit 文本，形成预算／表示限制也使用浮点上下文。失败时不替换当前 PendingFloat 节点，调用方仍按原私有候选规则处理整棵树；不承诺已处理的其它节点回滚。

新增底层测试检查两个入口诊断完全一致、准确来源与 f32 目标、失败节点保持、改为合法 f64 后恢复。源码级实例化测试断言具体 float-formation-error 和有限浮点范围原因，避免只检查任意错误。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p nir -p iir --test float_formation` 退出 0，4+3 项通过（`/tmp/luna-tools-mjsczhf5/result.json`）；`python3 -B development/check.py --timeout 300 -- test -p luna --test exact_floats` 退出 0，25 项通过（`/tmp/luna-tools-7j432aot/result.json`）。格式化退出 0（`/tmp/luna-tools-xowz2qvf/result.json`），git diff --check 通过。dev 未提交。

### 泛型整数字面量形成：局部验收完成

已补验工作区中的 PendingInteger 路径：定义侧证明目标属于 Integer／固定宽度整数／usize／isize 的类型联合域，保留原始字形与负号；具体实例才按目标形成。comptime interpretation、普通 materialization 和 staged 局部常量转发使用同一形成函数。PTC 只证明类型域，数值范围不交给 PTC；无约束 T、含 bool 的联合域及 opaque normal-form 头不能借此获得整数表示。泛型 BitVec 不在此路径的接纳范围内。

新增源码回归验证有符号最小边界、无符号最大边界、默认参数、溢出／负数转无符号拒绝后合法实例恢复、低预算后恢复、精确 comptime Integer、捕获泛型常量的闭包以及 IIR 未形成节点拒绝。两组正例经过参考执行及 LLVM O0/O3。仅编译时 Integer 仍不能进入 runtime 存储。底层测试另覆盖 i128 最小值、u128 最大值及溢出、超过 u128 的精确 Integer、显式 32/64 位目标差异、缺失目标拒绝、失败节点保持及更换合法目标后重试。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test integer_formation --test integer_contracts --test exact_floats --test default_calls` 退出 0，6+14+25+33 项通过（`/tmp/luna-tools-qqlulttw/result.json`）。`python3 -B development/check.py --timeout 300 -- test -p nir --test integer_formation` 最终退出 0，4 项通过（`/tmp/luna-tools-0wc3bej8/result.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-6ed_4bq7/result.json`），git diff --check 通过。未运行全量测试；dev 工作区未提交。

### 整数形成的 staged／平台目标组合

新增组合回归确认泛型 quote 展开和默认回调的负整数字面量经参考执行及 LLVM O0/O3 保持值；同一泛型定义在显式 32 位包中拒绝超出 usize 的数值，在 64 位包中成功，两者的 u64 实例都保持合法。staged forwarding 只在具体形成之后接纳 canonical input，未形成节点不会因具体类型注解直接成为 canonical value。

新增 integer_formation::FormationError，诊断以 S-expression 保存来源、类型与目标布局。缺失 usize/isize 布局明确报告需要 target pointer-width；普通完成与 staged folding 对同一输入返回相同诊断。数值范围失败保留具体目标布局，形成失败不修改输入节点，独立重试不受影响。

本轮工作目录 `/home/fin/luna`。最终 `python3 -B development/check.py --timeout 300 -- test -p nir -p iir --test integer_formation` 退出 0，4+2 项通过（`/tmp/luna-tools-l5dtpsh3/result.json`）；诊断修改后重跑 `test -p luna --test integer_formation --test target_integers` 退出 0，7+17 项通过（`/tmp/luna-tools-t0ntagyc/result.json`）。格式化退出 0（`/tmp/luna-tools-br7ldqof/result.json`），git diff --check 通过。dev 未提交；未重复全 workspace 测试。

### Handler 资源帧续接：共享精确查找

IIR Schedule 新增 exact_function，以完整 InstanceKey 和 ContextKey 序列定位唯一具体帧，缺失／重复分别返回可 Display 的 ScopeLookupError。此入口不跟随 alias；handler/default 激活需要自己的已发布资源证据。调用方先计入扫描预算。参考执行器的 handler 帧选择与 LLVM 默认参数帧选择已共用该规则；后者不再通过 find 接受第一个匹配帧。

原生 handler 尚未接通，现有 gate 保持。后续需在每个 arm 的 guard 前进入由 installation/handler/arm 标识的独立帧，隔离 prepared、临时值、pattern order 等状态；正常退出、resume、词法 return/break/continue 以及谓词失败分别执行相应边界，再恢复悬挂请求方。安装者 captures 与请求者 locals 不能混用。对应参考实现位于 comptime_interpretor/src/effects.rs::with_effect_handler_resources；原生 match_with_fallback 当前只有整组 handler 的 resume 目标，尚无每个 arm 的资源帧切换。

验证工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p iir --test cleanup_scopes` 退出 0，3 项通过（`/tmp/luna-tools-r8mtax26/result.json`）；`test -p luna --test default_calls --test effect_memory_transfers --test effect_spreads` 退出 0，33+85+21 项通过（`/tmp/luna-tools-gvlag6m4/result.json`）；`test -p luna --test host default` 退出 0，16 项实际原生回归通过（`/tmp/luna-tools-0fchyhss/result.json`）。格式化退出 0（`/tmp/luna-tools-p71pd44q/result.json`），git diff --check 通过。dev 未提交。本轮完成资源帧选择共用基础，未宣称原生 handler 资源清理完成。

### 原生局部资源帧切换

新增 host/llvm/frames.rs::LocalFrame，将 cleanup schedule 与 locals、live、temporaries、temporary_live、prepared、pattern_orders、temporary_fields、loops 一起悬挂和恢复。默认参数执行／失败展开已改用此机制，移除单独维护 schedule 的双份状态。enter_local_scope 在任何状态切换前检查预算并精确选择具体上下文；失败保留调用方的帧和存储。LLVM 指令、SSA 序号、receipt-path 表和 return/resume 目标属于整个机器函数，不随局部资源帧重置。

内部原生回归在嵌套默认清理之间插入 EffectHandler 上下文切换，验证缺失／重复／预算失败不改变当前帧，同一 LocalId 在子帧获得新存储、退出后恢复原存储；失败路径实际输出 CBA，正常路径不析构这些失败清理对象，O0/O3 均验证。此测试直接验证帧机制，不代表源码 handler 的生命周期 hooks 已接通。下一步仍需在 match_with_fallback 的逐 arm 路径安装资源帧，并接上 HandlerEnd／EffectResume／词法退出和失败传播；原生 handler gate 保留。

本轮工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib native_default_failure` 退出 0（`/tmp/luna-tools-4w0nppq4/result.json`）；`test -p luna --test host default` 退出 0，16 项通过（`/tmp/luna-tools-_gtsu6to/result.json`）。格式化退出 0（`/tmp/luna-tools-od3ita6x/result.json`），git diff --check 通过。dev 未提交。

### Handler 逐 arm 帧与正常／resume 边界

原生 DirectHandler 现在保留安装时的实例、上下文、installation 和 handler 序号；match_with_fallback 接收该身份，在每个 arm 的 pattern/guard 前按完整安装者身份进入具体资源帧，随后恢复被暂停的帧。帧查找不以请求方当前上下文拼装。普通 arm 返回执行 HandlerEnd，显式 resume 在跳转前执行 EffectResume。普通 match 继续使用原有 ScopeEnd 行为。

新增底层 LLVM 回归，构造安装者与请求者不同上下文、同一 LocalId 的情形：第一个 arm guard 为 false，第二个 arm 正常返回或 resume；只选中 arm 的析构 B 执行，恢复请求方后析构 A，O0/O3 均输出 BA，返回值为 true。此测试手工提供具体清理计划以隔离降码，不是源码 handler 的完整安全验收。

生产路径的 checked handler gate 仍保留。剩余工作包括失败传播时展开 handler 与暂停请求者的资源帧、词法 return/break/continue 的目标层清理，以及 cleanup admission 对 handler 子上下文及每个 boundary 的完整核对。不能仅因正常／resume 路径通过而开放一般 handler。

本轮工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host` 退出 0，46+374 项通过（`/tmp/luna-tools-92i1tss_/result.json`）。新增测试首次编译因误用 Pattern::Bind 变体名失败，改用现有 Binding { binding, inner } 后通过；定向报告 `/tmp/luna-tools-6b_49_cs/result.json`。格式化退出 0（`/tmp/luna-tools-wbqtgmwd/result.json`），git diff --check 通过。dev 工作区未提交。

### Handler 谓词失败与统一暂停帧栈

默认参数帧与 handler 请求帧现在共用 frames::UnwindFrame 栈，保存完整 LocalFrame、来源和失败边界。dispatch_direct 在替换为安装者 locals/loops 之前悬挂请求方帧；正常结果或降码错误后恢复该帧。handler 内的 PredicateFailure 先执行当前 arm 清理，再按反向求值顺序展开暂停请求方的 PredicateFailure 与外层默认参数的 DefaultFailure。默认参数嵌套到 handler 或 handler 嵌套到默认参数时不再依靠两套分离栈排序。

HandlerInvocation 保留实际 request 来源，fallback 沿用该来源；direct-passing 安装求值使用安装表达式来源。新增扩展回归直接通过 dispatch_direct 进入安装者上下文：选中 arm 为 B，请求方为 A，请求方外层默认帧为 C。正常返回／resume 输出 BA；handler 内谓词失败输出 BAC 后终止，O0/O3 都验证。同一 LocalId 在各层有不同物理存储，恢复后原调用方槽位和统一栈深度保持。

这仍是具体 IIR 清理计划的底层降码验收。checked handler 源码准入 gate 保留；词法 return/break/continue 和 cleanup admission 尚待接通。另需核对 EffectRequest 参数构造的 prepared 存储与失败退出边界，不能因本轮谓词展开通过就开放所有资源参数传递。

本轮工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host` 退出 0，46+374 项通过（`/tmp/luna-tools-f56vcyxh/result.json`）。定向 dispatch 原生测试报告 `/tmp/luna-tools-d9jgmjge/result.json`；格式化退出 0（`/tmp/luna-tools-7mea0t31/result.json`），git diff --check 通过。dev 未提交。

### EffectRequest 参数构造的资源保存与交付

LLVM EffectRequest 逐实参求值后调用 retain_prepared_field，使用 request 来源、参数位置和 value 来源定位已发布的 prepared 责任。后续参数失败时，先前构造成功的参数仍能由调用方清理；全部参数构造完成并组成 packet 后，finish_prepared_construction 清除调用方的持有标记，再进入 handler。该顺序对齐参考执行器的 retain_prepared／transfer_prepared。

底层原生回归从直接 dispatch 扩展为完整 EffectRequest 表达式，包含两个实参：第一项构造 B，第二项含运行时谓词。后项失败时不进入 handler，清理参数 B、请求方 A、外层默认帧 C；handler 内失败同样按 BAC 退出，但 B 的责任已转交 arm；正常返回与 resume 均为 BA。六种组合在 O0/O3 下执行通过。最初 fixture 错把转移后的 Prepared 责任保留到调用方正常退出，现有 live 检查正确 trap；修正为仅后续参数失败路径持有 Prepared 后通过，没有放宽重复清理保护。

一般 checked handler 的源码准入 gate 仍保留，词法控制流和完整 admission 续接未完成；本轮不宣称所有 effect 参数资源交付场景均已支持。

本轮工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib handler_arm_cleanup` 最终退出 0（`/tmp/luna-tools-dmmvntd_/result.json`）；`test -p luna --test host effect` 退出 0（`/tmp/luna-tools-rbmc9bdz/result.json`）。格式化退出 0（`/tmp/luna-tools-bbmxqgz1/result.json`），git diff --check 通过。dev 未提交。

### Handler 词法 return 的清理与后置条件

原生 ret 在当前资源上下文为 EffectHandler 时先执行该 Return 的 TemporaryEnd／Return 清理，再沿暂停请求帧反向展开。中间 handler 请求帧使用 EffectEscape；到达拥有此机器函数的非 handler 帧后，恢复其 locals 与 schedule，检查函数后置条件，执行 EffectReturn 并返回保留的 SSA 结果。后置条件失败时统一清理栈只保留尚未展开的外层帧，不能重复析构已经离开的 handler。代码生成结束后恢复 emitter 的编译状态，便于继续生成其它分支。

底层回归新增单层和嵌套 handler 的 return、目标函数后置条件成功／失败。后置条件读取局部 A 的实际值 65，而不是同 LocalId 的 handler B 或中间 handler D，证明恢复了正确资源帧。单层正常和失败路径均只输出 BA，嵌套均只输出 BDA；失败路径随后终止，成功返回 i32 0。所有新增组合实际通过 LLVM O0/O3。

跨默认参数求值的 handler return 仍明确拒绝；break/continue 与生产 cleanup admission 尚未完成。一般 checked handler gate 保持，不能把底层返回测试解释成整个源码效应资源系统已开放。

本轮工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host` 退出 0，46+374 项通过（`/tmp/luna-tools-weuwx56l/result.json`）；新增嵌套返回定向报告 `/tmp/luna-tools-tz6we8ud/result.json`。格式化退出 0（`/tmp/luna-tools-7jci8wxs/result.json`），git diff --check 通过。dev 未提交。

### Handler break／continue 的词法目标与资源范围

原生循环表改为 LoopTarget，保留入口／出口标签以及声明循环时的完整实例与资源上下文。handler 捕获该目标身份；在 arm 内新建的循环归属当前 arm，不能因为也能看到外层循环标签就误判为跨帧退出。

Break／Continue 的目标与当前 scope 一致时仍执行本地边界。跨 handler 时，在暂停帧中按循环 LocalId、标签及完整拥有者身份定位目标；当前 arm 清理 Break／Continue，中间帧清理 EffectEscape，目标帧清理 EffectBreak(id)／EffectContinue(id)，然后跳往出口／入口。生成其它分支所需的编译状态会恢复。跨默认参数求值帧仍明确拒绝，未静默当作普通 loop 跳转。

底层原生回归增加单层、嵌套两种 break/continue。目标基本块用 E/H 标记，结果分别 BAE、BAH、BDAE、BDAH，验证清理顺序与实际分支去向。所有既有 handler 场景还插入 arm 自己的局部循环，验证其 break 不触发请求方清理。O0/O3 均通过。生产 checked handler gate 保留，下一步是完整 cleanup admission 对 handler 上下文和边界的接纳；不能直接删除 gate。

本轮工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host` 退出 0，46+374 项通过（`/tmp/luna-tools-czwg1hao/result.json`）；新增循环转移定向报告 `/tmp/luna-tools-j6o4_l3_/result.json`。格式化退出 0（`/tmp/luna-tools-0b_rm12t/result.json`），git diff --check 通过。dev 未提交。

### 泛型平台整数算术

原生整数族判断和程序验证的有限类型域枚举加入 usize/isize；有符号取负加入 isize，无符号除余仅加入 usize。枚举保留平台类型本身，不替换为宿主 i64/u64；实际字面量形成和程序算术证明继续使用 package.target。有符号除余仍要求显式舍入协议。

新增 target_integers 回归覆盖泛型加减乘、无符号除余、取负、无目标拒绝、32/64 位差异，以及未被调用定义的溢出／除零拒绝；原生用例以 assumes 检验计算结果，避免仅以进程成功判定值正确。后续已通过 release 配置完成该组执行验收，见 [host-pipeline.md](host-pipeline.md) 的“泛型平台整数算术与低空间验收”；并未使用宿主位宽替代无目标输入。

2026-10-24，工作目录 `/home/fin/luna`：经 `development/check.py --timeout 900 --` 执行 `test -p elaboration --lib finite_domains`，3 项通过（`/tmp/luna-tools-86nhqo85/result.json`），包括平台与定宽类型身份分离、关联剪枝和预算恢复；`check -p luna --test target_integers` 退出 0（`/tmp/luna-tools-mf1asgaa/result.json`）。全量格式化退出 0（`/tmp/luna-tools-vcrmjvhf/result.json`），git diff --check 通过。两次 target_integers 执行尝试在链接时遭遇磁盘配额，日志包装器亦无法保存完整结果（`/tmp/luna-tools-ql7pulks`、`/tmp/luna-tools-7lf2jml4`），不计为测试结果。已清理授权范围内的 Cargo 产物，历史示例编译器副本清理仍待授权。dev 未提交。

后续执行证据（2026-10-24）：经 development/check.py 运行 `test --release -p luna --test target_integers`，21 项通过（`/tmp/luna-tools-q6wfin0r/result.json`）；`test --release -p luna --test host generic_`，29 项通过（`/tmp/luna-tools-wkefk15y/result.json`）。扩展后的泛型整数示例绑定真实 ops 协议，LLVM O0/O3 均得到预期 stdout（`/tmp/luna-tools-_khr7cbo/result.json`）。格式检查 `/tmp/luna-tools-zs__v3eh/result.json` 退出 0；本次以 release 配置恢复定向验收，未删除历史编译器副本，未宣称完整 host 组已重跑。

### 运行时计算接收者的阶段修复（2026-10-24）

字段域推测检查保持原调用方阶段，解除括号 do/if/when/match 投影误要求 comptime 的问题。12 项 computed host 测试与两个此前失败的资源定向测试通过，包含 LLVM O0/O3 和 Type 阶段拒绝回归。具体命令与报告见 [host-pipeline.md](host-pipeline.md) 顶部。资源全组未重跑；Tuple 参数解析和后置条件证明缺口仍保留。dev 未提交。
