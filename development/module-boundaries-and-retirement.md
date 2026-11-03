# 模块边界与源码退役依据

状态：静态依赖审计与迁移设计，尚未修改 Cargo manifest 或源码。
本文件把[逻辑架构](architecture.md)落实到现有入口；不要求每个职责独立建 crate，
也不以文件体积、命名陈旧或测试暂时未覆盖作为删除依据。

## 1. 实际依赖与注释中的架构不同

根 [Cargo.toml](../Cargo.toml)当前启用 26 个 workspace member。
compiler 下另有 codegen、mir、mir_build、typeck 四个未启用的 manifest，
它们仍引用不存在的 compiler/ty 路径。注释中的旧阶段图及被注释掉的 luna 条目
不代表生产依赖；luna 已在活动成员中。这里仅核对 manifest，没有尝试构建停用 crate。

| 当前边界 | 实际证据 | 对迁移的含义 |
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

## 4. 仓库内清理范围已核定，源码尚未删除

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

据此，后续允许修改源码时可将四个目录及根 manifest 的对应停用注释作为一个独立
退役单元，不必等待新 TypeRef、VM 或全后端重构完成。删除前保存基线提交及文件清单，
删除后核对路径依赖闭包与活动入口检查；历史由 Git 恢复，不另建一份平行可构建实现。
根 COMPILER_PROGRESS_REPORT.md 的旧后端断层描述是历史快照，不能用来判断当前
luna/host 不存在。本阶段仍只修改 development，未改写该报告或正式 docs。

### 4.1 middle 的旧类型子系统须另作一个完整退役单元

活动 manifest 中，ast_lowering、interface、intrinsic、luna 四者依赖 middle。
进一步核对源码：旧类型侧表的可见调用链是 interface 的 `adt_def/def_ty_exists`
包装 → LunaDatabase 的同名方法 → TyCtxt；未找到活动生产消费者向这些侧表注册类型。
intrinsic 的 `_ty_ctxt` 形参未被函数体读取。不能因此连同 Db/HIR 一起删掉。

该退役单元须一起处理：middle::ty 及其重导出、Db::ty_ctxt 的构造与字段、
LunaDatabase 的两个旧类型查询、interface 对应包装与导入、intrinsic 的无用形参及其
middle 依赖；同时修正声称仍有旧类型 pass 的模块说明。不能保留空实现或将旧 NFId
转换成新类型身份来维持表面兼容。HIR 查询、provider 注册、失效、所有者和 session
生命周期继续由现有机制承担；本项不要求先改名或拆出新 crate。

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

当前完成的是依赖、资产与退役范围核定，并执行了上述现有 HIR 查询基线。
未删除源码或修改依赖；具体 Rust 接口及迁移后的接线验收尚未完成。
