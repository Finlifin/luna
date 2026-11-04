# 当前架构审计与劣化根因

基线：`70214cdd08f71c4980b5e80a9b288cd573710cc5`，dev，2026-11-03。
架构结论以静态源码核对为主；第 6 节单独记录 2026-11-04 的有限运行基线，
不以历史结果或少量正例冒充新架构验收。
本文件保留初始审计；随后已实施的源码退役与检查见[模块边界](module-boundaries-and-retirement.md)。
完整架构迁移尚未完成。

## 1. 体量与结构事实

初始审计工作区 compiler 下 `.rs/.fl/.md/.py` 共 1,077 文件、766,700 行，含测试及停用目录；
不是生产代码净行数。development 同口径 86,904 行，根目录旧 Markdown 51 份。
其中旧 README 21,185 行、comptime-vm 11,451 行、implementation-plan 6,974 行，
大量按时间追加的结论相互覆盖。规模只用于定位阅读和维护成本，不作为删代码的理由。

已核对的较大生产文件：moves.rs 18,813 行，memory.rs 10,715 行，open_functions.rs
9,160 行，program.rs 8,840 行，comptime_interpretor/lib.rs 8,373 行，flow.rs 8,016 行。
拆文件不是终点；这些文件中上下文管理、证据变换、调度和执行职责是否独立才是问题。

## 2. 重新核对报告后的结论

| 编号 | 当前源码证据 | 已有能力与真实缺口 | 目标处理 |
| --- | --- | --- | --- |
| R01 | `compiler/comptime_interpretor/src/bytecode.rs::bytecode_invocation_available`、`effects.rs::eval_effect` | 字节码受 comptime_only、cleanup、effect environment 门控；runtime effect 参考执行与 comptime invocation 不能混称 | 独立 invocation engine；混合解释保留，语义操作共享；逐操作迁移后删除 invocation fallback |
| R02 | `compiler/comptime_interpretor/src/code_session.rs::CodeSession`、`compiler/luna/src/memoization.rs::Invocation` | 已复用不可变源码世界、索引和代码；不是每次必重建。尚非跨稳定 query 的统一会话 | 区分 WorldSnapshot、CodeCache、VmSession、结果缓存；以版本依赖复用 |
| R03 | `compiler/nir/src/program.rs::NormalFormArgument`、`compiler/comptime_interpretor/src/normal_values/` | NF 有 List/Spread/If/Block/Match 等专用上下文表达式载体，多个解释/替换路径 | 用通用 ContextualExpr 与 ArgumentPlan 代替 NF 专用求值语言 |
| R04 | `compiler/nir/src/normal_forms.rs::{Role,BuiltinCallable}`、`compiler/ptc/src/model_applications.rs` | 内置签名注册有合理 bootstrap 职责；Role::Tuple/TypeList 仍影响通用路径，tuple_heads 被排除于普通模型表 | 保留集中符号绑定，去除通用绑定/身份/模型对库身份的特权；结构观察改受检协议 |
| R05 | `compiler/nir/src/types.rs::{TypeCore,TypeRef}`、`compiler/iir/src/lib.rs::FunctionInstance` | core/substitution 已实现；普通 IIR Function 仍包装 NIR Function。现有两处 TypeRef 用于闭合 callable/template | 建立生产类型引用与环境证据；不能放宽闭合容器契约冒充接通 |
| R06 | `compiler/iir/src/lib.rs::Package::runtime_bindings`、`compiler/iir/src/template_storage.rs` | runtime identity 可用集合不包含域、取得路径、存活证明；没有 EnvironmentPlan 定义 | 带类型的依赖生产、环境取得与资源检查一起接线 |
| R07 | `compiler/middle/src/queries.rs::Providers`、`compiler/luna/src/compilation_cache.rs` | 生产 query 只有包级 hir_package；另有整前端进程内缓存，不能说完全无缓存 | 稳定定义接口/主体/实例边界；局部工作不塞进持久 query |
| R08 | `compiler/elaboration/src/program.rs::ensure_signature`、`compiler/type_resolution/src/obligations.rs::Engine` | 已有候选 metavariable 等待机制；大检查器仍手工保存/恢复多张表，不能说没有任何调度 | Region 统一拥有活动状态，具备真正输入生产者的工作才挂起 |
| R09 | `compiler/ptc/src/search/trail.rs`、`compiler/ptc/src/lib.rs::Atom`、`model_search.rs::witness` | 已有带理由 trail、四路通知、学习与回跳；Atom 四类，trait 原子仍阻止裸理论模型构造 | 在已有算法上补统一理论协议、模型与 guarded 复用；不从零重写 CDCL |
| R10 | `compiler/nir/src/types.rs::Primitive::BitVec` | 宽度为 usize；延后签名能在具体实例算宽度，不代表定义端抽象宽度完整 | 类型层保留合法静态宽度项，具体表示边界再检查宽度及资源限制 |
| R11 | `compiler/elaboration/src/program/function_templates/local_definitions.rs`、`program.rs::ensure_signature` | 已克隆局部 fn/alias/newtype；仍全量继承外围 telescope。已运行局部 helper 正例并确认失败；局部 alias 与顶层 helper 对照通过 | 按[精确依赖反例](dependency-observations.md#5-四个判断精确性是否正确的例子)核验实际 telescope/供应链；不以外提 helper 或改变预期规避 |
| R12 | `compiler/elaboration/src/definitions.rs::prepare_with_invocation_budget`、`compiler/comptime_interpretor/src/generated_structs/admission_session.rs` | batch 准备/原子发布已有机制，多个 struct/enum/newtype 服务仍各自桥接世界 | 统一 DefinitionTransaction、接收世界校验、发布及恢复协议 |
| R13 | `compiler/program_verification/src/memory.rs::pointer_input_layout`、`memory/text_regions.rs::TextRegionSummary` | 已有文本 read/return 来源摘要；pointer-only 消费者拒绝 Text，摘要不含全部清理足迹；真实 sysroot 的文本方法已选中正确 runtime trait，仍在资源检查报 string-view provenance 未实现 | 区分指针目标和文本 backing region，打通调用/捕获/返回/清理；保留[两阶段方法运行基线](libraries-and-delivery.md#111-真实六包的两阶段方法基线) |
| R14 | `compiler/luna/src/host/llvm/`、`compiler/luna/src/host/mod.rs` | LLVM host 已有真实降码，停用 compiler/codegen 并非生产后端 | 暂保留现有 LLVM，先独立已验证 IIR/ABI/目标边界，不先换后端 |
| R15 | `library/std/lib.fl`、`library/builtin/lib.fl`、`library/meta/lib.fl` | std 仍为再导出壳；meta/builtin 已有实际数据与协议。Iterator 已为 inline effect 模式，旧“仍用 next”过期 | 编译器基础改造同时定义真实库贯通验收，不把库重新推迟到最后 |
| R16 | `compiler/hir/src/arena.rs`、`compiler/middle/src/hir_package.rs`、`compiler/middle/src/ty/interner.rs`、`compiler/symbol/src/lib.rs` | 仍有 lifetime/表示擦除 unsafe；仅凭出现 transmute 不能判不健全 | 单独审计所有者、跨世界句柄、析构和并发边界；不先加 Send/Sync |
| R17 | 根 Cargo.toml、各 crate manifest、intrinsic::initialize | 26 个活动成员；HIR 查询实际使用 middle，intrinsic 接收但未使用 TyCtxt；HIR 的 NIR 值入口承载 builder 交付 | 按[模块边界](module-boundaries-and-retirement.md)区分必要通道与旧耦合；先接管消费者再退役，不只移动文件 |
| R18 | `compiler/luna/src/build_context.rs`、`compilation_cache.rs`、`package_stream.rs`、`output.rs` | 已有来源绑定快照、保守前端缓存、顺序驻留及单文件原子替换；字段观察未接入缓存，非空输出不等于完整目标接口 | 保留有效机制，补任务/源执行身份区分、实际依赖及多产物 manifest，不能将 v1 当作异步构建服务 |
| R19 | `compiler/nir/src/target.rs`、`compiler/luna/src/host/llvm.rs::repr_at`、`host/mod.rs::Prepared` | 指针宽度、受预算的机器表示、成对 graph/schedule 已有；ABI 属性未接通，通用布局/调用计划缺失 | 统一反射与后端的表示观察，保留薄指针和已有检查，区分私有机器表示与公开调用契约 |
| R20 | `compiler/nir/src/comptime_values.rs`、`comptime_interpretor/src/normal_terms.rs`、`normal_values/`、`value_reference.rs`、`elaboration/src/program/constants.rs` | list 已按准确域编码；值引用会重验路径，局部闭包已有共享快照和重建 factory；准入/编码/恢复仍跨多个入口 | 统一受检值与传送契约，保留域和图身份；写入只撤销旧内容证据，不一律废弃值引用 |

R01–R20 是当前证据索引，不是永久兼容承诺。位置用文件与符号定位；后续重构同步更新，
不向本文追加几十轮成功日志。

R09 的接线还需区分三个已有层次：`ptc/src/type_inputs.rs` 已有选定投影及符号解，
`type_resolution/src/obligations/type_inputs.rs` 保留原始匹配义务，
`type_resolution/src/capabilities/propositions.rs` 已能将 trait 选择/排除交给精确 world
绑定的证明。裸模型遇 trait 返回 Unknown 不表示这些能力不存在；缺口是联合模型和
跨理论证据的完整交付。迁移细节见[Type 模型与理论证据](type-models-and-theory-evidence.md)。

R16 的旧 middle::ty 已在清理中整体退役。其余边界仍分别核验：HirPackageBox 已有高阶生命周期构造和 owner 借用期访问，
CodeSession 已精确保活世界；HIR arena 载荷析构和 Symbol 指针位编码
仍须具体审计，不能只凭 lifetime 名称或地址稳定认定安全。目标存储与迁移条件见
[世界存储与生命周期](world-storage-and-lifetimes.md)；本次没有执行内存安全动态验证。

R18 的源码入口、保留资产和交付缺口详见[构建与产物交付](build-and-artifact-delivery.md)。
该项为静态审计，不证明实际工具链当前可用，也未运行 native 或驻留测试。

R19 的实际发射与迁移边界见[布局与调用 ABI](layout-and-call-abi.md)。host 控制槽已使用
thread_local 且入口保存/恢复外层控制指针；这不等于一般跨激活恢复已经完整，也不能
误报为所有线程共享一个普通全局槽。本轮没有执行对应动态验证。

## 3. 已经学到的东西应当保留

1. **候选只提出解，原义务仍需兑现。** 普通 NF 输入匹配已摆脱一部分 Tuple/List 特权；
   不可在新的“统一值”中丢失非单射边界。
2. **工作量先看依赖范围。** `38b7dd88` 修正先构造全包 binder 再筛选输入的做法。
   旧记录中小型真实 sysroot dev 检查由 40.620 秒降至 3.026 秒，是该 workload 的历史观测，
   不代表本轮或所有程序。教训是先消除无关扫描，再测 VM 内循环。
3. **不可变世界可以复用，活动存储不能复用。** CodeSession 的 warm/cold 区分有效；
   缓存结果不是解决准备成本的唯一办法。
4. **持久容器的成本要跟实际工作走。** 已有列表共享、拥有迭代、快照失效和预算检查值得保留；
   按逻辑展开长度收费和无条件展平成 Vec 都可能掩盖真实成本。
5. **构造成功不等于可在当前世界使用。** 冻结 callable、生成定义、引用类型都需要接收方准入。
6. **源码通过局部分析不等于原生安全。** text summary 和闭合 TypeRef 单元测试没有证明
   调用者环境及清理有效。测试必须覆盖生产消费者。

## 4. 劣化的主要机制

### 4.1 一个语义操作长出多套不完整解释

普通值、NF 输入、staged input、quoted code、builder 参数各自维护上下文求值与替换。
每新增表达式种类，都要追赶几套 match；“支持这个语法形状”的补丁会继续扩大差异。
替代边界是统一参数计划、绑定身份和受检替换，不是把所有值塞进一个无类型枚举。

### 4.2 缺少生产证据时，在消费者重复猜测

运行时依赖缺域和取得路径，消费者只剩身份集合；文本缺 backing region，消费者只看到
指针布局；lambda/模板缺精确依赖，签名阶段就捕获整个外层。下游无法可靠补回丢掉的信息。
必须由首次知道事实的生产者交付证据，再由变换维护；拒绝不应靠不断补白名单解除。

### 4.3 一个可变检查器承担太多事务

手工保存 owner、locals、captures、returns、context、pending obligations 的方式，使
新增字段很容易漏掉回退。应以 Region 和 CandidateTrial 的所有权表达隔离，稳定索引共享。
不把 rollback 理解成撤销外部 comptime 副作用。

### 4.4 两种“完成”混用

结构合法、类型形成、契约证明、实例可执行、目标可降码是不同完成条件。
当前同一个 Term/Function 在多个阶段流转，容易让检查过某一条件的节点被当成全面可信。
用受限构造入口与产物类型体现边界；不要只增加 `is_checked` 布尔字段。

### 4.5 局部优化和局部成功缺少退出条件

同一路径已经快速却继续微调，未接通路径反复留记录；新增 helper 或 crate 却仍由旧大函数
掌握所有状态，都不能算架构迁移。每个迁移阶段须说明删掉哪条旧语义路径、哪组消费者
开始真正消费新证据，以及哪条真实库用例成为可能。

## 5. 清理分类

| 类别 | 例子 | 处理条件 |
| --- | --- | --- |
| 过期记录 | 按轮追加的开发日志、停用 graph 流程 | 语义和未完成边界提炼后删除；Git 保留历史 |
| 重复语义路径 | NF 专用表达式求值、分散冻结恢复 | 通用替代者覆盖域、顺序、失败和消费者后删除 |
| 错位所有权 | 大检查器共享临时状态、跨世界缓存引用 | 新所有者真正接管全部生命周期后迁移 |
| 历史停用源码 | typeck/mir/mir_build/codegen | 先查工具、文档、fixture 引用及独有设计，再作删除决定 |
| 有意的多载体 | mixed interpretation、invocation、runtime reference | 保留职责，抽取共享语义；不能按重复名称删除 |
| 有证据的优化 | 持久值共享、索引、证明重放 | 保留并纳入新边界，禁止为整齐退回全量复制 |

本轮只实施文档清理，尚未执行上述任何源码删除或迁移。

## 6. 可复现的现状基线

2026-11-04 在 `/home/fin/luna` 从当前源码构建了独立 driver。
构建时 HEAD 为 `7ddc13d5`，**包含工作区原有、未提交的
compiler/program_verification/src/moves/effects.rs 修改**，不是纯 HEAD 构建。
该修改未由本阶段改写或提交。构建前状态、源码 diff 与受保护文件摘要保存在
`/tmp/luna-design-baseline-run-j1ebxbgw`，独立 driver 为 `/tmp/luna-design-baseline-driver`。

```text
driver sha256:
ba11923643f874414560f70c3d27d821cbb8dc7d61e2b4483033b8aa078fa1a8
```

执行入口如下，所有命令均在仓库根运行：

```sh
python3 -B development/command.py --timeout 900 -- python3 -B development/build_host_driver.py /tmp/luna-design-baseline-driver
python3 -B development/reproduce.py --timeout 60 --binary /tmp/luna-design-baseline-driver -- interpret development/examples/sysroot-reflected-field-objects.fl --entry run --sysroot library --build-target x86_64 linux 64 --fuel 4000000
python3 -B development/reproduce.py --timeout 60 --binary /tmp/luna-design-baseline-driver -- interpret development/repros/default-local-comptime-binding.fl --entry run --sysroot library --build-target x86_64 linux 64 --fuel 4000000
```

| 项目 | 实际结果 | 本机证据 |
| --- | --- | --- |
| 受管 dev 构建与独立 driver 保存 | 退出 0，存在编译警告 | `/tmp/luna-tools-s5xb0qfx/result.json` 及 stderr.log |
| 真实 sysroot 反射 Object、字段编辑、StructBuilder、生成类型及普通调用 | 退出 0，stdout 为 `(bool true)`，stderr 空 | `/tmp/luna-tools-t6qx5l16/result.json` 与 stdout.log |
| 默认供应中的局部 comptime const 与不同 mode 实参 | 退出 0，stdout 为 `(bool true)`，stderr 空 | `/tmp/luna-tools-qma9rt_9/result.json` 与 stdout.log |

这两项是现有公开路径的保真输入，应在相应迁移中继续成立。它们不证明任意 definition
builder 已完整、不证明无关外层输入捕获已修正，也不证明 Tuple/List 专用 Role 已退役。
执行使用 reference interpreter，未做 LLVM/native、Miri、完整资源验收或全量测试。
两个进程独立运行并有时间重叠，报告耗时不是隔离性能基准，不与旧性能数字作速度比较。

环境检查还发现：受管 `/tmp/luna-dev-002/target` 原先不存在，仓库旧 `target/debug/luna`
并非当前子命令 CLI；调用其 `--help` 实际运行旧示例并生成被忽略的 ast.lisp/hir.lisp。
该旧程序输出不能作为当前源码行为证据，后续复现显式指定上述新 driver。
临时路径只定位本机证据，不是仓库依赖；未来重现应重新构建到一个未存在的独立路径，
不能凭文件名复用来源不明的旧二进制。
