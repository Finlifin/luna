# Flurry (Luna) 编译器开发进度评估报告

- **评估时间**：2026-10-18
- **工作目录**：`/home/fin/luna`（分支：`dev`，基线提交：`196cc0ad` + 当前工作区未提交演进）
- **评估依据**：[docs/README.md](file:///home/fin/luna/docs/README.md) 正式语言规范、[development/implementation-plan.md](file:///home/fin/luna/development/implementation-plan.md) 阶段依赖图、[development/README.md](file:///home/fin/luna/development/README.md) 实测记录、各 crate 源码与测试集。

---

### 2026-10-19：trait 父级值子句的生成器捕获

- 生成器声明记录父级 Type／comptime-value binder，实例化使用带 sort 的参数替换。trait 默认方法中的父级值按其声明域成为编译期数据绑定，依赖前置 Type 的域先完成替换；值捕获不加入 PTC 的类型相等假设，错误 sort 明确拒绝。
- 正文和程序契约共享捕获环境，嵌套调用后恢复外层值。方法默认值在完整生成器帧内接受定义检查，修复父级值缺失时的过早检查；显式提供实参仍不能掩盖错误默认值。trait 继承保留父级值，索引类型签名继续检查 normal-form 身份。
- 新增 8 项源码测试，覆盖两个固定值 trait 实例、父级值供应方法默认值、依赖值域、程序前置条件、嵌套实例恢复、继承、未调用的错误默认值，以及带父级值索引的代码签名与不匹配实现拒绝。
- 工作目录 `/home/fin/luna`，dev，未提交。`python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test execution --test quote_functions --test instantiation --test normal_patterns`：1902 项通过，退出 0，报告 `/tmp/luna-tools-527dqkm8/result.json`。相关 Rust 已格式化，`git diff --check` 通过。
- 本项完成的是 trait 父级值的代码捕获；nominal 容器及泛型 impl 的值参数表示／推断仍需扩展。引用接收者和代码结果绑定契约也仍待实现。

### 2026-10-19：生成器方法的值子句与域保真

- `CodeFunctionSignature` 增加有序 clause_domains，统一保存生成器和方法契约的子句域，移除 Generator 中的重复域列表。复制预算、类型映射、依赖遍历及 S-expression Display 同步覆盖域；接口比较连未使用的值子句也比较，Type／comptime-value 类别不匹配在 trait 契约入口拒绝。
- 方法兼容性替换按声明域建立 binder sort，不再将所有方法子句当作 Type。固有／trait 方法支持显式值输入、公开默认值与名称、依赖前置 Type 的值域，以及值接收者和默认正文中的值子句转调。调用继续保留公开与实现两侧环境。
- 修复程序规格分类遗漏 ResolvedClauseParam：读取值子句的布尔条件进入程序规格检查，不进入 PTC；normal-form 类型应用中的值索引仍保留为类型身份。新增拒绝例验证 Tagged<false> 不因函数体相同而等于 Tagged<true>。
- 新增 8 项源码测试及 1 项共享 IR 测试，覆盖实际结果、默认值、依赖域、类型／值域不匹配、程序前置条件、接收者转调、带值索引的类型命题，以及域的映射／预算／dump。移除已开放值子句的旧声明拒绝例。
- 工作目录 `/home/fin/luna`，dev，未提交。`python3 -B development/check.py --timeout 900 -- test -p nir -p iir -p comptime_interpretor -p elaboration`：886 项通过，报告 `/tmp/luna-tools-javif67c/result.json`。`python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test execution --test quote_functions --test instantiation --test normal_patterns`：1894 项通过，报告 `/tmp/luna-tools-yfph88yj/result.json`。均退出 0；相关 Rust 已格式化，`git diff --check` 通过。
- 父级值子句的实现选择／捕获、引用接收者与代码结果绑定契约仍待实现；本项开放的是方法自身的值子句，不表示这些剩余路径已经完成。

### 2026-10-19：生成器方法的值接收者

- 生成器签名支持 `self`，其域由 Self 类型及既有不可变数据规则检查。固有／trait 调用将接收者放在首个数据实参位置，复用普通生成器的参数准备、默认值、展开与属性路由；接收者先于剩余实参求值一次，runtime 对象不能进入 required-comptime 输入。
- 方法选择的类型预检支持 nominal 构造、泛型构造、已有变量、Self、已知函数结果和 intrinsic 字面量；预检不执行构造字段或接收者生产函数。trait 默认正文可用 `self.as(Trait)` 转调，公开接口与实现中的接收者使用相同的映射和契约流程。
- 新增 10 项源码测试，覆盖固有／trait 方法、泛型 Self、默认正文转调、接收者生产函数的次数和顺序、代码上下文连接、展开与属性调用、接收者前置契约、primitive trait 实现，以及 runtime 输入／缺失接收者／将静态方法当实例方法调用的拒绝。计数 fixture 的解引用与乘法使用分离 token；primitive 正例使用本地 trait，遵守固有 impl 的本地 nominal 规则。
- 工作目录 `/home/fin/luna`，dev，未提交。`python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test execution --test quote_functions --test instantiation`：1876 项通过，退出 0，报告 `/tmp/luna-tools-hn044t1v/result.json`。相关 Rust 已格式化，`git diff --check` 通过。
- 引用接收者 `*self` 仍明确拒绝，尚需独立接入编译期内存／别名模型；父级与方法值子句、代码结果绑定契约也仍未接通。本项不扩大 runtime 资源或引用值可进入代码生成阶段的范围。

### 2026-10-19：静态 trait 生成器分派

- 接通显式 trait 视图上的静态代码方法：确认请求视图、查找父 trait 成员、选择实现并分别建立公开签名与实现的 Type 捕获环境。共享声明缓存保持符号形式，调用局部目标与环境在成功／失败后恢复。
- 调用路由使用 trait 声明的参数名、默认值和方法子句；准备输入并检查公开前置契约后，将数据与代码实参映射到实现帧。输入生产函数不重新执行，代码上下文支持实现内部改名，结果回到公开接口后检查其后置契约并恢复默认连接。
- 默认 trait 正文直接执行；抽象 Self 和有 trait 约束的泛型生成器可以在定义期检查调用接口，真正 invocation 才选择具体实现。父 trait 实现不能替代请求的子 trait 资格。展开参数与属性调用复用既有路由。
- 新增 13 项源码测试，并将旧 trait 调用拒绝例改为缺少实现的拒绝例；14 项专项覆盖默认正文、公开参数与默认值、开放上下文、隐式连接、单次输入执行、前后置契约、trait／方法 Type 参数、泛型 impl、抽象 Self 转调、泛型延迟选择、父 trait 视图以及展开／属性调用。
- 工作目录 `/home/fin/luna`，dev，未提交。`python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test execution --test quote_functions --test instantiation`：1866 项通过，退出 0，报告 `/tmp/luna-tools-65y1sfyw/result.json`。相关 Rust 已格式化，`git diff --check` 通过。
- 接收者、父级／方法值子句和代码结果绑定契约仍未接通；这里未声称程序契约定义期细化证明、跨包代码模板导出或 quoted pattern/arm 已完成。

### 2026-10-19：泛型容器的静态代码方法

- 固有生成器方法调用从实现选择证据取得父级 Type 实参，替换数据参数、代码上下文与结果。调用局部捕获环境和共享声明缓存分离；正文、Self、默认参数及方法类型契约使用同一环境，递归调用与失败路径恢复旧绑定。Generator 的 S-expression dump 增加 captures，复制预算覆盖捕获类型。
- 无标注代码绑定可以从显式 nominal Type 实参取得分类证据；预检不执行默认值或类型生产函数，真实调用仍检查容器形成条件与实现证据。父级参数和方法自己的 Type 参数分别保存；默认 Type 参数可以引用父级参数。
- 新增 8 项源码测试，覆盖嵌套／独立 impl、开放代码重映射、正文与数据默认值、父级／方法参数分离、递归切换实例、捕获类型约束、父级类型默认值、Self 与 nominal 结果、容器 trait 约束拒绝。移除已支持泛型容器调用的旧拒绝例，保留 trait 分派拒绝例。
- 工作目录 `/home/fin/luna`，dev，未提交。`python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test execution --test quote_functions --test instantiation`：1853 项通过，退出 0，报告 `/tmp/luna-tools-iysvb_wt/result.json`。相关 Rust 已格式化，`git diff --check` 通过。
- trait 生成器方法调用（含默认实现的契约与上下文映射）、父级值子句和接收者仍未实现；本项提供父级 Type 环境基础，不代表这些调用路径已开放。

### 2026-10-19：固有静态生成器方法调用

- 接通无泛型容器上的嵌套成员和独立 impl 静态生成器方法；与普通方法共用固有实现选择、证据重放和访问检查，选定后复用生成器的 Type 参数推导、默认实参、展开包及属性调用。没有构造伪普通 callable。
- 无类型标注的代码绑定通过已知 nominal 主语的实现查询识别成员类别；分类不执行类型生产函数、默认参数或实参，真正调用重新检查实现证据。不同类型上的同名普通方法保持原有路径。
- 新增 8 项源码测试，覆盖闭合／开放代码结果、上下文重映射、显式／推导方法 Type 参数、无标注 const、单次实参执行、默认值、展开参数、属性调用、类型约束、私有成员访问、runtime 数据与普通函数值逃逸拒绝。泛型容器、trait 生成器分派、receiver 和方法值子句仍未实现。
- 工作目录 `/home/fin/luna`，dev，未提交。`python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test execution --test quote_functions --test instantiation`：1845 项通过，退出 0，报告 `/tmp/luna-tools-zsge660j/result.json`。相关 Rust 文件已格式化，`git diff --check` 通过。

### 2026-10-19：枚举载荷的生成器参数类型证据

- 将已知枚举布局接入参数预推断，支持上下文变体的元组／命名载荷、泛型字段替换和嵌套解构；类型复制消耗现有推断预算，预检不执行主语或 guard。
- 验收覆盖两种变体实际执行、回调前置参数推断、主语单次执行与 guard 顺序、缺失覆盖、重复／未知字段、载荷形状拒绝，以及未调用的发散生产函数。完全限定的变体头仍不提供本项提前证据。
- 工作目录 `/home/fin/luna`，dev，未提交。`python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test quote_functions --test instantiation --test normal_patterns`：502 项通过，退出 0；报告 `/tmp/luna-tools-_fbxidav/result.json`。相关 Rust 文件已格式化。

### 2026-10-19：组合模式的生成器类型证据

- 接通 `as` 的内外绑定、嵌套 guard 与 `or` 候选的类型预推断。各候选使用独立临时环境，只合并一致类型，并沿用首候选的出口绑定身份；错误路径同样恢复临时环境。类型复制使用既有预算。
- 验证两个变体的真实执行、嵌套 whole binding、块内元组重解构、不同类型／不同名称候选的拒绝，以及 guard 读取前一实参写入后的状态；runtime guard 不能借类型预检越过 comptime 阶段检查。
- 工作目录 `/home/fin/luna`，dev，未提交。`python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test quote_functions --test instantiation --test normal_patterns`：506 项通过，退出 0；报告 `/tmp/luna-tools-45bx7g0w/result.json`。相关 Rust 文件已格式化，`git diff --check` 通过。此项不表示自定义模式协议、quoted pattern/arm 或生成器方法已完成。

### 2026-10-19：共享代码接口与完整复制预算

- 将生成器内部的表达式代码接口迁入 `nir::code_interfaces::ExpressionCodeInterface`，保留结果类型、runtime 上下文、开放端口、默认连接与 S-expression Display。生成器实例化已使用统一的失败原子类型映射入口，泛型替换不展开 normal-form 头，也不把固定依赖变成开放端口。
- 接口复制预算增加端口名称及连接表的工作量。该变化暴露了发散程序可能先在解释器耗尽预算的诊断差异：现为 `StepsExhausted` 补充明确的 `step budget exhausted` 文本，保留原错误种类与源码位置，未放宽失败测试。
- 工作目录 `/home/fin/luna`，dev，未提交。`python3 -B development/check.py --timeout 900 -- test -p nir --test code_interfaces`：4 项通过，报告 `/tmp/luna-tools-12xq4dw2/result.json`；`python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test quote_functions --test instantiation --test normal_patterns`：506 项通过，报告 `/tmp/luna-tools-64dbpmwf/result.json`，均退出 0。相关 Rust 文件已格式化，`git diff --check` 通过。
- 这是生成器方法的表示层前置工作，方法分派尚未接通。下一步需在方法契约中显式区分普通 callable 与代码接口，衔接接口一致性、继承／实现选择、父级参数替换和接收者求值；不能把代码结果伪装成普通 `Term` 后调用既有分派。

### 2026-10-19：完整生成器签名的共享表示

- 新增共享 `CodeFunctionSignature` 和 `CodeInput`，完整保存数据参数、表达式代码参数、参数绑定及代码输出；同步实现 S-expression Display、统一预算计费与失败原子的全签名类型映射。现有 Generator 已迁入该表示，实例化与复制走共同入口。
- 专项检查混合参数不会因结果类型相同而混淆数据／代码类别，替换覆盖参数和输出上下文，输出替换失败不污染已检查的输入，并验证嵌套接口的预算和 dump。
- 工作目录 `/home/fin/luna`，dev，未提交。`python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test quote_functions --test instantiation --test normal_patterns`：506 项通过，退出 0，报告 `/tmp/luna-tools-9t3814_m/result.json`；`python3 -B development/check.py --timeout 900 -- test -p nir --test code_interfaces`：8 项通过，退出 0，报告 `/tmp/luna-tools-1n1tjmrw/result.json`。相关 Rust 文件已格式化，`git diff --check` 通过。
- 生成器方法尚未接通：现有 `MethodContract.signature: Term` 仍需明确区分普通 callable 与 `CodeFunctionSignature`，并同步实现兼容性、调用选择和继承。当前未解除源码的明确拒绝，也未使用伪造的普通结果类型让契约检查通过。

### 2026-10-19：方法契约显式区分 callable 与生成器签名

- `MethodContract.signature` 改为 `MethodSignature::{Callable, Generator}`。生成器类别保存完整 `CodeFunctionSignature`，泛型替换保留类别与代码上下文，不再要求把它装进普通 `Term`。Display 保留普通方法原格式，生成器输出结构化代码签名。
- 同步 trait 兼容性、具现化、协议校验、NIR 函数摘要、IIR 校验及解释器消费者。普通路径必须显式取得 callable；生成器类别返回具体诊断，不能生成伪造的普通函数摘要或进入运行时分派。摘要在参数替换前检查类别，避免失败类别触发无关替换。
- 测试包含类别保持、泛型实例化、摘要拒绝、伪造 IIR/解释器方法的拒绝与后续有效输入恢复；迁移直接构造／修改 MethodContract 的旧拒绝例，保留其语义预期。
- 工作目录 `/home/fin/luna`，dev，未提交。`python3 -B development/check.py --timeout 900 -- test -p nir -p iir -p comptime_interpretor -p elaboration`：879 项通过，报告 `/tmp/luna-tools-wn8cxss8/result.json`。摘要提前检查类别后，`test -p nir --test code_interfaces --test calls`：17 项复查通过，报告 `/tmp/luna-tools-l7ebvb2c/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test execution --test drop --test cmtt --test quote_functions --test instantiation --test normal_patterns --test list_patterns`：2620 项通过，报告 `/tmp/luna-tools-9tsbqqiz/result.json`。上述完成检查均退出 0，合计 3499 个不同测试；相关 Rust 文件已格式化，`git diff --check` 通过。
- 源码生成器方法仍明确拒绝。后续需要允许无正文的生成器声明、纳入 Self／父级参数、实现代码契约兼容性及上下文重映射，并在代码调用完成后剥离只服务于生成阶段的契约；不能让代码成员阻止同 trait 普通成员的 IIR 使用。

### 2026-10-19：生成器方法的代码签名兼容性分支

- trait 实现检查已按 `MethodSignature` 类别分派，代码签名比较先验证参数类别、数量、端口形状、绑定双射、固定依赖与默认连接，再调用既有类型等价检查。普通 callable 的 binder scope 与原有契约假设路径保留。
- 关联类型方程收集改为遍历签名中的真实类型依赖；收集器的临时 tuple 不是代码签名的类型身份。身份／连接关系由结构检查处理，只有 sort 为 Type 的字段交给 PTC；normal-form 头不展开。共享绑定不得被拆成不同身份，固定依赖不得被抽象成端口。
- IR 级专项覆盖局部改名、默认连接改向、参数类别／数量、无效端口、固定依赖、共享上下文、契约类型假设、normal-form 区别、非法 scope/value 字段和预算失败后的恢复。
- 工作目录 `/home/fin/luna`，dev，未提交。`python3 -B development/check.py --timeout 900 -- test -p luna --test execution --test quote_functions --test instantiation --test cmtt`：1831 项通过，报告 `/tmp/luna-tools-up6s_35j/result.json`。新增 Type-sort 守卫后，`python3 -B development/check.py --timeout 900 -- test -p elaboration --lib program::interfaces::signatures`：8 项通过，报告 `/tmp/luna-tools-rg00wlfi/result.json`。均退出 0；相关 Rust 已格式化，`git diff --check` 通过。
- 源码生成器方法仍未开放，本项证明的是比较器及普通方法回归。无正文声明、Self／父级参数、代码上下文重映射、生成阶段契约剥离和调用分派仍需实现；依赖 runtime identity 的类型不能靠本次结构改名自动视为相等。

### 2026-10-19：源码生成器方法声明、默认正文与普通成员共存

- Generator 正文改为可选，trait 的无正文代码方法进入完整签名／默认参数／契约检查；真正 invocation 缺少正文仍报错。MethodContract 生成器分支保留代码签名与抽象数据上的程序契约，检查谓词不能读取 quoted runtime 上下文。
- 源码已验证无接收者的 trait 声明、默认正文、实现兼容性、方法 Type 参数、父级类型参数、显式 Self 数据域，以及缺失成员／参数类别／上下文／结果类型冲突的拒绝。修复生成器方法 capability 空记录未初始化的内部错误；拒绝例使用 builtin char，确保失败发生于契约检查。
- elaboration 保留完整成员世界，最终普通执行产物移除代码方法槽。普通 trait dispatch 与 required-comptime sibling dispatch 均已执行通过；未调用的错误默认参数／正文仍拒绝。
- 方法调用分派、receiver、方法值子句域及代码结果绑定契约仍未实现，入口明确拒绝；没有把未完成的分派或跨包生成器导出声称完成。
- 工作目录 `/home/fin/luna`，dev，未提交。`python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test execution --test quote_functions --test instantiation`：1837 项通过，退出 0，报告 `/tmp/luna-tools-8i_vws0f/result.json`。相关 Rust 已格式化，`git diff --check` 通过。声明检查正例中遗漏 `where` 的 fixture 已修正后重跑通过。

## 1. 总体结论与量化概览

Flurry 编译器（`luna`）在已支持的语言子集上，已有从源码解析、模块解析、HIR、elaboration、comptime invocation 到 NIR/IIR、资源检查及参考解释执行的通路。测试证明的是具体输入与契约，不能据此宣布前端、中端或语言整体完成。

目前仍有基础实现缺口：普通 `for` 的迭代协议尚未展开为可执行 NIR；CMTT 的 contextual 参数、值子句的完整推断、方法生成器、quoted pattern/arm、代码容器与 Any/any 证据仍未接通；部分函数值的参数协议、开放 effect 行与 runtime 依赖 effect 请求也有明确拒绝入口。这些缺口之外，原生后端、标准库迁入及 anar 对接仍需完成。不能把剩余工作概括为仅后端和工具链。

Normal-form 身份、Tuple 头、PTC 与程序验证分层、quote fn 阶段及 IR dump 等要求继续作为实现约束。已执行的检查列在下面；本报告不声称所有语义边界已经逐项验收。

### 2026-10-19：有限列表协议入口

- 增加 `Protocols.list_pattern`、包限定 source selector 和 sysroot `list-pattern-protocol` 配置。检查关联 Element、引用接收者、长度／编译期索引的同一无符号类型；模式经既有 Then 节点降码，保留 S-expression dump。
- 已验证长度过滤、逐项短路、后续字段 guard、let-else、索引特化及失败候选后的单次主体清理。普通 guard 也支持由内存摘要证明只读的显式指针调用；地址表达式自身仍接受观察检查。
- 仍缺 comptime trait 相、endless/rest 和拥有资源的提取结果。列表 or-pattern 内部主体绑定随后改为非导出的 Observe 作用域，已接通候选合并、嵌套、whole binding 与展开后的资源检查。原生 usize 字面量等待目标宽度；当前测试通过显式固定宽度 size 别名运行，没有假定宿主指针宽度。
- 工作目录 `/home/fin/luna`，dev，未提交。`development/check.py --timeout 900 -- test -p luna --test list_patterns`：9 项通过（`/tmp/luna-tools-vxjzk1h9/result.json`）。`--test execution --test drop --test cmtt guard`：107 项通过（`/tmp/luna-tools-uc146fio/result.json`）。`--lib list_pattern`：配置优先级检查通过（`/tmp/luna-tools-wpl0cgrc/result.json`）。所有命令均通过 `python3 -B` 调用。

### 2026-10-19：列表候选的内部观察作用域

- 新增 NIR `Observe` 模式及 S-expression dump。主体别名只在内部模式计算中有效，不进入成功导出、or-pattern 绑定统一或所有权转移。同步自由变量、遍历、展开重命名、解释执行、程序流、内存与资源分析；内部主体地址逃逸被拒绝。
- 列表 or-pattern、嵌套 tuple、whole binding、let-else 和真正的 optional inline／quote 展开已通过资源检查。Drop 用例同时检查未清理与重复清理，覆盖前一候选失败及所有列表候选失败后的回退。
- 修复 optional inline 归一化提前展开 trait 方法关联结果签名的问题：保留接口引用的原契约，在外层调用结果处归一化。
- 工作目录 `/home/fin/luna`，dev，未提交。`python3 -B development/check.py --timeout 900 -- test -p luna --test execution --test drop --test cmtt --test list_patterns --test quote_functions --test instantiation`：2,519 项通过，退出 0，报告 `/tmp/luna-tools-q78kxro9/result.json`。`python3 -B development/check.py --timeout 900 -- test -p nir -p iir`：259 项通过，退出 0，报告 `/tmp/luna-tools-3iopxvtq/result.json`。改动已格式化，相关 `git diff --check` 通过。

### 2026-10-19：泛型列表调用与分派复用

- 贯通 `T :- ListView`、关联 `Element` 返回值和两层泛型转发的验收；同一程序分别实例化 bool 与 u64 元素，覆盖长度匹配与 fallback，并经过资源检查执行。
- 纯度分析补齐 Observe 内部绑定作用域和提取器计算检查。已有独立 P4 依据的提取器可用于纯函数；忽略元素值不能隐藏写入。跨函数只读指针来源的 P4 推导仍有保守限制，不宣称任意只读列表实现均可用于 pure。
- 资源摘要中的具体方法引用改用已选实现的实例化签名，保留分派前接口契约校验；同时兼容 quote 模板中的方法主体。新增包内成功分派缓存，键包含声明签名、接口与方法实参、subject；缓存命中仍计入预算并更新诊断位置，失败结果不缓存。纯度与资源分析不再为每个元素重复求解相同分派。
- 工作目录 `/home/fin/luna`，dev，未提交。`python3 -B development/check.py --timeout 900 -- test -p luna --test list_patterns --test execution --test drop --test inline_control --test instantiation`：2,396 项通过，退出 0，报告 `/tmp/luna-tools-3l7u2bw7/result.json`。其中列表 15、执行 1,335、资源清理 764、inline 控制流 231、实例化 51。修改的 Rust 文件已格式化，`git diff --check` 通过。

### 2026-10-19：CMTT 生成器值子句

- 显式 comptime 代码生成器增加值子句：区分 Type 与 ComptimeValue 绑定，实例化值域、输入／输出接口及生成数据环境；Display 输出每个子句的域和值。复用已完成数据的冻结与重建，未引入 runtime 上下文捕获权限。
- 已验证位置值实参、具名默认项、混合 Type／值参数、`T, value:T` 依赖值域、聚合值、嵌套调用、默认函数计算、程序契约及显式覆盖。Type 推断可与已完成的值默认项组合。递归调用默认项隔离当前应用的绑定，避免读取外层同一声明的旧参数。
- 修正值子句字段投影被误认成 Type 头的问题。值子句可提升为 quoted 主体常量；runtime 参数越阶段供应、错误值域、自引用默认值和未调用生成器中的非法分支均拒绝。程序契约继续由程序计算／验证路径处理，不向 PTC 添加值状态事实。
- 仍缺值子句的一般反向推断与依赖未完成 Type 推断的默认计算调度、quote fn 值泛型子句和方法生成器；这些不属于本次通过的能力。完整 CMTT 仍未完成。
- 工作目录 `/home/fin/luna`，dev，未提交。`python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test execution --test quote_functions --test normal_patterns --test instantiation`：1,762 项通过，退出 0，报告 `/tmp/luna-tools-8oy4surj/result.json`。其中 CMTT 279、执行 1,335、quote 函数 87、normal-form 模式 10、实例化 51。已格式化修改的 Rust 文件，`git diff --check` 通过。

### 2026-10-19：生成器默认项等待 Type 推断

- 生成器子句计划区分立即可用输入与等待推断的默认项。探查仅检查表达式类型、绑定与静态结构，不调用生产函数；取得 Type 后按声明顺序执行延迟默认项，并复用已完成值。后续 Type 默认项可以依赖值默认项，计算所得 Type 必须与实参推断一致，不能被实参覆盖。
- 已通过普通实参及目标代码接口推断、comptime trait 方法默认数据、Type／值交替默认链、嵌套生成调用、契约拒绝和结构性 Type 默认项的数字字面量上下文回归。尚未完成由默认项决定实参接口时的一般循环依赖恢复，也不提供任意值子句反向推断。
- 修复子句默认表达式的阶段检查遗漏：未调用或显式覆盖的生成器默认项也检查函数调用阶段，不能隐藏 runtime 调用。定义检查保留抽象子句输入，实际构造完成默认项与契约后才交付代码。
- 工作目录 `/home/fin/luna`，dev，未提交。`python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test quote_functions --test instantiation --test normal_patterns`：433 项通过，退出 0，报告 `/tmp/luna-tools-ijb1c3wv/result.json`。随后新增默认 Type 不被实参覆盖的回归，`--test cmtt generator_deferred_type_defaults_cannot -- --nocapture`：1 项通过，退出 0，报告 `/tmp/luna-tools-e43m1_lm/result.json`；合计 434 个不同测试通过。修改的 Rust 文件已格式化，`git diff --check` 通过。

### 2026-10-19：默认计算提前提供生成器输入上下文

- 生成器调用在取得新 Type 证据后推进就绪的默认链，随后刷新输入接口。默认值按调用缓存，重建推断候选时重放约束；已完成的值索引代入签名，尚未完成的值索引暂缓参与相关输入匹配。
- 覆盖普通数字实参、字面量展开包、位于 Type 证据之前的 quoted code 参数，以及 u8／u16 越界拒绝。默认 Type 不可被实参推断覆盖；定义检查仍不执行默认生产函数。一般循环依赖恢复与任意值反向推断仍未完成。
- 工作目录 `/home/fin/luna`，dev，未提交。`python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test quote_functions --test instantiation --test normal_patterns`：437 项通过（289 + 87 + 51 + 10），退出 0，报告 `/tmp/luna-tools-mbi0x430/result.json`。扩充展开包用例后 `--test cmtt generator_ready_defaults -- --nocapture`：3 项通过，退出 0，报告 `/tmp/luna-tools-bvcf1vfu/result.json`。已格式化修改的 Rust 文件。

### 2026-10-19：直接变参集合向生成器回调传递类型证据

- 修复直接收集变参跳过类型预检的问题：完整集合的固有元素类型可确定生成器的 T，为源码位置更早的 tactic lambda 提供参数列表。复用已有的有界类型证据读取，并从展开包的字段表读取已知类型，不提前执行生产函数。
- 新增六项行为回归：位置／展开／具名整包、空包与属性记录、状态计数验证源序和包只求值一次、未调用的发散生产函数与 runtime 阶段拒绝、数值目标上下文、注册 Tuple normal-form 身份。
- 工作目录 `/home/fin/luna`，dev，未提交。`python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test quote_functions --test instantiation --test normal_patterns`：443 项通过（295 + 87 + 51 + 10），退出 0，报告 `/tmp/luna-tools-fxlxze8p/result.json`。完整的部分集合证据调度及任意表达式之间的双向推断仍未完成。

### 2026-10-19：生成器集合的部分类型证据

- 将集合的结构与已知字段作为 InputContext 导入调用候选，未知字段保留候选内部变量；不再要求整个集合类型先完成。目标结构向内传递，数字字面量不在预检阶段默认成整数；Tuple 的 normal-form 头通过 NormalTuple 上下文保留。
- 重新建立候选时重放部分输入约束，再推进默认链；尚未完成的值索引仍延后参与匹配。覆盖 `(T,u8)`、具名整包、嵌套元组、注册 Tuple、部分证据解锁默认 Type 并向前面的代码字面量提供上下文，以及形状／范围／回调错误拒绝。
- 工作目录 `/home/fin/luna`，dev，未提交。`python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test quote_functions --test instantiation --test normal_patterns`：447 项通过（299 + 87 + 51 + 10），退出 0，报告 `/tmp/luna-tools-2glvhze3/result.json`。已格式化修改的 Rust 文件。任意循环依赖与需要部分 callable 上下文的生成器输入仍待完善。

### 2026-10-19：生成器部分函数上下文与回调集合

- 将调用候选的 InputContext 保留到生成器数据检查与私有状态帧，允许先确定回调输入，再由正文推断输出。部分上下文支持普通参数、变参元素及嵌套元组；已经完成的接口仍使用原目标类型检查。
- 结构元组／匿名记录的生成器数据重建递归处理字段，允许无捕获 comptime 回调，并保留原类型、字段次序、预算与深度限制。带捕获环境的闭包、指针及活动存储不因容器包装获得冻结权限；名义包装内的通用回调冻结未开放。
- 新增六项测试覆盖 tactic lambda 输出推断、位置与具名回调包、嵌套回调集合、生成器状态及源序、未调用正文阶段检查、捕获闭包拒绝。
- 工作目录 `/home/fin/luna`，dev，未提交。`python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test execution --test quote_functions --test instantiation --test normal_patterns`：1,788 项通过（305 + 1,335 + 87 + 51 + 10），退出 0，报告 `/tmp/luna-tools-qw3gg1e2/result.json`。修改的 Rust 文件已格式化。任意循环依赖与一般捕获数据图仍未完成。

### 2026-10-19：生成器收集过程逐字段推进推断

- 每个变参元素或属性字段检查完成后，将其类型写回部分输入上下文并重新推进候选／默认链；后续元素和交错供应的代码参数立即取得新上下文。字段值仍只按源码顺序准备一次，非泛型调用不增加候选推断开销。
- 新增四项回归覆盖回调输出指导后续窄数字字面量、字段之间推进 Type 默认计算与越界拒绝、交错 implicit 代码参数、状态计数及注册 Tuple 身份。
- 工作目录 `/home/fin/luna`，dev，未提交。`python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test quote_functions --test instantiation --test normal_patterns`：457 项通过（309 + 87 + 51 + 10），退出 0，报告 `/tmp/luna-tools-gtjdom5x/result.json`。已格式化修改的 Rust 文件；一般循环依赖调度仍未完成。

### 2026-10-19：具名元组字面量复用逐字段生成器推断

- 已确认变参槽中的具名元组字面量在路由后转换为字段准备序列，复用直接收集的候选更新、默认链和上下文检查，不增加元组嵌套。转换按字段数量计费，仅处理无外层计算的字面量；生产函数和带语句的块保留整体求值。
- 新增四项回归覆盖字段间类型传播、默认计算和越界拒绝、状态更新次序与注册 Tuple 身份、计算包及块只执行一次。
- 工作目录 `/home/fin/luna`，dev，未提交。`python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test quote_functions --test instantiation --test normal_patterns` 首次 CMTT 312 通过、1 项新增 fixture 因算术表达式缺少分隔空格解析失败，报告 `/tmp/luna-tools-nd_n6odh/result.json`。修正 fixture 后 `--test cmtt generator_named_literal_packets_keep -- --nocapture` 1 项通过，退出 0，报告 `/tmp/luna-tools-v5bg0aoh/result.json`；随后 `--test quote_functions --test instantiation --test normal_patterns` 148 项通过，退出 0，报告 `/tmp/luna-tools-jf6wlwkn/result.json`。合计 461 个不同测试通过。一般计算包内部的循环依赖调度仍未完成。

### 2026-10-19：名义生成器数据中的无捕获回调

- 名义数据重建复用常量路径的声明身份、枚举 variant、泛型字段实例化与 newtype 表示布局检查；其字段递归进入生成器数据边界。取消先对整个名义值进行普通 normal-term 冻结导致回调被过早拒绝的限制，不改变普通常量重建行为。
- 新增四项测试覆盖命名／匿名无捕获回调、结构体嵌套枚举、泛型字段布局、newtype 回调表示，以及捕获闭包、指针、runtime 函数和 quoted runtime 接口拒绝。活动捕获图仍未开放。
- 工作目录 `/home/fin/luna`，dev，未提交。`python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test execution --test quote_functions --test instantiation --test normal_patterns`：1,800 项通过（317 + 1,335 + 87 + 51 + 10），退出 0，报告 `/tmp/luna-tools-grfpxqih/result.json`。追加 quoted runtime 接口拒绝断言后 `--test cmtt generator_nominal_callback_data_keeps -- --nocapture`：1 项通过，退出 0，报告 `/tmp/luna-tools-7mv_ml90/result.json`。已格式化修改的 Rust 文件。

### 2026-10-19：生成器只读捕获回调

- 生成器数据域接受带显式环境的 comptime Closure，递归检查环境域。重建含捕获的回调前复用已有闭包快照验证，检查嵌套捕获、循环、写入和捕获存储取址，再按已声明环境类型重建；不把有环境闭包擦除成无捕获函数类型。
- 新增四项测试覆盖重复调用与跨生成器转发、嵌套只读回调和名义包装、原变量变更后仍保持创建时快照、修改捕获状态和暴露捕获地址的拒绝。
- 工作目录 `/home/fin/luna`，dev，未提交。`python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test execution --test quote_functions --test instantiation --test normal_patterns`：1,804 项通过（321 + 1,335 + 87 + 51 + 10），退出 0，报告 `/tmp/luna-tools-nu6_wkj0/result.json`。修改的 Rust 文件已格式化。一般名义捕获对象及可变共享捕获图仍未完成。

### 2026-10-19：只读闭包的名义捕获图

- 扩展共享闭包快照验证到结构体、枚举与 newtype：有界复制并检查名义类型实参的作用域，递归验证实际字段和表示值，继续使用布局重建保留类型／variant 身份。嵌套回调沿用共享节点检查及循环、写入、取址拒绝。
- 新增四项回归覆盖枚举—结构体—newtype 嵌套捕获、泛型名义字段的不同实例、隐藏于结构体中的可变回调、指针字段与捕获对象字段取址拒绝。可变共享环境仍未完成。
- 工作目录 `/home/fin/luna`，dev，未提交。`python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test execution --test quote_functions --test instantiation --test normal_patterns`：1,808 项通过（325 + 1,335 + 87 + 51 + 10），退出 0，报告 `/tmp/luna-tools-eyfvwei5/result.json`。已格式化修改的 Rust 文件。

### 2026-10-19：命名函数值向生成器提供类型证据

- 生成器的有界类型预检现在读取函数值的完整签名，并根据声明恢复 comptime 阶段。显式 Type 实例、静态 Type 默认项复用已有签名代入路径；normal-form 头不作为普通函数值展开，动态默认计算不在预检中执行。
- 新增五项回归覆盖前置代码字面量的窄类型上下文、显式及默认 Type 实例、源码求值顺序、发散函数体与被覆盖默认计算不执行、runtime 阶段和类型约束拒绝。
- 工作目录 `/home/fin/luna`，dev，未提交。`python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test quote_functions --test instantiation --test normal_patterns`：478 项通过（330 + 87 + 51 + 10），退出 0，报告 `/tmp/luna-tools-0zclvymg/result.json`。已格式化修改的 Rust 文件。
- 关联生成器仍未实现：`program/cmtt/generators.rs::ensure_code_generator` 显式拒绝 method；`program/interfaces.rs` 的方法契约和选择结果使用普通 Term 签名与 NIR 方法值。后续需要保留 contextual code 的接口／绑定证据并接入方法选择，不能仅删除拒绝检查或把 quoted code 擦除为普通函数返回值。

### 2026-10-19：生成器内建字段类型预检

- 有界类型预检支持内建拥有型字段投影：匿名记录按字段名取得类型，结构体按已知名义实参实例化字段布局；结果可向前置代码参数提供上下文，也可供展开包取得宽度和元素类型。该路径不求值对象、不选择方法或 getter，不读取指针目标。
- 新增四项回归覆盖返回配置对象的回调字段、泛型结构体与匿名记录、字段包展开的源码顺序和单次生产、未调用的发散生产函数、runtime 阶段与实际前置条件拒绝。
- 工作目录 `/home/fin/luna`，dev，未提交。`python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test quote_functions --test instantiation --test normal_patterns`：482 项通过（334 + 87 + 51 + 10），退出 0，报告 `/tmp/luna-tools-bvmulht7/result.json`。修改的 Rust 文件已格式化；方法生成器及一般循环依赖调度仍未完成。

### 2026-10-19：计算所得回调的返回类型证据

- 类型预检从计算所得 callee 的已知函数／闭包类型继续取得独立于本次普通实参值的返回类型，支持配置字段回调、工厂返回闭包、共同函数类型的条件分支，以及回调返回参数包的展开。递归沿用深度和工作预算，不执行 callee 或选择分支。
- 新增五项回归覆盖前置 tactic lambda、只读闭包工厂与包工厂、条件选择的源码顺序、实参类型／数量／前置条件与 runtime 阶段拒绝、未调用的发散工厂和回调。
- 工作目录 `/home/fin/luna`，dev，未提交。`python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test quote_functions --test instantiation --test normal_patterns`：487 项通过（339 + 87 + 51 + 10），退出 0，报告 `/tmp/luna-tools-hryzwhmw/result.json`。已格式化修改的 Rust 文件。依赖本次普通实参值的返回类型仍不作为无求值预检依据。

### 2026-10-19：生成器块内绑定类型证据

- 有界类型预检顺序追踪块内简单按值绑定和内建元组解构，从静态注解或已有初始化类型证据取得绑定类型，供块尾表达式和后续局部投影使用。临时类型环境按绑定身份保存，成功与失败均恢复；不执行初始化、赋值或其它块内语句，不选择数值默认类型。
- 新增四项回归覆盖函数值局部别名、静态窄数值注解、嵌套遮蔽、元组生产者与解构的源码顺序、无效／越界初始化、未调用发散生产函数及 runtime 阶段拒绝。
- 工作目录 `/home/fin/luna`，dev，未提交。`python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test quote_functions --test instantiation --test normal_patterns`：491 项通过（343 + 87 + 51 + 10），退出 0，报告 `/tmp/luna-tools-7zg13tgt/result.json`。修改的 Rust 文件已格式化；复杂模式与一般循环依赖的无求值推断仍待完善。

### 2026-10-19：块内记录解构的生成器类型证据

- 块内类型预检支持内建结构体／匿名记录字段解构，复用字段投影的有界布局类型读取，保留泛型实参、字段重命名和元组嵌套。未知字段不产生证据，实际模式检查继续负责拒绝无效字段及重复字段；不执行模式协议或 guard。
- 新增三项回归覆盖泛型结构体与匿名记录、嵌套元组字段解构及生产者源码顺序、未调用的发散生产函数、runtime 阶段和无效字段拒绝。
- 工作目录 `/home/fin/luna`，dev，未提交。`python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test quote_functions --test instantiation --test normal_patterns`：494 项通过（346 + 87 + 51 + 10），退出 0，报告 `/tmp/luna-tools-12_v4e9h/result.json`。修改的 Rust 文件已格式化，`git diff --check` 通过；协议模式及一般循环依赖调度仍未完成。

### 2026-10-19：match 模式绑定的生成器类型证据

- 类型预检从已知 subject 类型为分支内的简单绑定、元组和记录解构建立临时类型环境；各分支结束或失败均恢复环境，只有共同结果类型才作为外部证据。subject、guard 和模式计算均不在预检中执行，覆盖性及模式合法性由实际检查负责。
- 新增四项回归覆盖元组分支回调、泛型记录模式、subject／guard 与前置参数的源码顺序、不完整覆盖／错误模式／不同分支类型拒绝及未调用的发散 subject。
- 工作目录 `/home/fin/luna`，dev，未提交。相关四组检查中 CMTT 349 项通过、1 项新增 fixture 因保留关键字作为形参解析失败，报告 `/tmp/luna-tools-ivin56t7/result.json`。修正 fixture 后 `python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt generator_match_binding_evidence_keeps -- --nocapture`：1 项通过，退出 0，报告 `/tmp/luna-tools-mecbbgbd/result.json`；随后 `--test quote_functions --test instantiation --test normal_patterns`：148 项通过，退出 0，报告 `/tmp/luna-tools-nb63y5hs/result.json`。合计 498 个不同测试通过。复杂协议模式和一般循环依赖仍待完善。

### 1.1 代码库规模统计

以下规模为原评估时的快照，未在本次集成检查中重新统计。

| 统计维度 | 数值 | 说明 |
|---|---|---|
| **Workspace 启用 Crate 数** | 21 个 | 见根 [Cargo.toml](file:///home/fin/luna/Cargo.toml#L1-L28) |
| **暂未启用旧 Crate 数** | 4 个 | `codegen`、`mir`、`mir_build`、`typeck`（旧 `ty` 架构遗留，尚未迁移至 NIR/IIR） |
| **编译器源码规模 (`src/`)** | 255 个 `.rs` 文件 / **183,798 行** | 核心集中在 `elaboration`（120,440 行）、`comptime_interpretor`（13,669 行）、`nir`（12,562 行）、`iir`（8,091 行） |
| **编译器测试规模 (`tests/`)** | 120 个 `.rs` 文件 / **180,229 行** | 含 **5,685 个 `#[test]`** 函数；端到端语义回归集中在 `compiler/luna/tests/`（39 个文件，146,978 行） |
| **标准库源码 (`library/`)** | 11 个 `.fl` 文件 + 1 个配置 / **622 行** | 包含 `builtin`、`meta`、`std` 基础定义与 [language-items.conf](file:///home/fin/luna/library/language-items.conf) |
| **设计规范与开发记录** | `docs/` 70+ 篇正式规范 | [development/README.md](file:///home/fin/luna/development/README.md) 累计记录 1,090+ 个增量验证节点 |

### 1.2 验证锚点

2026-10-19，guard 拥有型观察与临时清理的阶段性集成检查：

- 工作目录：`/home/fin/luna`，dev 工作区，未提交。
- 命令：`python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test newtypes`。
- 结果：drop 760、newtypes 207，共 967 项通过，退出 0。报告 `/tmp/luna-tools-vscg0ffx/result.json`，结果锚点为同目录 `stdout.log` 的两条 `test result`。
- 覆盖本阶段新增的直接 guard 聚合实参、临时工厂字段投影、newtype 解包、枚举名称观察，以及内联前后的次数、候选回退和清理顺序。复合条件的临时 owner 保留到完整条件结束；短路不执行另一侧。
- 仍拒绝消费外部非 Copy 所有者、可能拒绝的观察调用／析构及未取得来源依据的输入。这不是一般候选事务完成的证明，也不是全 workspace 验收。


物化、晚期 inline、条件 callee 与资源清理衔接后的阶段性集成检查：

- 工作目录：`/home/fin/luna`，dev 工作区，未提交。
- 首次命令：`python3 -B development/check.py --timeout 900 -- test -p luna --test execution --test drop --test quote_functions --test instantiation --test cmtt`。cmtt 261、drop 752、execution 1328 通过；实例化组两项旧断言要求成功展开后仍保留独立函数条目，命令退出 101。报告 `/tmp/luna-tools-owxi7uv_/result.json`。
- 将验收拆为成功展开与残余实例两类：成功展开检查调用条目消除和实际执行结果；递归回退继续检查 inline 标记、泛型及 deferred 类型完成。未通过保留无用实例来满足旧断言。
- 后续命令：`python3 -B development/check.py --timeout 900 -- test -p luna --test instantiation --test quote_functions`，51 + 85 项通过，退出 0。报告 `/tmp/luna-tools-lc02kuv6/result.json`。
- 五组累计 2477 项通过；这是上述语义路径的阶段性检查，不是全 workspace 或语言完成度验收。


近期 CMTT 状态帧、类型信息与普通 comptime 生成器改动后的集成检查：

- 工作目录：`/home/fin/luna`，dev 工作区，未提交。
- 命令：`python3 -B development/check.py --timeout 900 -- test -p luna --test execution --test resources --test runtime_purity`。
- 结果：execution 1328、resources 346、runtime_purity 52，共 1726 项通过，0 失败；退出码 0，耗时 110.343 秒（含构建）。报告 `/tmp/luna-tools-xceswsfi/result.json`，各组结果见同目录 `stdout.log`。
- 配合专项检查 `/tmp/luna-tools-xni4aq1x/result.json` 的 300 项通过，只证明这些测试覆盖的行为；不是本次全 workspace 验收，也不证明尚未接通的语法和协议已实现。

以下为原评估保留的历史检查记录：

1. **全 Workspace 编译与目标检查（原评估实测）**：
   - 命令：`python3 -B development/check.py --timeout 300 -- check --workspace --all-targets`
   - 工作目录：`/home/fin/luna`
   - 结果：`exit_code: 0`，`wrapper_exit_code: 0`，耗时约 20 秒，报告锚点 `/tmp/luna-tools-ae4581s6/result.json`。
2. **近期全量测试与核心回归集锚点**：
   - 全 Workspace 测试（`python3 -B development/check.py --timeout 900 -- test --workspace`）：162 组测试共 **5,469 passed, 0 failed, 11 ignored**（耗时 150.5 秒，报告 `/tmp/luna-tools-dstnc3bj/result.json`）。
   - 后续增量演进在此基础上新增约 200+ 项专项测试（如 `open_functions`、`execution`、`drop`、`inline_control`、`runtime_purity`），最近一次联合回归（`open_functions` + `type_spreads` + `execution` + `drop`）2,470 项通过（`/tmp/luna-tools-rfx5l1ff/result.json`）。

---

## 2. Crate 级架构与完成度矩阵

| Crate 路径 | 源码行数 | 测试行数 | 状态 | 当前职责与真实能力边界 |
|---|---:|---:|---|---|
| [compiler/symbol](file:///home/fin/luna/compiler/symbol/src/lib.rs) | 192 | 0 | **稳定** | 符号驻留（interning）与标识符基础表示 |
| [compiler/vfs](file:///home/fin/luna/compiler/vfs/src/lib.rs) | 234 | 0 | **稳定** | 虚拟文件系统、文件 ID、AST 挂载与源码目录记录 |
| [compiler/diagnostic](file:///home/fin/luna/compiler/diagnostic/src/lib.rs) | 699 | 0 | **可用** | 基于 `ariadne` 与 `rustc_span` 的诊断收集与格式化输出 |
| [compiler/lex](file:///home/fin/luna/compiler/lex/src/lib.rs) | 1,779 | 483 | **成熟** | 词法分析、运算符空白敏感规则、精确字面量拼写保留、注释与分隔符 |
| [compiler/ast](file:///home/fin/luna/compiler/ast/src/lib.rs) | 2,677 | 0 | **成熟** | 完整语法树定义与 Lisp S-expression 导出（`--emit ast`） |
| [compiler/parse](file:///home/fin/luna/compiler/parse/src/lib.rs) | 6,571 | 1,544 | **成熟** | 90 级绑定力表达式解析、右结合函数箭头、模式优先级、multi 属性链、声明/语句解析 |
| [compiler/resolve](file:///home/fin/luna/compiler/resolve/src/lib.rs) | 3,252 | 385 | **成熟（单包/sysroot）** | 模块树发现、统一命名空间、具名/通配导入、再导出、别名、锚点、入口与声明独立权限检查 |
| [compiler/hir](file:///home/fin/luna/compiler/hir/src/lib.rs) | 2,992 | 1,550 | **成熟** | 高层 IR、専用 take-view 节点、模式候选作用域、确定性 S-expression `Display`（`--emit hir`） |
| [compiler/ast_lowering](file:///home/fin/luna/compiler/ast_lowering/src/lib.rs) | 5,972 | 0 | **成熟** | AST → HIR 转换、标签身份绑定、`when` 单语句块作用域、局部顺序模式作用域 |
| [compiler/query](file:///home/fin/luna/compiler/query/src/lib.rs) | 572 | 156 | **基础就绪** | 查询缓存、环检测、panic 恢复与键隔离 |
| [compiler/middle](file:///home/fin/luna/compiler/middle/src/lib.rs) | 1,225 | 0 | **过渡/桥接** | Salsa 查询数据库输入包装与 `TyCtxt` 桥接（旧类型表示不作为新语义权威） |
| [compiler/intrinsic](file:///home/fin/luna/compiler/intrinsic/src/lib.rs) | 675 | 347 | **部分完成** | 语言项（[`LangItem`](file:///home/fin/luna/compiler/intrinsic/src/lang_item.rs)）表、内建原语登记与 sysroot 包结构描述；`builtins` 模块仍注释待接 |
| [compiler/interface](file:///home/fin/luna/compiler/interface/src/lib.rs) | 309 | 0 | **可用** | [`Session`](file:///home/fin/luna/compiler/interface/src/lib.rs) 与 [`CompilerInstance`](file:///home/fin/luna/compiler/interface/src/lib.rs) 生命周期管理 |
| [compiler/nir](file:///home/fin/luna/compiler/nir/src/lib.rs) | 12,562 | 8,668 | **核心成熟** | 不可变类型 Core + 作用域 Substitution、Opaque Normal-Form Head、带类型程序树、阶段检查、Newtype/Enum 编码与 S-expression `Display`（`--emit nir`） |
| [compiler/elaboration](file:///home/fin/luna/compiler/elaboration/src/lib.rs) | 120,440 | 15,268 | **深度实现** | 双向类型检查、候选区域事务、PTC 求解器、Trait 选择与 Coherence、Comptime 调度、Open 函数、CMTT、代数效应、程序验证（范围流、移动/所有权、Drop 计划、内存效应、纯度 P1–P6） |
| [compiler/comptime_runtime](file:///home/fin/luna/compiler/comptime_runtime/src/lib.rs) | 1,287 | 1,389 | **成熟** | 规范任意精度 `Integer`、精确十进制 `Float`、定宽有界整数与任意宽度 `BitVec` 值内核 |
| [compiler/comptime_interpretor](file:///home/fin/luna/compiler/comptime_interpretor/src/lib.rs) | 13,669 | 3,230 | **成熟** | NIR 与 IIR 双入口参考解释器、共享预算/深度控制、活动闭包图、内存/指针/效应/Drop 清理执行 |
| [compiler/comptime_executor](file:///home/fin/luna/compiler/comptime_executor/src/lib.rs) | 258 | 429 | **可用** | 基于驱动端语义 `Eq`/`Hash` 与版本作用域的有界纯查询缓存（[`PureQueryCache`](file:///home/fin/luna/compiler/comptime_executor/src/lib.rs)） |
| [compiler/comptime_compiler](file:///home/fin/luna/compiler/comptime_compiler/src/lib.rs) | 14 | 0 | **占位** | 仅含 `cargo new` 生成的 `add` 占位函数，尚无编译期 JIT/字节码编译实现 |
| [compiler/iir](file:///home/fin/luna/compiler/iir/src/lib.rs) | 8,091 | 1,790 | **核心就绪** | 非 SSA 高层具体实例图（[`iir::Package`](file:///home/fin/luna/compiler/iir/src/lib.rs#L91-L103)）、泛化/阶段消解（Staging）、内联展开、Drop 清理挂载与类型完成性校验 |
| [compiler/luna](file:///home/fin/luna/compiler/luna/src/main.rs) | 5,893 | 146,978 | **核心成熟** | CLI 驱动器（`--emit`、`--invoke-comptime`、`--instantiate`、`--interpret`、`--analyze-resources`）、多包/sysroot 装配、协议绑定、记忆化调用与端到端回归测试集 |
| `codegen` / `mir` / `mir_build` / `typeck` | 3,030 | 0 | **已停用（旧版）** | 早期基于 `ty` 与 MIR 的旧管线，已在 `Cargo.toml` 中注释停用；尚未重写对接至 `iir::Package` |

---

## 3. 各核心子系统与语言维度进度详评

### 3.1 词法、语法、AST 与 HIR 降码
- **已完成能力**：
  - **精确字面量前端**：整数与浮点在词法/HIR 阶段保留原始源码拼写（`(integer (spelling "..."))`、`(float (spelling "..."))`），避免经过宿主 `i64`/`f64` 造成精度截断或双重舍入；支持 `LiteralInto` 后缀转换节点。
  - **语法覆盖**：完整的 90 级运算符优先级、右结合 `->` 函数类型、前缀/后缀投影（`.ref`、`.*`）、每个 take-view 操作独立对应 HIR 节点、multi 属性前缀链、`if`/`when`/`match`/`for`/`while`、带标签 `break`/`continue`、`let ... else`、顺序模式（`and`/`is`）与 `or` 模式、位向量分段模式、`%{...}` 显式闭包环境、类型列表展开 `...`。
  - **结构化导出**：AST 与 HIR 均实现确定性 Lisp S-expression 导出，通过 Emacs 批处理单表达式读取与缩进一致性验证。
- **未完成／受限项**：
  - 字符串插值降码目前明确报错 `unsupported`；
  - 模板宏（Template Macro）与构造宏（Construction Macro）的展开管线尚未实现；
  - `for` 循环已保留完整 HIR 节点，但因迭代器协议（基于 `inline fn` + 代数效应的 `Iter`/`IntoIterator` 契约）尚未在库与编译器间最终定型，Elaboration 阶段暂明确拒绝源码 `for`（已接通 `inline for` 范围迭代与 `tuple_enumerate`）。

### 3.2 模块、包组织与构建接口
- **已完成能力**：
  - 支持单文件模式与以 `main.fl`/`lib.fl` 为入口的源码根自动扫描（含文件模块、`mod.fl`、无入口目录模块与内联模块）。
  - 具名导入、通配导入（`*`）、分组导入、重命名（`as`）、`pub use` / `private use` 再导出、`@` 与前导 `.` 路径锚点均已贯通。
  - 声明自身可见性与导入入口可见性分离存储和校验；支持带种子的导入环收敛与歧义诊断。
  - 支持 `--sysroot` 加载多包（`builtin`、`std`、`meta` 等）并读取 [library/language-items.conf](file:///home/fin/luna/library/language-items.conf) 绑定核心语言项。
- **未完成／受限项**：
  - 外部构建工具 `anar` 仓库尚未在当前环境中定位与接入；
  - 跨 package 的增量文件发现失效（invalidation）与符号链接源码目录支持尚未开放。

### 3.3 类型系统、NIR 表示与 PTC（命题类型约束）
- **已完成能力**：
  - **NIR 核心表示**（[`compiler/nir/src/types.rs`](file:///home/fin/luna/compiler/nir/src/types.rs)）：不可变类型 Core 与引用端作用域 Substitution 分离；严格区分刚性类型参数、运行时依赖绑定（`RuntimeId`）、推断变量；Normal-form 头（如 `Identity<T>`、`Tuple<...Es>`）保持不透明，绝不自动展开函数体。
  - **PTC 求解器**（[`compiler/elaboration/src/ptc.rs`](file:///home/fin/luna/compiler/elaboration/src/ptc.rs)）：
    - 支持布尔组合、类型等式/不等式、结构同余与分解、显式注册的 Normal-form 头同余；
    - 支持数学整数差分逻辑（[`ptc/integer.rs`](file:///home/fin/luna/compiler/elaboration/src/ptc/integer.rs)，Bellman–Ford 负环检测与具体反例生成）；
    - 支持类型分类查询（[`ptc/classification.rs`](file:///home/fin/luna/compiler/elaboration/src/ptc/classification.rs)，涵盖 bool/integer/signed/unsigned/float/tuple/pointer/function）；
    - 支持计算效应行（Effect Row）的交换同余、重数区间与分支局部不相交反驳（[`elaboration/src/effects.rs`](file:///home/fin/luna/compiler/elaboration/src/effects.rs)）；
    - 所有证明树与反例支持独立作用域回放（Proof Replay）与 S-expression 导出。
  - **用户定义类型与别名**：泛型/非泛型 `struct`、`enum`（含 unit/位置/命名变体、变体分组、显式判别编码与配置）、`newtype`（含表示权限与递归冻结）、透明泛型 `typealias`（含残余类型构造器）均已打通。
- **未完成／受限项**：
  - PTC 的 19 类原子命题与 11 个求解模块中，一般线性/同余整数算术（超越差分逻辑与 `i128` 容量边界）、符号位宽 BitVec 理论、宇宙层级与槽观察尚未完全接通；
  - PTC 当前采用有界分支搜索，尚未演进为带子句学习的完整 CDCL(T) 引擎。

### 3.4 Trait 系统、实现选择与两相协议
- **已完成能力**：
  - Trait 声明（含泛型参数、关联类型、关联常量、默认方法、`quote fn` 方法、父 Trait 约束与 `Self` 依赖类型要求）。
  - **三态候选求解與一致性（Coherence）**（[`compiler/elaboration/src/implementation.rs`](file:///home/fin/luna/compiler/elaboration/src/implementation.rs)）：基于候选局部推断区域、证明/反驳要求与同精度重命名冲突检查。
  - **两相 Trait 与分派物化**：支持 runtime 与 comptime 方法分派、inherent 方法索引、具体实例的静态分派表生成（`DispatchInstance`）以及语言项协议（算术、`Index`、`LiteralInto`、`Copy`、`Drop`、`Splice`、`Eq`、`Hash`）显式绑定。
- **未完成／受限项**：
  - 负向 Trait 谓词（negative predicates）、自动派生（`deriver` 策略）与动态接口对象（Dynamic Interfaces / vtable）尚未完整实现。

### 3.5 Elaboration、开放函数（Open Functions）与编译期计算（Comptime）
- **已完成能力**：
  - **候选局部事务与交织求值**：全调用参数与返回上下文联合推断，支持部分已知回调上下文下的 Lambda 与高阶泛型调用。
  - **开放签名与分阶段实例化**（[`compiler/elaboration/src/program/open_functions.rs`](file:///home/fin/luna/compiler/elaboration/src/program/open_functions.rs)）：支持带 `comptime` 参数、类型选择器回调、参数包展开（`...`）、混合阶段结构解构（含 `let-else`、顶层/子模式守卫、`or` 候选一致值交集、枚举载荷证据）的定义侧检查与调用点物化。
  - **活动闭包图传递**：在类型计算调度器与开放函数调用中支持传递有状态的活动闭包图（Live Closure Graph），保持跨参数/默认值的共享捕获状态更新。
  - **Inline 控制流**：接通 `inline if`/`inline when`/`inline match`、有界范围 `inline for` 以及异构非空元组的 `tuple_enumerate` 展开，并与完成性分析（`Never` 传播）和 Drop 清理作用域严密衔接。
  - **语义缓存与跨阶段传递**：完成 `Splice` 协议接入（含高阶函数工厂拼接）以及基于源码 `Eq`/`Hash` + 不可变输入快照（[`Snapshot`](file:///home/fin/luna/compiler/luna/src/comptime_keys.rs)）的 [`MemoizedInvocation`](file:///home/fin/luna/compiler/luna/src/memoization.rs)。

### 3.6 资源管理、程序验证与运行时纯度
- **已完成能力**（位于 [`compiler/elaboration/src/verification/`](file:///home/fin/luna/compiler/elaboration/src/verification.rs)，共 35 个模块、约 4.4 万行源码）：
  - **严格独立于 PTC**：所有程序状态、算术范围、内存与生命周期验证均在独立的 `verification` 层完成，绝不污染 PTC。
  - **机器算术安全门禁（Flow & Range Analysis）**：基于版本化标量身份、三次多项式规范形与分支快照的范围流分析（[`verification/flow.rs`](file:///home/fin/luna/compiler/elaboration/src/verification/flow.rs)），强制证明定宽整数加减乘取反及位索引无溢出/越界；通过验证的检查可在优化 pass（[`verification/optimization.rs`](file:///home/fin/luna/compiler/elaboration/src/verification/optimization.rs)）中安全消除，未知情况保留基准运行时检查。
  - **所有权、移动与部分初始化**（[`verification/moves.rs`](file:///home/fin/luna/compiler/elaboration/src/verification/moves.rs)）：完整追踪 `Once`/`Copy`、字段级部分移出與重初始化、指针可达性、内存视图与六种内存效应。
  - **自动清理计划（Drop Elaboration）**（[`verification/drop/plan.rs`](file:///home/fin/luna/compiler/elaboration/src/verification/drop/plan.rs) 与 [`luna/src/cleanup.rs`](file:///home/fin/luna/compiler/luna/src/cleanup.rs)）：为局部变量、聚合字段、未命中分支、提前返回/`break`、临时值和闭包环境生成精确的逆序清理计划，并在 NIR/IIR 解释器中通过 717 项 `drop` 回归测试检验实际析构事件顺序与次数。
  - **运行时纯度（P1–P6）**（[`elaboration/src/program/purity.rs`](file:///home/fin/luna/compiler/elaboration/src/program/purity.rs)）：支持默认参数 P3 稳定纯度验证、P4 递归行为见证（含不同回调环境与闭包工厂代码证据）等。
- **未完成／受限项**：
  - 源码级 `Copy`/`Drop` 仍通过 CLI 或驱动协议选择器显式绑定，尚未在默认无参数模式下自动从内置 `builtin` 包隐式挂载全部安全模式协议；
  - 整数除法 `/` 与取余 `%` 的源码有符号舍入策略仍等待作者最终语义裁定（底层截断除法内核已就绪）。

### 3.7 元编程（CMTT）与代数效应
- **已完成能力**：
  - **CMTT（上下文模态类型理论）**（[`elaboration/src/program/cmtt.rs`](file:///home/fin/luna/compiler/elaboration/src/program/cmtt.rs)）：已接通带上下文的表达式代码、显式替换、代码生成器及 `quote fn` 展开。生成器支持普通／pure comptime 声明、具名 implicit 供应、数据状态帧、范围及非空 tuple 遍历、代码别名和已知 Type 信息传递。最近专项检查为 203 项 `cmtt` + 85 项 `quote_functions` + 12 项 `comptime_memory`，全部通过；报告 `/tmp/luna-tools-xni4aq1x/result.json`。这不证明完整 CMTT 已实现。
  - **代数效应**（[`elaboration/src/program/effects.rs`](file:///home/fin/luna/compiler/elaboration/src/program/effects.rs)、[`comptime_interpretor/src/effects.rs`](file:///home/fin/luna/compiler/comptime_interpretor/src/effects.rs)）：支持效应声明、操作组、handler 安装、`.use` 处理能力、直接传值、多提示符限定 continuation（单次/尾恢复/逃逸清理）与异步效应在参考解释器中的完整语义执行。
- **未完成／受限项**：
  - quoted pattern／arm、template 接入、异构代码容器、Any/any 代码证据、上下文多态及依赖 runtime identity 的类型替换尚未接通。此前本报告将 quoted 模式与 arm 组合列为已实现，缺乏实现证据，现予纠正。
  - 生成器的方法调用仍受限：现有方法选择返回普通可调用 NIR 签名，代码接口尚不能直接复用该载体；不能只删除方法入口拒绝来宣称支持。空 tuple 遍历的定义检查规则仍待确定。
  - 效应系统的原生池化后端 ABI 与跨语言边界降码尚未进入 LLVM 后端。

### 3.8 高层具体实例 IR（IIR）与原生代码生成后端
- **已完成能力**：
  - [`iir::Package`](file:///home/fin/luna/compiler/iir/src/lib.rs#L91-L103) 从指定 runtime 入口出发，完成泛型与分阶段参数消解（Staging）、按 `(DeclarationId, Vec<Term>)` 去重实例化、保留非 SSA 高级控制流与程序契约、挂载 Drop 清理计划，并通过 `Interpreter::from_iir` 实现独立于源 NIR 的参考执行。
- **未完成／受限项**：
  - **原生后端断层**：`compiler/codegen` 与 `compiler/mir`、`compiler/mir_build` 仍停留在早期依赖旧 `ty` crate 的骨架状态（合计约 2,400 行旧代码），尚未与 `iir::Package` 对接。因此编译器目前可输出 AST/HIR/NIR/IIR 并通过解释器执行验证，但**尚不能从新管线直接编译生成原生机器码或 LLVM IR 可执行文件**（LLVM 21 C API 环境已通过独立探针验证可用）。

---

## 4. 十二个工程依赖组状态总表

对照 [development/implementation-plan.md](file:///home/fin/luna/development/implementation-plan.md#L11-L27) 中的 12 个依赖组，历史阶段估计如下；百分比没有逐项验收分母，不能据此证明功能完成。CMTT 行按近期实现证据修正，其余历史数字未在本次重新统计：

| 依赖组 | 当前阶段评级 | 核心已交付证据 | 主要剩余缺口 |
|---|---|---|---|
| 1. Source and diagnostics | **85%（成熟）** | CLI、精确字面量、90 级解析、HIR S-expression 导出、142 项 CLI 进程级测试 | 字符串插值、宏语法展开 |
| 2. Binding and organization | **80%（单包/Sysroot 就绪）** | 目录发现、模块/导入/再导出/权限分离、sysroot 与多包测试（84 项测试） | 外部 `anar` 对接、增量发现失效 |
| 3. Stable semantic representation | **95%（核心完成）** | NIR 不可变类型 Core、作用域替换、Opaque NF Head、Newtype/Enum/Alias 表示 | 少量高阶依赖环境细节 |
| 4. Elaboration engine | **85%（深度实现）** | 候选事务、全调用联合推断、Open 函数、部分类型上下文、447 项 elaboration 测试 | 更一般的延迟部分类型连续体调度 |
| 5. Comptime and freezing | **85%（深度实现）** | 任意精度 Integer/Float、活动闭包图、Splice、基于 Eq/Hash 的 MemoizedInvocation | 编译期字节码/JIT（`comptime_compiler`）、全自动查询缓存挂载 |
| 6. PTC and trait protocols | **85%（深度实现）** | 等式/分类/整数差分/效应行求解、证明回放、三态 Trait 选择、Coherence、两相分派 | 完整 CDCL(T)、负向 Trait 谓词、动态接口 vtable |
| 7. Functions and runtime types | **90%（核心完成）** | 命名/默认参数、变参展开、闭包捕获环境、`quote fn`、Inline 控制流与元组枚举 | 源码级一般 `for` 循环迭代器协议接线 |
| 8. Resources and runtime spec | **85%（深度实现）** | `Once`/`Copy`/`Drop`、部分移动、717 项 Drop 测试、346 项资源测试、范围流门禁、P1–P6 纯度 | 默认安全模式协议自动隐式挂载、有符号除法舍入裁定 |
| 9. Effects and staging | **部分实现** | 表达式代码生成（203 项专项测试）、quote fn（85 项专项测试）、效应与限定 continuation 解释执行 | quoted pattern/arm、template、代码容器与 Any 证据、生成器方法、原生效应 ABI |
| 10. Projection and synthesis | **65%（BitVec 内核就绪）** | BitVec 值/切片/回写、IEEE 浮点位视图、分段模式与符号覆盖检查 | Pipeline projection 目标后端验证与合成等价优化器 |
| 11. IIR and executable backend | **50%（IIR 完成 / Codegen 待接）** | `iir::Package` 实例化、Staging、清理挂载与 IIR 解释执行 | 重写 `codegen` 将 `iir::Package` 降码至 LLVM 21 |
| 12. Library and anar | **25%（基础骨架）** | `library/builtin`、`meta`、`std` 骨架与 `language-items.conf` | 标准库按最新编译器能力全面实现与验证、定位并对接 `anar` |

---

## 5. 风险、阻塞项与下一步开发建议

### 5.1 已知语义待决项与外部阻塞
1. **有符号整数除法与取余（`/`、`%`）舍入规则**：底层截断算术内核与除零恢复已在 [compiler/elaboration/src/verification/division.rs](file:///home/fin/luna/compiler/elaboration/src/verification/division.rs) 就绪，但源码运算符接线需等待作者确认有符号舍入语义。
2. **源码 `for` 循环的迭代器协议契约**：[library/builtin/iterator.fl](file:///home/fin/luna/library/builtin/iterator.fl) 中的历史 `Iterator.next` 与当前文档规划的 `Iter`/`IntoIterator` + `inline fn` + 代数效应架构存在代差，需先对齐协议签名与语言项后再接通源码 `for`。
3. **`anar` 仓库未就位**：当前工作区及 `/home/fin/anar` 尚无 `anar` 仓库，包管理与构建图外部调度需待仓库就位后开展。

### 5.2 推荐优先推进路线
1. **打通 IIR → LLVM 21 原生代码生成最小闭环**：
   - 绕开已废弃的旧 `ty`/`mir` 栈，直接从已经完成类型消解、静态分派选择和 Drop 清理挂载的 [`iir::Package`](file:///home/fin/luna/compiler/iir/src/lib.rs#L91-L103) 构建向 LLVM 21（`/usr/lib/llvm21`）降码的新后端通道，并用现有的 IIR 参考解释器结果作为黄金对照基准。
2. **推进 `library/`（`builtin` / `meta` / `std`）实战化与默认协议自动装配**：
   - 将测试中显式传入的 `--copy-protocol`、`--drop-protocol`、`--splice-protocol` 等语言项逐步沉淀到 `library/builtin` 与 [library/language-items.conf](file:///home/fin/luna/library/language-items.conf)，使用户源码在 `--sysroot library` 下开箱即享完整协议支持。
3. **继续收敛开放函数、参数包与分阶段实例化边界**：
   - 完成未知分阶段输入的计算型参数包描述路由，以及将 [`MemoizedInvocation`](file:///home/fin/luna/compiler/luna/src/memoization.rs) 正式接入编译器内部的纯 `comptime` 查询调度。
