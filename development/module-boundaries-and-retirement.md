# 模块边界与源码退役依据

状态：已按审计删除四个停用 crate 的 11 个文件及根 manifest 停用条目。
middle 的旧类型子系统及关联包装也已退役；下文的现状表保留清理前审计依据。
本文件把[逻辑架构](architecture.md)落实到现有入口；不要求每个职责独立建 crate，
也不以文件体积、命名陈旧或测试暂时未覆盖作为删除依据。

## 1. 实际依赖与注释中的架构不同

初始清理基线的根 [Cargo.toml](../Cargo.toml)启用 26 个 workspace member。
11-15 在 `3fd323ca` 上直接读取 manifest 为 28 个，后续加入了 fmt 与 anar；
下文的退役证据仍对应清理时的成员集合，不把后续工具变更计作旧 crate 退役。
清理前 compiler 下另有 codegen、mir、mir_build、typeck 四个未启用的 manifest，
它们仍引用不存在的 compiler/ty 路径。注释中的旧阶段图及被注释掉的 luna 条目
不代表生产依赖；luna 已在活动成员中。这里仅核对 manifest，没有尝试构建停用 crate。

| 清理前边界 | 实际证据 | 对迁移的含义 |
| --- | --- | --- |
| ast_lowering → middle | [providers.rs](../compiler/ast_lowering/src/providers.rs)使用 Db、Providers、HirPackageBox | 当前依赖主要在查询适配层；lowering 算法不因此需要旧 TyCtxt |
| middle → HIR/query 与旧 ty | [queries.rs](../compiler/middle/src/queries.rs)同时拥有 HIR 输入、缓存及 TyCtxt | 不能删除 middle 整包来清理旧类型系统；先分离实际职责 |
| intrinsic → middle | [initialize](../compiler/intrinsic/src/lib.rs)接收 `_ty_ctxt`，函数体只建立 lang-item 表 | 类型上下文参数是当前未使用的耦合；注释声称注册类型签名不足以证明实际用途 |
| interface → middle/intrinsic | [CompilerInstance](../compiler/interface/src/lib.rs)构造 Db、传入 TyCtxt，并暴露旧查询包装 | 解除依赖须同步构造入口和公开包装，不能只删 intrinsic 参数 |
| HIR → NIR | [DeliveredType](../compiler/hir/src/expr.rs)及 builder 辅助入口携带 `Arc<nir::Term>` | 已有值进入生成语法是必要通道；解除 crate 依赖不能把值退化成文本再解析 |
| materialization → interpreter | [staged.rs](../compiler/materialization/src/staged.rs)使用 Interpreter、Value 及契约结果 | 实例化确实需要执行能力；应限定执行协议，不假装可以完全移除执行依赖 |
| program_verification → materialization | [memory.rs](../compiler/program_verification/src/memory.rs)解析方法实例 | 资源分析的摘要/实例请求要有独立边界，不能借此获得整个可变 elaborator |
| luna → comptime_executor | [memoization.rs](../compiler/luna/src/memoization.rs)使用 Cache、Context、KeyProtocol | executor 当前是调用缓存基础设施，不能依据名称把它当成完整 VM |

单包入口 [frontend.rs](../compiler/luna/src/frontend.rs)和多包入口
[packages.rs](../compiler/luna/src/packages.rs)都构造 HIR 查询输入。
只接通其中一个入口不能证明查询层已经迁移。`HirPackageBox` 的所有权与借用保证、
query 的失效和循环检测需要保留，不能随着旧 TyCtxt 一起抛弃。

## 2. 分离数据契约、服务协议和装配

第一步保持现有 crate 能构建，在内部拆职责并收窄可见性；只有真实依赖需要时再移动 crate。
目标不是让所有层互相无关，而是让依赖携带的信息与可执行的权限清楚。

| 边界 | 允许跨越的内容 | 不允许隐含获得的权限 |
| --- | --- | --- |
| HIR / 稳定语义数据 | 已交付值、域与来源的稳定引用，接收世界准入所需身份 | VM 活动槽、候选替换、执行整个函数体的隐式能力 |
| lowering / 查询适配 | 源码与解析输入、拥有 HIR 的结果、诊断及依赖 | 旧类型 arena、调用方未声明的全局当前包 |
| elaboration / solver | 有作用域的类型义务、候选私有状态、模型及解释证据 | 任意程序状态证明、直接提交候选或发布定义 |
| VM / 宿主 | 带动态请求身份的输入、响应、暂停与取消状态 | 全包可变检查器、重放已完成不纯计算 |
| 实例化 / 执行 | 冻结定义、显式替换、所需执行服务、带来源的结果 | 自建另一套值准入及类型身份规则 |
| 程序检查 / 实例服务 | 指定世界中的实例或契约摘要及其依赖证据 | 根据分析顺序隐式改变候选世界 |
| 后端 / 驱动 | 已验证交付、布局/调用计划、目标及产物请求 | 自行猜测缺失资源来源或重新检查语言类型规则 |

稳定语义数据不能直接依赖 provider 注册、查询缓存或 VM 执行器。
目前 nir 依赖 comptime_runtime 的基础值表示并不自动构成错误；要辨认被依赖的是
整数/对象等数据，还是活动执行状态。不得为了让依赖图看起来分层而复制这些数据类型。
同样，HIR 携带已求出的 Type 可以保留，但入口仍须证明作用域、世界与值域合法，
不能把一个任意构造的 Term 包装成“已经检查”。

协议的抽象也不能只是把 `&mut Elaborator` 藏在 trait 对象后面。
宿主请求须能说明输入的所有者、所用 SemanticView、完成后可交付的证据以及取消规则；
同步实现可以直接返回，需等待时沿[状态协议](elaboration-state-and-publication.md)保存进度。
程序检查的请求不能在借用内部可变分析状态期间任意重入相同实例；递归摘要按其专门协议
处理，不能把正在计算的结果当成成功。

装配由 luna 驱动及其内部接口层负责；协议实现留在对应服务中。
暂不创建新的全局上下文 crate，也不强制删除 interface 这个名字。
明确所有者与调用关系后，才决定哪些协议适合独立 crate，哪些只需受限模块接口。

## 3. 旧路径分批退役

| 对象 | 已知承载内容 | 允许退役前必须闭合的事项 |
| --- | --- | --- |
| middle::ty 与 Db 的旧类型字段 | TyCtxt 存储、interface 查询包装、intrinsic 初始化参数 | 枚举真实调用者；解除未使用参数；迁移仍需要的能力，保留 HIR 查询及所有权 |
| ast_lowering 的 Db/provider 适配 | 多文件 lowering、生成目录模块、模板链接、诊断 | 新适配器交付同一完整 HIR；单包和多包都切换，输入变更仍正确失效 |
| HIR 中直接引用 NIR 的入口 | builder 的已完成字段类型和 impl self 类型 | 如移动数据层，值域、身份与准入一起迁移；不增加字符串或专用 Tuple 旁路 |
| typeck | 旧类型检查与类型表达式解析 | 审查旧算法及输入资产；按正式语义辨认可保留用例，不能原封不动继承旧预期 |
| mir / mir_build | 基本块、place、控制流与聚合降码 | 核对独有处理及 dump/工具消费者；不因此把新 IIR 强制改为旧 MIR |
| codegen | 旧 LLVM C API 降码、对象输出与测试入口 | 核对仍有价值的用例/算法；生产后端以 luna/host 为准，不恢复旧后端来凑统一目录 |

四个停用 crate 的仓库内消费者与资产检查见下节。外部未登记消费者不在本次可观察
范围内，不能宣称对它们兼容；也不以假想消费者为由永久维持两套语义实现。
公开 Rust 包装的移除应明确记录为内部 API 退役，而不是伪装成兼容实现。

每个新入口先接管实际消费者，再移除旧入口；adapter 只保留单向转交。
不能让旧 Ty 与新 TypeRef 各自维护一套可修改的类型身份，再靠调用顺序决定谁覆盖谁。
缓存、错误诊断、预算耗尽和失败后恢复同属迁移契约，不是删除后的补充工作。

## 4. 停用源码已退役，保留清理依据

2026-11-04 在 `feeda203` 工作区解析全部活动成员 manifest，并扫描 Git 跟踪文件中的
旧目录、crate 引用及 `typeck_package`、`resolve_ty_expr`、`build_mir`、`codegen_llvm`
入口。完整文件 hash、依赖边及命中位置保存于
`/tmp/luna-retirement-audit-5npq1wjm/inventory.json`。

四个停用目录只有 11 个跟踪文件：4 份 Cargo.toml 和 7 份 Rust 源码；未包含独立
fixture、tests 或 examples，源码也未声明 `#[test]`。26 个活动成员没有指向它们的
本地依赖，Cargo.lock 未列入这四个 package。仓库跟踪的开发脚本没有调用上述旧入口，
当前也没有 `.github` 目录。目录外的匹配是根 manifest 的注释、历史报告及本阶段审计。
这些结论覆盖仓库内可见输入，不证明外部脚本、外部 CI 或未登记路径依赖不存在。

旧代码不能作为一组“必须恢复通过的测试”迁移：

| 资产 | 实际内容 | 清理决定 |
| --- | --- | --- |
| typeck | 按名字识别 primitive/旧别名、专用 Tuple；部分类型不匹配仅记录而不诊断 | 从生产树退役；不搬回旧名称规则和宽松检查 |
| mir / display | 基本块、place、Copy/Move、聚合及非 S-expression dump | 控制流与位置模型可供历史参考，不保留第二套中间类型身份或旧 dump 契约 |
| mir_build | 无法识别的表达式落为 Unit，无法识别的位置落为 return place | 退役，不把这些回退当作新降码的容错策略 |
| codegen | LLVM C API 发射与对象输出；无 main 时生成测试 main，按形参数量塞固定整数并打印 | 不迁移这个伪测试入口；后端继续使用 luna/host 和明确的用户入口、目标、测试协议 |

对应源码锚点为 `typeck/src/resolve_ty.rs::resolve_ty_expr_in`、
`typeck/src/check.rs::check_body`、`mir_build/src/lib.rs::{lower_expr,lower_place}`、
`codegen/src/lib.rs::{codegen_test_main,codegen_llvm}`。未尝试修复缺失的 compiler/ty
依赖或启动旧后端；上述算法阅读也不构成新旧后端行为等价证明。

据此已将四个目录及根 manifest 的对应停用注释作为独立单元删除，提交 `c5f4481d`。
删除前保留基线与文件清单，删除后核对 26 个活动成员的本地依赖闭包，并通过
`check -p luna`（`/tmp/luna-tools-dxcnvqmt/result.json`）。历史由 Git 恢复，不另建平行实现。
根 COMPILER_PROGRESS_REPORT.md 的旧后端断层描述是历史快照，不能用来判断当前
luna/host 不存在。本阶段仍只修改 development，未改写该报告或正式 docs。

### 4.1 middle 的旧类型子系统须另作一个完整退役单元

清理前活动 manifest 中，ast_lowering、interface、intrinsic、luna 四者依赖 middle。
进一步核对源码：旧类型侧表的可见调用链是 interface 的 `adt_def/def_ty_exists`
包装 → LunaDatabase 的同名方法 → TyCtxt；未找到活动生产消费者向这些侧表注册类型。
intrinsic 的 `_ty_ctxt` 形参未被函数体读取。不能因此连同 Db/HIR 一起删掉。

该退役单元已一起处理：middle::ty 及其重导出、Db::ty_ctxt 的构造与字段、
LunaDatabase 的两个旧类型查询、interface 对应包装与导入、intrinsic 的无用形参及其
middle 依赖；同步删除 middle 对旧 arena/集合库的两条直接依赖，更新 Cargo.lock，
并修正声称仍有旧类型 pass 的模块说明。未保留空实现，也未将旧 NFId
转换成新类型身份。HIR 查询、provider 注册、失效、所有者和 session 生命周期保持
现有实现；未改名或拆出新 crate，也未增加 unsafe Send/Sync。

已针对必须保留的行为运行现有测试：

```sh
python3 -B development/check.py --timeout 180 -- test -p middle --test query_inputs
```

工作目录 `/home/fin/luna`，退出 0；两个测试分别覆盖跨 Db clone 的输入替换及观察者
失效，以及 Db/输入销毁后拥有的 HIR 仍可读取和移动。报告在
`/tmp/luna-tools-2xbgrcq1/result.json`，stdout 记录两项通过，stderr 含现有构建警告。
这是修改前保留能力的基线，不是删除后的验收，也未覆盖跨线程安全或全部 HIR 查询。
删除 TyCtxt 后仍须核对原有生命周期拒绝用例和 auto-trait 变化，不能因某个非 Sync
字段消失就宣布 Db 可以并发访问，更不能手工增加 Send/Sync 来凑构建通过。

## 5. 与第一条纵向迁移的关系

[迁移计划](refactoring-plan.md)中的用户工厂 → 普通值 → 类型 → builder/调用仍是主干。
上述模块调整服务于这条生产路径，不能先交付一批无人调用的新 crate，再声称完成重构。
开始源码迁移时，用真实 sysroot 输入核对单包与多包入口，检查旧入口是否仍被调用，
以及失败、重新发布和输入变化是否仍保持相同契约。

四个停用 crate 已删除，活动 workspace 成员保持原有集合；旧源码可从 `20af1c41`
恢复。middle 旧类型侧表已退役；通用参数、世界存储和 TypeRef 等语义迁移尚未完成。

接线变更后的检查均在 `/home/fin/luna` 通过，使用 development/check.py 调用受控 Cargo：

- `test -p middle -p interface -p intrinsic`：HIR 输入失效/所有权、intrinsic 注册测试及
  四项生命周期/线程 compile-fail 用例通过；报告 `/tmp/luna-tools-1wj2_9j4/result.json`。
- `test -p luna --test function_template_local_definitions clause_defaults_rebind_their_local_definitions -- --exact`：
  真实 sysroot 的模板默认值路径通过；报告 `/tmp/luna-tools-k450175d/result.json`。
- `test -p luna --test packages imported_generic_function_executes_from_its_provider_package -- --exact`：
  多包入口与提供者泛型调用通过；报告 `/tmp/luna-tools-q7ys3_j9/result.json`。

这些检查验证本次退役的消费者闭包；不代表全部架构迁移或历史已知失败已修复。

## 6. 执行测试驱动与生产调度分离

沿实际调用关系核对，`evaluate_contextual_argument`、`evaluate_input_preparation`、
`stage_normal_argument` 及其同步 InputFrame.run 都只被测试使用；生产消费者已由
`input_collection::Frame::next` 选择输入帧，交给 invocation session 驱动与完成。
原入口混在生产 impl 中，容易被误认为第二条仍需迁移的运行路径。

这些比较驱动与原有 execute_code 已移入 cfg(test) 的 `execution_test_support`；
多余的 try_stage_bytecode 包装删除。它们继续调用真正的帧准备、解释、完成和环境
恢复操作，原测试未删除，也没有把语义实现复制成测试替身。借入根帧的
Session::fragment、run_fragment 与该根的显式 cancel 同样仅在测试构建存在。
生产 session 仍拥有并释放自身执行栈，所有输入准备和宿主请求仍经过原生产入口。

借用式 fragment 的暂停／恢复测试能验证共用 driver 的状态转换，不能单独证明
前端已能交付待决源码片段。本次是生产／测试边界清理，不是前端源 activation
接线，也没有删除仍在使用的树解释器或 NormalFormArgument 表示。

验证目录 `/home/fin/luna`，执行
`python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib -p luna --test invocation_host_requests --test type_invocation_sessions --test staged_invocation_reuse --test invocation_value_templates --test normal_input_plans --test ordinary_clause_collectors --no-fail-fast`：
`/tmp/luna-tools-l9volhbp/result.json` 退出 0，762 项通过、1 项原有 ignore。
包含 VM 单元比较及真实前端入口；未更改测试预期，未运行全 workspace。
首次编译发现 bytecode_compilation 测试仍需访问迁移后的 InputFrame.run，已修正
测试模块可见性；最终结果来自修正后的源码。验证包含原有资源分析工作区修改。
修改文件的格式检查 `/tmp/luna-tools-ue_e6r_r/result.json` 退出 0。

## 7. Contextual 调用保留一个完整上下文入口

CMTT generator、pattern generator 和展开参数已使用能接收完整上下文的入口，
旧的省略上下文包装没有生产或测试消费者。现删除九个无消费者的方法，保留原有
检查实现，并将实际调用者改用规范名称：

| 边界 | 当前入口与显式输入 | 退役内容 |
| --- | --- | --- |
| generator 候选 | `generator_call_candidate` 同时接收结果类型与结构上下文 | 自动补 None 的旧同名包装 |
| pattern 调用 | `invoke_pattern_generator`／`invoke_pattern_properties` 接收 subject 与 interface context | 三层不带 interface context 的便利包装 |
| 展开参数 | `spread_input_packet_contexts` 接收已检查 packet 宽度；推导保留 `infer_routed_packet_inputs` | 空宽度包装、partial context 转换包装及无调用的 infer_spread_inputs |
| 只读元数据 | 保留正在使用的字段类型、来源观察与完整 trait 域查询 | 无调用的 SpreadPlan.width 与 interface_parameter_binders |

完整上下文仍可显式传 None；这次不增加推导规则、不提前执行 packet 或默认值，
也不改变 quote／pattern 的类型、phase 或身份。旧包装均为内部可见性，不存在
仓库外可调用的 Rust 公共 API。源码检查覆盖整个 compiler，当前实现和回归均没有
借用旧包装；不能据此宣布 CMTT、参数计划或前端可恢复检查整体完成。

验证目录 `/home/fin/luna`，统一入口 `python3 -B development/check.py --timeout 900 -- test`：

- `-p elaboration --lib -p luna --test cmtt --test cmtt_methods --test cmtt_type_inputs --test quote_functions --test effect_spreads --test function_template_inputs --test ordinary_callable_types --no-fail-fast`：
  1,237 项通过、1 项原有 ignore，`/tmp/luna-tools-slft4kds/result.json` 退出 0。
- `-p luna --test contextual_spreads --test spread_control --test type_spreads --no-fail-fast`：
  114 项通过，`/tmp/luna-tools-khwo7ray/result.json` 退出 0，覆盖部分结果上下文、
  方法／属性路由、输入次序、默认值和提前退出。未修改测试或语义预期。

源码格式检查 `/tmp/luna-tools-oxx4gx0n/result.json` 退出 0。上述为包含原有资源分析
修改的工作区验证，未运行全 workspace 或性能基准。

## 8. Type 依赖队列退出生产模块

按 `30d62729` 的调用者核对，`elaboration::type_computations::Session` 在
`627ab804` 移除 required Type 专用分支后，只剩测试消费者：elaboration 的
`type_computation_demands`、required/prepared 内联测试，以及 luna 的 execution、
builder_invocations 和 type_invocation_sessions。正常前端、CLI 和库源码路径
均不调用它。不能把测试编译出的完整 NIR 包交给队列的实验当作源 Region 恢复。

三个文件现整体移至 `compiler/elaboration/test_support/type_computations/`。
elaboration 仅在 cfg(test) 中私有加载；各集成测试显式加载同一份测试模块，复用
gradual 描述、type_resolution 的真实推断和赋值权限，以及真实 VM。共享模块原有
的 admission budget 单测移到 type_computation_demands，避免每个加载者重复注册。
生产库不再
导出该队列，也不再保留只供它使用的 fresh_identity 导入。没有新增 Cargo feature、
替代解释器或复制语义实现；所有旧断言与 S-expression dump 保留。

`nir::type_computations` 未迁移：它的残余调用、scope 替换、依赖检查及预算辅助
仍有大量生产消费者。type_resolution 的 AssignmentPermit 也继续作为独立的
受检赋值能力保留；本项删除的是把每个 Type invocation 绑定一个输出 MetaId 的
旧调度器生产身份，不是撤销身份消费者需要的权限检查。

源码普通 Type 返回继续走 PreparedRequired 和通用 packet 恢复。完整根依赖、
源码／VM 挂起恢复、Type 描述数据和多消费者准入仍未接通；清理后不能重新用旧
队列填充这些位置，也不以本次构建和协议测试通过声称恢复功能完成。

本阶段在 `/home/fin/luna` 经 `development/check.py --timeout 900 --` 验证：

- `test -p elaboration --lib --test type_computation_demands --no-fail-fast -- --test-threads=1`：
  最终 266 项单元测试、9 项需求测试通过，退出 0，
  `/tmp/luna-tools-jf8_6a7a/result.json`。最初路径迁移因子模块查找目录错误而
  编译失败（`/tmp/luna-tools-eawhxm3y/result.json`）；改为目录内 mod.rs 后修复。
- `test -p luna --test builder_invocations --test type_invocation_sessions --no-fail-fast -- --test-threads=1`：
  两组通过，退出 0，`/tmp/luna-tools-qqaddiy_/result.json`，输出分别为 87 和
  14 项。该次各组还注册了同一个共享 admission budget 单测，随后将它移至需求
  测试且在上一项最终检查中通过；不将重复注册计作新的覆盖。
- `test -p luna --test execution -- --test-threads=1` 加报告 command 中列出的
  26 个队列消费者过滤项：退出 0，26 项通过，
  `/tmp/luna-tools-h9m67mo3/result.json`。未执行整个 execution 测试集。
- `check -p luna --lib --bin luna`：默认生产构建检查通过，退出 0，
  `/tmp/luna-tools-3x316uuq/result.json`；不通过 feature 重新导出测试调度器。

各报告的 stdout 保留 `test result:`，check 的 stderr 保留构建完成位置。
两个子模块 demand/execution 与迁移前字节相同；mod.rs 除模块说明、导入路径、
测试注册位置外保持原实现，测试预期未变。9 个改动 Rust 文件的 scoped rustfmt
检查退出 0（`/tmp/luna-tools-yc532qm3/result.json`），已更新开发文档的本地链接
目标全部存在。未运行全 workspace；原有工作区改动保留，docs 未修改。


## 9. Gradual 测试模型与生产域观察分离

在 `c01ed925` 上核对调用者，`elaboration::gradual` 的 compatibility、join、
RefinementScope、冻结和签名 API 均只被测试使用。生产中唯一引用是
`open_functions` 借用 FrozenDescription 的两态枚举；Pending、InputType 与
Suspended 注解都被观察为 Any。该名字混淆了“没有域证据”和“求得显式 Any”。

普通源码检查现在直接使用私有 `TypeObservation::{Known, Unknown}`，保留原有
各处检查与拒绝条件，只将未知域诊断改为准确的表述。观察类型有独立 S-expression
Display，不拥有调用状态，不提供描述相容性或 refinement 准入 API，也不作为
VM 的 Type 值载荷。显式 Any 仍需要共同数据、作用域与调用方准入接线。

原 gradual 模块整体移到 `compiler/elaboration/test_support/gradual.rs`；除模块
说明与导入路径外实现不变。elaboration 单元测试与五个集成测试根加载同一模型，
仍使用真实 type_resolution 与 PTC。生产 crate 不再导出它，不增加 feature、
替代解释器或库名特判；旧模型验证继续保留，不冒充普通源程序的验收。


本阶段工作目录 `/home/fin/luna`，使用 `development/check.py --timeout 900 --`：

- `test -p elaboration --lib --test gradual --test type_computation_demands
  --no-fail-fast -- --test-threads=1`：三组通过（266、20、9），退出 0，
  `/tmp/luna-tools-_rpilfa4/result.json`。
- `test -p luna --test ordinary_callable_types --test ordinary_clause_collectors
  --test open_input_observation --test open_functions --test builder_invocations
  --test type_invocation_sessions --test library_std_format --no-fail-fast --
  --test-threads=1`：除下述 open_functions 的 4 项外通过，两个原有待定 Type
  正例仍 ignored，整体退出 101，`/tmp/luna-tools-rcgs8503/result.json`。

失败分别为 `runtime_staged_callbacks_prepare_factories_after_generic_substitution`、
`runtime_staged_callback_aggregates_prepare_generic_producers`、
`runtime_staged_callback_aggregates_prepare_enum_producers` 和
`runtime_staged_newtypes_keep_callback_dependencies`；均在运行入口报
`staged invocation result cannot yet be frozen as an instance input`。

为隔离回归，临时将本次独占的 lib.rs、open_functions.rs 和 gradual.rs 换回
`c01ed925` 内容，保留其余工作区文件，按这四个名字过滤同一 open_functions
测试目标；4 项仍以相同错误失败，退出 101，
`/tmp/luna-tools-08klmabm/result.json`。随后三处源码逐字恢复本次版本。
这是修改前后对照，不是纯 HEAD 或完整语言验证；没有修改失败测试或忽略标记。
冻结边界的既存缺口仍需修复，不能用本次清理掩盖。

9 个 Rust 文件 scoped rustfmt 检查退出 0，
`/tmp/luna-tools-5sqc02ay/result.json`；正式 docs 与原有工作区改动未纳入提交。

旧调度器的 26 个 execution 消费者按既有过滤名单执行，退出 0，
`/tmp/luna-tools-mw5ajn01/result.json`；未运行整个 execution 集。
`check -p luna --lib --bin luna` 也退出 0，
`/tmp/luna-tools-yt3ohe06/result.json`，确认正常生产构建不依赖测试模型导出。
