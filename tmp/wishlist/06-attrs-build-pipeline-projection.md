# Compiler Wishlist 06: `attrs` 属性体系、`build` 构建系统与 `pipeline-projection` 异构投影协议

> **撰写角色**：`@meta-author`  
> **目标交付物**：`/home/fin/luna/tmp/wishlist/06-attrs-build-pipeline-projection.md`  
> **状态**：Wave 2 完整需求规格与缺口清单（统一收口 `__flurry_*` 官方属性键、`library/build` 六子目录、`luna` 单二进制构建任务、以及 `pipeline-projection` / `intrinsic` / 内联汇编 / 后端 IIR 执行协议）

---

## 1. 范围、权威来源与归属包

### 1.1 覆盖范围

本 Wishlist 覆盖 Flurry 官方编译属性体系、源码归属构建上下文与多管线编译投影栈的全部核心机制：

1. **`attrs` 包与全量 `__flurry_*` 官方属性注册表**：系统化梳理并补齐 `layout`（`packed.{byte, bit}` 与提案 `align(bytes)`，废除旧 `{ packed: true }`）、`calling_convention`（`abi`）、`ffi`（`link`, `link_name`）、`staging`（`comptime_phase`）、`effects`（`direct_passing` 与有限栈/续体策略属性）、`mem`（`volatile`）、`derivation`（解决 `default` 保留字冲突）、`stability`（`stable_since`）、`compilation`（`pipeline`, `project`, `intrinsic_enum`, `inline`, `cold`, `hot`, `target_feature`, `caller_location`）、`macros`（`ast`, `peg`, `macro3`, `macro4`）、`quote_fn_template`（`for_match_arms`, `allow_non_exhaustive_instance`）及 `metadata`（`metadata(Object)`）；同步对齐 [05-meta-cmtt-macros-reflection.md](file:///home/fin/luna/tmp/wishlist/05-meta-cmtt-macros-reflection.md)（`OQ-5.9`）与 [07-std-mem-collections-io-os.md](file:///home/fin/luna/tmp/wishlist/07-std-mem-collections-io-os.md)（`WL-07-08/17/18/20`）中所有尚未登记的 `__flurry_*` 键。
2. **`build` 包全 6 个子目录（`info`, `package`, `targets`, `tasks`, `generated`, `util`）**：严格保留**源码定义包归属（source-owned build context）**与 **host vs target vs device** 三重分离语义；设计面向单一 `luna` 二进制 CLI 与驻留协议的包清单、依赖图、构建任务 DAG、产物缓存失效键、代码生成产物登记及带依赖自动追踪的沙盒构建工具接口（**严禁回归独立 `anar` 计划**）。
3. **`pipeline-projection` 完整协议栈**：覆盖跨管线投影接口（`PipelineProtocol`, `KernelInterface`, `LaunchConfig`）、host-device POD 与存储契约（`DeviceCopy`, `DeviceBuffer`, `SharedMem`）、`^attrs.compilation.intrinsic_enum` 与 `expr'intrinsic(...).as(T)` 开放输出协议、`target.{Scalar, Simd, Cuda, SpirV, Wgsl}` 指令枚举分层、受检内联汇编包装（`build.target` + `meta.compilation.err` + 内存契约）、以及 `^attrs.compilation.pipeline` + `meta.compilation.IIR.access(ir)` + `meta.runtime.invoke` 宿主后端交付闭环。
4. **3-Tier 严格反膨胀边界**：保持 Tier 1 六内置包轻量与编译期正交性，将完整 GPU 驱动、Vendor SDK 与高级张量运行时严格留在 Tier 2（`tensor_protocol.GpuProjection` 等）和 Tier 3（`cuda`, `vulkan_compute`, `cuda_compile`）。

### 1.2 权威依据与裁决顺位

依据 [AGENTS.md](file:///home/fin/luna/AGENTS.md) 确立如下权威优先级（**绝不引用任何 2026-05 之前的 `proposal_trace/` 历史草案**）：

1. **第一顺位（最高权威）**：[AGENTS.md](file:///home/fin/luna/AGENTS.md) 第 1–5 节协作与语义边界（取消独立 `anar` 保持单一 `luna` CLI、无 `Option`/`Result` 枚举、点分路径 `a.b.c`、无 comptime 指针、错误类型 `PascalCaseErr`/错误变体 `PascalCase`/普通变体 `snake_case`、仅允许 `^attrs.*` 封装 `__flurry_*`、禁用 `extern` 与 `handles`、`trait`/`struct`/`fn` 类型参数写入 `where` 子句且 trait 约束用 `:-`、无主体声明以 `;` 结尾、元组类型写 `Tuple<A, B>`）。
2. **第二顺位（现行正式规范）**：
   - 属性与配置：[attribute.md](file:///home/fin/luna/docs/comptime/attribute.md)、[official-attribute-keys.md](file:///home/fin/luna/docs/comptime/official-attribute-keys.md)、[builtin-packages.md](file:///home/fin/luna/docs/code-organization/builtin-packages.md)、[configuration-and-scripts.md](file:///home/fin/luna/docs/code-organization/configuration-and-scripts.md)、[the-comptime-phases-of-traits.md](file:///home/fin/luna/docs/comptime/the-comptime-phases-of-traits.md)。
   - 管线投影全套规范：[pipeline-projection/index.md](file:///home/fin/luna/docs/pipeline-projection/index.md)、[declarations-and-interfaces.md](file:///home/fin/luna/docs/pipeline-projection/declarations-and-interfaces.md)、[intrinsics.md](file:///home/fin/luna/docs/pipeline-projection/intrinsics.md)、[target-validation-and-assembly.md](file:///home/fin/luna/docs/pipeline-projection/target-validation-and-assembly.md)、[backends-and-artifacts.md](file:///home/fin/luna/docs/pipeline-projection/backends-and-artifacts.md)。
   - 跨文档对齐：[05-meta-cmtt-macros-reflection.md](file:///home/fin/luna/tmp/wishlist/05-meta-cmtt-macros-reflection.md)、[07-std-mem-collections-io-os.md](file:///home/fin/luna/tmp/wishlist/07-std-mem-collections-io-os.md)、[10-official-protocol-packages-and-boundary.md](file:///home/fin/luna/tmp/wishlist/10-official-protocol-packages-and-boundary.md)、[04-compiler-wishlist.md](file:///home/fin/luna/tmp/silver-design/04-compiler-wishlist.md)。
3. **第三顺位（近半年 `proposal_trace` 参考，2026-05 至 2026-10）**：[2026-8-26-pipeline-projection.fl](file:///home/fin/luna/proposal_trace/2026-8-26-pipeline-projection.fl)、[2026-8-24-intrinsic.fl](file:///home/fin/luna/proposal_trace/2026-8-24-intrinsic.fl)、[2026-9-7-inlining-asm.fl](file:///home/fin/luna/proposal_trace/2026-9-7-inlining-asm.fl)、[2026-5-22-heter-flurry.fl](file:///home/fin/luna/proposal_trace/2026-5-22-heter-flurry.fl)、[2026-7-14-global-variable-initialization-dependency.fl](file:///home/fin/luna/proposal_trace/2026-7-14-global-variable-initialization-dependency.fl)；与 `docs/` 或 `AGENTS.md` 冲突处（例如裸 `^intrinsic_enum`、小写 `.diagnostic(...)`、`.packed` 裸键）一律以 `docs/` 与 `AGENTS.md` 纠正。

### 1.3 六内置包与 3-Tier 反膨胀边界

| 层级 | 包 / 模块 | 允许纳入的职责 | 严格排除在外的内容（不得写入 Tier 1） |
|---|---|---|---|
| **Tier 1：编译器与系统基线** | `attrs` | 官方编译器消费者所需的 `__flurry_*` 键及其类型化 `^attrs.*` 构造器与校验（含布局、ABI/FFI、阶段、效应、内存、派生、稳定性、管线/编译提示、宏、模板匹配与元数据包装）。 | Tier 2 领域库元数据（如 `serialization`/`database` 字段标签禁止使用 `__flurry_*` 前缀，亦不入 `attrs`）。 |
| **Tier 1：编译器与系统基线** | `build` | 单一 `luna` CLI 驱动下的源码归属构建上下文（`mode`/`target`/`package_metadata`）、包清单与依赖描述、目标与产物规范、构建 DAG 任务与失效缓存、生成源码登记及带依赖登记的受控工具脚本封装。 | 独立可执行程序 `anar`、具体包管理器网络协议客户端（Git/Registry 下载器应属独立 CLI 驱动/插件层，不耦合语言核心类型检查）、具体 GPU SDK 检测硬编码。 |
| **Tier 1：投影与指令核心机制** | `builtin` / `meta` / `std` 交叉边界 | `^attrs.compilation.{pipeline, project, intrinsic_enum}`、`expr'intrinsic(...).as(T)` 核心语法与类型接口规则、`PipelineProtocol` / `DeviceCopy` 最小编译期可复制与布局投影基线、通用 IIR 句柄及宿主/SIMD 基础 `Scalar`/`Simd` intrinsic 包装接口。 | **严禁纳入**：完整 `cuda` / `vulkan` / `metal` / `webgpu` 运行时驱动、`cuBLAS`/`cuDNN` 包装、动态多维 `Tensor` 结构与自动微分磁带、第三方着色器编译库（这些归 Tier 2 [10 `tensor_protocol`](file:///home/fin/luna/tmp/wishlist/10-official-protocol-packages-and-boundary.md) 与 Tier 3 `cuda`/`cuda_compile`/`vulkan_rt`）。 |

---

## 2. 现状与缺口全景总表

基于对 [library/attrs](file:///home/fin/luna/library/attrs/lib.fl)、[library/build](file:///home/fin/luna/library/build/lib.fl)、[library/build/README.md](file:///home/fin/luna/library/build/README.md)、[issues/0003](file:///home/fin/luna/issues/0003-builtin-packages-compiler-blockers.md)、[issues/0004](file:///home/fin/luna/issues/0004-macro-system-carbon.md) 及相关文档的核查：当前 `attrs` 仅实现了 8 个零散叶模块，大量关键 `__flurry_*` 键缺失或尚未在 `attributes.rs` 中接入消费者；`build` 仅实现了 `info/{mode, target, package}` 三项只读快照，其余 5 个目录全为空占位；`pipeline-projection` 在代码库中尚无对应的标准库协议或 `IIR` 后端桥接实现。

| ID | 领域 | 需求项 / 符号 | 现状（已核实） | 核心缺口 | 优先级 | 目标归属（编译器 / 库） | 关联追踪 |
|---|---|---|---|---|---|---|---|
| `WL-06-01` | `attrs` 布局 | `^attrs.packed.{byte, bit}` 接入与旧 `packed` 清理 | `layout_impl/constructors.fl:10-14` 仍导出 `{ packed: true }` 与 `packed_layout`；编译器未接通 `__flurry_packed_mode` 完整消费。 | 删除旧裸键 `packed`/`packed_layout`，接通 NIR/LLVM 对 `__flurry_packed_mode` (`byte` / `bit` + `bitvec_opt`) 的消费。 | `P0` | `elaboration/attributes.rs`, `nir/layout.rs`, `library/attrs/layout_impl` | [issues/0003](file:///home/fin/luna/issues/0003-builtin-packages-compiler-blockers.md), `05 WL-05-16` |
| `WL-06-02` | `attrs` 布局 | **`^attrs.layout.align(bytes)`（`__flurry_align`，Silver P0-4）** | `official-attribute-keys.md` 与 `library/attrs/` 均未定义对齐属性。 | 在 `official-attribute-keys.md` 登记 `__flurry_align`；新增 `attrs.layout.align(bytes: usize)`，要求 2 的正整数次幂并注入类型/字段/栈变量对齐。 | `P0` | `library/attrs/layout_impl`, `nir/layout.rs`, `codegen/llvm` | [04-compiler-wishlist.md](file:///home/fin/luna/tmp/silver-design/04-compiler-wishlist.md) `P0-4`, `07 WL-07-08/20` |
| `WL-06-03` | `attrs` ABI/FFI | `^attrs.abi(...)` 扩展与 `^attrs.ffi.{link, link_name}` | `attrs.abi(.c)` 仅支持闭合标量/指针/`void`/`Never`；无 `link`/`link_name`（`07 WL-07-18`）。 | 登记 `__flurry_ffi_link`、`__flurry_ffi_link_name`；支持 `.system`/`.win64`/`.sysv64`/`.aapcs`、C 聚合传递、C fn 指针及无主体外部声明降码。 | `P0` | `library/attrs/{calling_convention,ffi_impl}`, `nir/control.rs`, `llvm` | `07 WL-07-18`, [effect-abi.md](file:///home/fin/luna/docs/algebraic-effect-and-delimited-continuation/effect-abi.md) |
| `WL-06-04` | `attrs` 阶段与效应 | `^attrs.comptime_phase` 与 `^attrs.effects.{direct_passing, ...}` | `staging/phase.fl` 与 `effects_impl/strategy.fl` 仅导出对象字面量，部分消费者缺失；无栈策略属性。 | 接通 `__flurry_comptime_phase` 与 `__flurry_direct_passing` 的完整 elaboration 检查；补齐 `__flurry_effect_strategy`（如有限栈内联/独立栈池提示）。 | `P1` | `library/attrs/{staging,effects_impl}`, `elaboration/traits`, `mir/effects` | [the-comptime-phases-of-traits.md](file:///home/fin/luna/docs/comptime/the-comptime-phases-of-traits.md) |
| `WL-06-05` | `attrs` 内存与稳定性 | `^attrs.mem.volatile`、`^attrs.stable_since(version: str)` | 构造器已存在（`volatile.fl`, `stability_impl/constructors.fl`），但缺少全面 NIR/LLVM 消费者。 | 强制 `__flurry_volatile` 对结构/字段/读写禁止消除与重排；将 `__flurry_stable_since` 接入 128-bit `type_id` Merkle 锚点计算。 | `P1` | `nir`, `llvm`, `library/attrs/{mem_impl,stability_impl}` | `05 WL-05-04`, `07 Zeroize` |
| `WL-06-06` | `attrs` 派生 | `^attrs.derivation.default`（`__flurry_derivation_default`） | `library/attrs/derivation/README.md` 指出 `default` 在 `lexical.md` 中为关键字，导致成员路径冲突无法导出。 | 裁决允许路径成员位置关键字逃逸或调整关键字规则后导出 `attrs.derivation.default`；打通 `__flurry_derivation_default` 默认派生物查找。 | `P1` | `compiler/parse`, `elaboration/derivation`, `library/attrs/derivation` | `library/attrs/derivation/README.md` |
| `WL-06-07` | `attrs` 宏与模板 | `^attrs.macros.{ast, peg, macro3, macro4}` 与 `^attrs.quote_fn_template.*` | `handlers.fl` 已使用 `.ast`/`.peg`/`.tokens`/`.source`，但 `attributes.rs` 未放行顶层 `__flurry_macro_*`（`issues/0004`）；`quote_fn_template/README.md` 为空占位，键未登记。 | 放行 `__flurry_macro_*` 并保持 `.tokens`/`.source` 枚举值接通 4 类宏；在 `official-attribute-keys.md` 与 `library/attrs` 正式登记并实现 `attrs.quote_fn_template.{for_match_arms, allow_non_exhaustive_instance}`。 | `P0` | `library/attrs/{macros_impl,quote_fn_template}`, `elaboration/attributes.rs`, `macro_expander` | [issues/0004](file:///home/fin/luna/issues/0004-macro-system-carbon.md), `05 WL-05-25/27` |
| `WL-06-08` | `attrs` 编译管线与提示 | `^attrs.compilation.{pipeline, project, intrinsic_enum, inline, cold, hot, target_feature, caller_location}` | `library/attrs/compilation/README.md` 为空占位；`pipeline`/`project`/`intrinsic_enum`/`align`/`caller_location` 全未登记。 | 登记并导出全套 `__flurry_*` 编译策略与代码生成提示属性（替代旧文档裸 `^intrinsic_enum`），打通内联控制、目标特性验证与调用点位置注入（`07 WL-07-17`）。 | `P0` | `library/attrs/compilation/`, `elaboration/attributes.rs`, `iir`, `llvm` | `05 OQ-5.9`, `07 WL-07-17`, [backends-and-artifacts.md](file:///home/fin/luna/docs/pipeline-projection/backends-and-artifacts.md) |
| `WL-06-09` | `attrs` 元数据 | `^attrs.metadata(data: Object)` 与领域属性防冲突约定 | `metadata_impl/constructors.fl` 直接覆盖 `{ __flurry_metadata: data }`，多属性叠加或混淆 Tier 2 metadata 风险高。 | 规范 `__flurry_metadata` 保留语义及多条目合并策略，明确隔离官方 `__flurry_*` 与 Tier 2 领域前缀键（`serialization_*` 等）。 | `P2` | `library/attrs/metadata_impl`, `elaboration/attributes.rs` | [official-attribute-keys.md](file:///home/fin/luna/docs/comptime/official-attribute-keys.md), `10 §5 Q4` |
| `WL-06-10` | `build.info` | `mode`、`target`、`package_metadata` 上下文完备化 | 已有 `mode.fl`、`target.fl`、`package.fl` 基础快照；缺乏 ABI/字节序/目标三元组与精确字段失效键。 | 保持源码归属与 `Target.pointer_bytes -> Integer` 契约；扩充 `Target`（`endianness`, `abi`, `triple`）与 `BuildContext` 只读审计，为精准字段级重编失效提供观察钩子。 | `P0` | `library/build/info/`, `compiler/build_context` | [library/build/README.md](file:///home/fin/luna/library/build/README.md) |
| `WL-06-11` | `build.package` | 包清单、依赖图与源码归属身份 | `library/build/package/README.md` 为空占位；驱动仅传裸 `--package` ID 与 hex 元数据。 | 设计 `PackageId`, `PackageManifest`, `DependencySpec`, `PackageSource`, `PackageGraph`，严格区分解耦语义包 ID 与解析分辨率 ID。 | `P1` | `library/build/package/`, `luna` CLI driver | [builtin-packages.md](file:///home/fin/luna/docs/code-organization/builtin-packages.md) |
| `WL-06-12` | `build.targets` | 构建目标、host/device 管线配置与产物类型 | `library/build/targets/README.md` 为空占位。 | 定义 `TargetSpec`, `PipelineKind`, `ArtifactKind`, `LinkConfig`，强制隔离 host pipeline target 与设备投影目标。 | `P1` | `library/build/targets/`, `luna` CLI driver | [declarations-and-interfaces.md](file:///home/fin/luna/docs/pipeline-projection/declarations-and-interfaces.md) |
| `WL-06-13` | `build.tasks` | **`luna` 单二进制构建 DAG、产物与缓存失效** | `library/build/tasks/README.md` 为空占位；现有缓存仅按全请求键保守失效（无任务 DAG 接口）。 | 设计 `TaskSpec`, `BuildDag`, `Artifact`, `CacheKey`, `InvalidationDep`（观测文件/env/feature 精确签名），全面服务单二进制 `luna` 构建调度。 | `P1` | `library/build/tasks/`, `luna` CLI driver | `AGENTS.md` 取消独立 anar, [configuration-and-scripts.md](file:///home/fin/luna/docs/code-organization/configuration-and-scripts.md) |
| `WL-06-14` | `build.generated` | 生成源码、配置模块与工具产物登记 | `library/build/generated/README.md` 为空占位。 | 提供 `GeneratedEntry`, `GeneratedKind`, `register_generated_source`, `register_generated_bytes`, `emit_config_module`，限制输出在工作区目标输出目录内并链接失效依赖。 | `P2` | `library/build/generated/`, `luna` CLI driver | `library/build/generated/README.md` |
| `WL-06-15` | `build.util` | 构建脚本沙盒辅助工具（路径/环境/工具链执行） | `library/build/util/README.md` 为空占位。 | 提供 `BuildPath`, `read_tracked_file`, `tracked_env_var`, `run_tracked_tool`，强制任何文件/env 读取自动注册失效边且默认隔离外部网络/写逃逸。 | `P2` | `library/build/util/`, `luna` sandbox | `library/build/util/README.md` |
| `WL-06-16` | 管线投影接口 | `PipelineProtocol`, `KernelInterface`, `LaunchConfig` 与 `ExtendedVariadicCollector` | 仓库完全空白；仅见于 `docs/pipeline-projection/declarations-and-interfaces.md`。 | 定义投影回调协议、基于 `F matches Kernel<A, Any, any>` 的类型联系与两阶段扩展变参收集不变量（`Tuple<Tmp, Self>`，普通实参只求值一次，不可直接按宿主 ABI 调用）。 | `P1` | `builtin.ops`, `builtin.pipeline`, `meta.compilation`, `elaboration/attributes.rs` | [declarations-and-interfaces.md](file:///home/fin/luna/docs/pipeline-projection/declarations-and-interfaces.md) |
| `WL-06-17` | Host-Device 存储 | `DeviceCopy`、`DeviceBuffer`、`SharedMem` 与边界契约 | 仓库完全空白。 | 定义设备可按位安全跨边界传递标记 trait `DeviceCopy`（要求无裸宿主指针与宿主 Drop 绑定）、类型化 `DeviceBuffer` 句柄协议与静态共享内存 `SharedMem`（runtime-dependent / value-clause 支持），同时将具体驱动留在 Tier 2/3。 | `P2` | Tier 1 核心投影协议接口 & Tier 2 `tensor_protocol` 对接 | `10 WL-10-18`, [2026-5-22-heter-flurry.fl](file:///home/fin/luna/proposal_trace/2026-5-22-heter-flurry.fl) |
| `WL-06-18` | Intrinsic 机制 | `^attrs.compilation.intrinsic_enum` 与 `expr'intrinsic(...).as(T)` | 仓库中仅 `std.os` 硬编码 11 个 `platform.ll` intrinsic；用户自定义 `intrinsic_enum`、`expr'intrinsic().as(T)` 语法与 IIR 节点均未接通。 | 接入通用 `Intrinsic` IIR 节点保留（标签身份 + 按序实参 + 显式输出类型或 `void`），强制 `unsafe` 要求且严禁优化器凭 `void` 消除带副作用指令。 | `P0` | `parse`, `elaboration/views`, `iir/intrinsics`, `mir` | [intrinsics.md](file:///home/fin/luna/docs/pipeline-projection/intrinsics.md), `07 WL-07-19` |
| `WL-06-19` | Target 指令分类 | `target.{Scalar, Simd, Cuda, SpirV, Wgsl}` 分类学与分层归属 | 无统一分层；`2026-8-24-intrinsic.fl` 与 `2026-9-7-inlining-asm.fl` 为独立原型。 | 明确宿主 `target.Scalar`/`target.Simd` 与外部投影设备指令簇 `target.{Cuda, SpirV, Wgsl}` 的分层边界与标准枚举命名形态（Tree Enum 命名空间）。 | `P1` | `builtin`/`std.simd`/`target.*` 与 Tier 3 设备后端 | [2026-8-24-intrinsic.fl](file:///home/fin/luna/proposal_trace/2026-8-24-intrinsic.fl), `07 WL-07-20` |
| `WL-06-20` | 内联汇编封装 | 受检内联汇编包装（`intrinsic` + `build.target` + 内存/目标验证） | 无通用内联汇编/目标验证通道。 | 规范无字符串 `asm!` 宏的安全封装范式：`inline if build.target.*` + `meta.compilation.err` + `unsafe` + 显式内存契约（由 `verification` 而非 PTC 负责）与后端降码不变量。 | `P1` | `iir`, `nir`, `llvm`, `std.asm` / 目标封装库 | [target-validation-and-assembly.md](file:///home/fin/luna/docs/pipeline-projection/target-validation-and-assembly.md), [2026-9-7-inlining-asm.fl](file:///home/fin/luna/proposal_trace/2026-9-7-inlining-asm.fl) |
| `WL-06-21` | 后端与产物交付 | `^attrs.compilation.pipeline` + `meta.compilation.IIR.access` + `meta.runtime.invoke` 产物清单 | `backends-and-artifacts.md` 与 `05 WL-05-29/32` 仅给出接口概念，无编译器后端插件加载/缓存协议。 | 闭环冻结 IIR 句柄传递、宿主动态/JIT 后端入口调用、多跨度 `Diagnostic` 回放与 `ArtifactManifest` 缓存（禁止进程内句柄或同名路径充当缓存命中依据）。 | `P1` | `iir`, `elaboration`, `comptime_host`, `luna` | [backends-and-artifacts.md](file:///home/fin/luna/docs/pipeline-projection/backends-and-artifacts.md), `05 OQ-5.9` |

---

## 3. 详细规格设计

### 3.1 `attrs` 包与统一 `__flurry_*` 官方属性键注册表

依据 [attribute.md](file:///home/fin/luna/docs/comptime/attribute.md) 与 [official-attribute-keys.md](file:///home/fin/luna/docs/comptime/official-attribute-keys.md)，所有影响编译器语义、布局、ABI、阶段、效应、派生、稳定性、宏或编译管线的属性**必须统一经由 `^attrs.*` 构造器导出，底层固定产出 `__flurry_*` 键**。任何裸键（如 `^packed`、`{ packed: true }`、`^intrinsic_enum`、`^inline`）以及旧版 `.id expr` 官方配置形式均判定为不合规实现或文档残留，必须淘汰。

#### 3.1.1 官方 `__flurry_*` 完整注册总表（含 `05`/`07`/文档跨项对齐）

| 公开 `attrs` 入口 | 底层 `__flurry_*` 键与值域 | 适用声明目标 | 当前登记与实现状态 | 编译器主责消费者 |
|---|---|---|---|---|
| `^attrs.packed.byte` | `__flurry_packed_mode: .byte` (`Symbol`) | `struct` | 已登记，缺完整 NIR/LLVM 消费（`issues/0003`） | `elaboration/attributes.rs`, `nir/layout.rs` |
| `^attrs.packed.bit` | `__flurry_packed_mode: .bit` (`Symbol`) | `struct`, `enum` | 已登记，需接通 `BitVec` 降码（`bitvec_opt`） | `elaboration/attributes.rs`, `nir/layout.rs` |
| `^attrs.layout.align(bytes)` | `__flurry_align: usize`（2 的正整数次幂） | `struct`, `enum`, `union`, 字段, `static`, 局部绑定 | **待登记**（`Silver P0-4`, `05 OQ-5.9`, `07 WL-07-08/20`） | `nir/layout.rs`, `llvm` align attr |
| `^attrs.abi(name)` | `__flurry_abi: Symbol`（`.c`, `.system`, `.win64`, `.sysv64`, `.aapcs`） | `fn`, 外部无主体 `fn`, `fn_type` | 已登记 `.c` 标量；缺其余约定、C 聚合与无主体导入（`07 WL-07-18`） | `nir/control.rs`, `llvm` callconv |
| `^attrs.ffi.link(.lib, .symbol, .kind)` | `__flurry_ffi_link: Object` | 无主体外部 `fn` / `static` 声明 | **待登记**（`07 WL-07-18` Q5） | `elaboration`, linker/artifact emitter |
| `^attrs.ffi.link_name(name)` | `__flurry_ffi_link_name: str` | `fn`, `static`（含导出/导入重命名） | **待登记**（`07 WL-07-18`） | `nir/symbols`, `llvm` linkage name |
| `^attrs.comptime_phase` | `__flurry_comptime_phase: bool` | `trait` 内方法 | 已登记，需校验与 `pure comptime trait` 一致性 | `elaboration/traits` |
| `^attrs.effects.direct_passing` | `__flurry_direct_passing: bool` | 零参 `effect` 操作/处理策略 | 已登记，需按参数形态拒绝带参操作（`10 §3.2`） | `mir/effects`, `effect_abi` |
| `^attrs.effects.stack_strategy(mode)` | `__flurry_effect_strategy: Symbol`（`.bounded_pool`, `.inline_frame`, `.heap_frame`） | `effect` handler / 调用点 | **待登记**（`Silver P0-3` / `effect-abi.md` 联动） | `mir/effects`, `StackPool` lowerer |
| `^attrs.mem.volatile` | `__flurry_volatile: bool` | `struct`, 字段, `static`, 指针操作 | 已登记，需全面禁用死存储消除与读写重排（`07 Zeroize`） | `nir/mem`, `llvm` volatile load/store |
| `^attrs.derivation.default` | `__flurry_derivation_default: bool` | `deriver` / `impl` 默认派生策略 | 已在 `official-attribute-keys` 登记，受 `lexical.md` `default` 关键字阻塞无法导出 | `parse`, `elaboration/derivation` |
| `^attrs.stable_since(version)` | `__flurry_stable_since: str`（严格 semver） | `struct`, `enum`, `newtype`, `fn`, 公开声明 | 已登记，缺与 `Type.type_id` Merkle 锚点联动（`05 WL-05-04`） | `nir/type_id.rs`, `elaboration/attributes.rs` |
| `^attrs.macros.ast(.variadic)` | `__flurry_macro_kind: .ast`, `__flurry_macro_variadic: bool` | `comptime fn` 宏处理函数 | 已部分定义于 `handlers.fl`，`attributes.rs` 未放行顶层键且 `meta.macros` 漏再导出 | `elaboration/attributes.rs`, `macro_expander` |
| `^attrs.macros.peg(grammar)` | `__flurry_macro_kind: .peg`, `__flurry_macro_grammar: str` | `comptime fn` PEG 宏函数 | 已部分定义于 `handlers.fl`，同上待顶层放行与接通 | `elaboration/attributes.rs`, `macro_expander` |
| `^attrs.macros.macro3` | `__flurry_macro_kind: .tokens` | `comptime fn(TokenBuffer) -> !MacroErr TokenBuffer` | 键已定义于 `handlers.fl:13`，缺编译器真展开通道（`05 WL-05-27`） | `macro_expander/src/registry.rs`, `macro_host.rs` |
| `^attrs.macros.macro4` | `__flurry_macro_kind: .source` | `comptime fn` 混合结构化输出宏 | 键已定义于 `handlers.fl:14`，缺编译器真展开通道（`05 WL-05-27`） | `macro_expander/src/registry.rs`, `macro_host.rs` |
| `^attrs.quote_fn_template.for_match_arms` | `__flurry_quote_fn_template_for_match_arms: bool` | `template quote fn` 匹配臂模板 | **待登记**（`docs/cmtt/quote-functions-and-templates.md:139`, `05 OQ-5.9`） | `elaboration/cmtt` |
| `^attrs.quote_fn_template.allow_non_exhaustive_instance` | `__flurry_quote_fn_template_allow_non_exhaustive_instance: bool` | `template quote fn` 非穷尽实例许可 | **待登记**（`docs/cmtt/quote-functions-and-templates.md:140`, `05 OQ-5.9`） | `elaboration/cmtt` |
| `^attrs.compilation.pipeline` | `__flurry_pipeline: bool` | `pure comptime fn(IIR.Func) -> void` 管线处理器 | **待登记**（`backends-and-artifacts.md`, `05 OQ-5.9`） | `elaboration/pipeline`, `iir` |
| `^attrs.compilation.project(handler)` | `__flurry_pipeline_projection: Any` | 投影声明（如 GPU kernel / 异构入口） | 仅文档内裸键 `^{ __flurry_pipeline_projection: ... }`，缺正式包装（`declarations-and-interfaces.md:160`） | `elaboration/pipeline`, `iir` |
| `^attrs.compilation.intrinsic_enum` | `__flurry_intrinsic_enum: bool` | `enum` 内联指令标签族 | **待登记**（替换文档裸 `^intrinsic_enum`，`intrinsics.md:66`） | `elaboration/views`, `iir/intrinsics` |
| `^attrs.compilation.inline` / `inline_mode` | `__flurry_inline: Symbol`（`.hint`, `.always`, `.never`） | `fn`, 方法 | **待登记**（`target-validation-and-assembly.md` 优化提示） | `mir`, `llvm` inline attr |
| `^attrs.compilation.cold` / `hot` | `__flurry_branch_hint: Symbol`（`.cold`, `.hot`） | `fn` | **待登记**（代码布局与分支预测提示） | `llvm` cold/hot fn attr |
| `^attrs.compilation.target_feature(features)` | `__flurry_target_feature: str` | `fn`（要求与目标支持配合校验） | **待登记**（`target-validation-and-assembly.md`, `std.simd`） | `elaboration`, `llvm` target-features |
| `^attrs.compilation.caller_location` | `__flurry_caller_location: bool` | `fn` / 隐式调用点参数注入 | **待登记**（`07 WL-07-17` `panic` 调用位置定位） | `elaboration/calls`, `diagnostics` |
| `^attrs.metadata(data)` | `__flurry_metadata: Object` | 任意声明 | 已存在，需规定叠加语义与禁止混充编译器键 | `elaboration/attributes.rs`, `meta.reflection` |

#### 3.1.2 `library/attrs/` 完整模块化接口签名

> **旧键清理声明**：必须从 [library/attrs/layout_impl/constructors.fl](file:///home/fin/luna/library/attrs/layout_impl/constructors.fl) 第 10–14 行彻底移除过渡裸键 `pub const packed = { packed: true }` 与 `packed_layout`，并同步清理 `library/meta/reflection/structure.fl:133` 的 `.packed` 回退。

```flurry
-- 包：attrs
use builtin.{Any, Symbol, bool, str, usize}
use meta.compilation;
use meta.data.Object;

-- 1. attrs.packed 与 attrs.layout
pub mod packed {
    pub const byte: Object = { __flurry_packed_mode: .byte }
    pub const bit: Object = { __flurry_packed_mode: .bit }
}

pub mod layout {
    pure comptime fn is_power_of_two(n: usize) -> bool {
        if n == 0usize {
            return false;
        }
        var v = n;
        while v > 1usize {
            if (v % 2usize) != 0usize {
                return false;
            }
            v = v / 2usize;
        }
        true
    }

    pub pure comptime fn align(bytes: usize) -> Object {
        inline if not is_power_of_two(bytes) {
            meta.compilation.err("attrs.layout.align requires a positive power-of-two byte alignment");
        }
        { __flurry_align: bytes }
    }
}

-- 2. attrs.abi 与 attrs.ffi（彻底替代 extern 关键字）
pub pure comptime fn abi(convention: Symbol) -> Object {
    inline if convention != .c
        and convention != .system
        and convention != .win64
        and convention != .sysv64
        and convention != .aapcs
    {
        meta.compilation.err("unsupported calling convention in attrs.abi");
    }
    { __flurry_abi: convention }
}

pub mod ffi {
    pub pure comptime fn link(
        .lib: ?str = null,
        .symbol: ?str = null,
        .kind: Symbol = .dylib,
    ) -> Object {
        {
            __flurry_ffi_link: {
                lib: lib,
                symbol: symbol,
                kind: kind,
            }
        }
    }

    pub pure comptime fn link_name(name: str) -> Object {
        { __flurry_ffi_link_name: name }
    }
}

-- 3. attrs.comptime_phase、attrs.effects、attrs.mem、attrs.stable_since
pub const comptime_phase: Object = { __flurry_comptime_phase: true }

pub mod effects {
    pub const direct_passing: Object = { __flurry_direct_passing: true }

    pub pure comptime fn stack_strategy(mode: Symbol) -> Object {
        { __flurry_effect_strategy: mode }
    }
}

pub mod mem {
    pub const volatile: Object = { __flurry_volatile: true }
}

pub pure comptime fn stable_since(version: str) -> Object {
    { __flurry_stable_since: version }
}

-- 4. attrs.derivation（依据 library/attrs/derivation/README.md 与 docs/basics/lexical.md:40-42）
pub mod derivation {
    -- 正式目标接口：pub const default: Object = { __flurry_derivation_default: true }
    -- 严禁以 mark_default 等变通命名替代；待成员定义位放行 default 关键字后直接启用（见第 5 节 OQ-6.1）
}

-- 5. attrs.compilation（管线、投影、指令枚举、codegen 提示与调用位置）
pub mod compilation {
    pub const pipeline: Object = { __flurry_pipeline: true }
    pub const intrinsic_enum: Object = { __flurry_intrinsic_enum: true }
    -- 注：成员名 `inline` 同样受成员定义位关键字检查约束（docs/basics/lexical.md:28，见第 5 节 OQ-6.1）
    pub const inline: Object = { __flurry_inline: .hint }
    pub const cold: Object = { __flurry_branch_hint: .cold }
    pub const hot: Object = { __flurry_branch_hint: .hot }
    pub const caller_location: Object = { __flurry_caller_location: true }

    pub pure comptime fn project(handler: Any) -> Object {
        { __flurry_pipeline_projection: handler }
    }

    pub pure comptime fn inline_mode(mode: Symbol) -> Object {
        { __flurry_inline: mode }
    }

    pub pure comptime fn target_feature(features: str) -> Object {
        { __flurry_target_feature: features }
    }
}

-- 6. attrs.macros、attrs.quote_fn_template 与 attrs.metadata
pub mod macros {
    pub pure comptime fn ast(.variadic: bool = false) -> Object {
        {
            __flurry_macro_kind: .ast,
            __flurry_macro_variadic: variadic,
        }
    }

    pub pure comptime fn peg(grammar: str) -> Object {
        {
            __flurry_macro_kind: .peg,
            __flurry_macro_grammar: grammar,
        }
    }

    -- 必须与 library/attrs/macros_impl/handlers.fl:13-14 及 compiler/luna/src/macro_host.rs:1476-1499 保持一致
    pub const macro3: Object = { __flurry_macro_kind: .tokens }
    pub const macro4: Object = { __flurry_macro_kind: .source }
}

pub mod quote_fn_template {
    pub const for_match_arms: Object = { __flurry_quote_fn_template_for_match_arms: true }
    pub const allow_non_exhaustive_instance: Object = {
        __flurry_quote_fn_template_allow_non_exhaustive_instance: true
    }
}

pub pure comptime fn metadata(data: Object) -> Object {
    { __flurry_metadata: data }
}

-- 7. library/attrs/lib.fl 根导出与默认 prelude（遵循 AGENTS.md 第 5 节，向除 attrs 自身外的包注入）
pub use .abi;
pub use .comptime_phase;
pub use .mem.volatile;
pub use .metadata;
pub use .stable_since;

pub mod prelude {
    pub use .abi;
    pub use .comptime_phase;
    pub use .metadata;
    pub use .stable_since;
}
```

#### 3.1.3 拒绝例

```flurry
-- [拒绝例 3.1-A] 禁止非 attrs 源码或 Tier 2 领域库构造 __flurry_* 官方属性键
-- const bad_domain_attr = { __flurry_my_domain_tag: true }  -- 规范与静态检查拒绝

-- [拒绝例 3.1-B] 禁止裸属性键绕过 attrs 封装或传递非 2 的幂对齐值
-- ^{ packed: true }         -- 拒绝：裸键 packed 已被废弃删除
-- ^intrinsic_enum           -- 拒绝：必须写成 ^attrs.compilation.intrinsic_enum
comptime {
    let _ = attrs.layout.align(6usize);  -- 编译期 Err：6 不是 2 的正整数次幂
}

-- [拒绝例 3.1-C] 带参 effect 操作严禁标注 direct_passing
effect bad_param_eff(payload: u64) -> void;
-- ^attrs.effects.direct_passing  -- 拒绝：direct_passing 仅允许零参操作
```

---

### 3.2 `build` 包全 6 子目录接口与单一 `luna` CLI 工程体系

依据 [AGENTS.md](file:///home/fin/luna/AGENTS.md)、[builtin-packages.md](file:///home/fin/luna/docs/code-organization/builtin-packages.md)、[configuration-and-scripts.md](file:///home/fin/luna/docs/code-organization/configuration-and-scripts.md) 与 [library/build/README.md](file:///home/fin/luna/library/build/README.md)：
1. **彻底取消独立 `anar` 计划**：所有包清单、依赖解析、目标配置、任务 DAG 调度、缓存失效与产物交付统一收口至单一 `luna` 二进制 CLI 与其流式协议；`library/build/` 给出编译器、CLI 驱动与包构建脚本之间的类型化契约。
2. **源码归属构建上下文不变量（Source-Owned Build Context）**：`build.mode`、`build.target` 与 `build.package_metadata` **始终按读取表达式所属源码定义的语义包身份求解**。跨包调用、导入别名、再导出、`const` 常量保留及 `template` 默认参数均不把上下文切换成调用者或编译器宿主机的值；缺失显式配置时直接报编译错误，绝不静默回退到进程环境变量或 `debug`。
3. **Host Pipeline Target ≠ Build Host ≠ Device Pipeline**：`build.target` 仅描述当前语义包正在构建的目标二进制环境（host pipeline target），投影到 GPU/CUDA/SPIR-V 时**绝不**把 `build.target` 改写为设备目标；设备端能力与目标架构必须显式通过 `build.package_metadata(alias)` 取得。

#### 3.2.1 `build.info`（延续并扩充 `library/build/info/{mode, target, package}.fl`）

```flurry
-- 集中绑定层：library/build/bindings.fl（仅此边界保存构建输入原语绑定）
-- pub const mode: Symbol = .build_mode'builtin;
-- pub const target: Target = .build_target'builtin;
-- pub const package_metadata: comptime fn(str) -> PackageMetadata = .build_package_metadata'builtin;

-- 包：build.info（library/build/info/{mode, target, package}.fl）
use bindings;
use builtin.{Symbol, Type, bool, str}
use meta.data.{Integer, List, Object}

pub enum Endianness {
    little_endian,
    big_endian,
}

pub struct TargetFeatures {
    entries: Object,
}

impl TargetFeatures {
    -- 未配置返回 null，显式关闭返回 false?，显式开启返回 true?
    pub pure comptime fn get(self, name: Symbol) -> ?bool;
    pub pure comptime fn is_enabled(self, name: Symbol) -> bool;
    pub pure comptime fn is_disabled(self, name: Symbol) -> bool;
    pub pure comptime fn keys(self) -> List<Symbol>;
}

pub struct Target {
    arch: Symbol,
    os: Symbol,
    abi: Symbol,
    triple: str,
    endianness: Endianness,
    pointer_bits: Integer,
    features: TargetFeatures,
}

impl Target {
    pub pure comptime fn is_64_bit(self) -> bool;
    pub pure comptime fn is_32_bit(self) -> bool;
    -- 保持与 library/build/info/target.fl:25 及 pointer_bits: Integer 一致
    pub pure comptime fn pointer_bytes(self) -> Integer;
    pub pure comptime fn is_arch(self, expected: Symbol) -> bool;
    pub pure comptime fn is_os(self, expected: Symbol) -> bool;
    pub pure comptime fn has_feature(self, name: Symbol) -> bool;
}

pub struct MetadataFeatures {
    entries: Object,
}

impl MetadataFeatures {
    -- 按名查询原始类型值；缺失或类型域不匹配返回 null，保留重复键“后覆盖前”但类型不符不回退语义
    pub pure comptime fn get(self, name: Symbol, comptime value_type: Type) -> ?value_type;
    pub pure comptime fn has(self, name: Symbol) -> bool;
    pub pure comptime fn keys(self) -> List<Symbol>;
}

pub struct PackageMetadata {
    package_alias: str,
    features: MetadataFeatures,
}

-- 保持 library/build/info/{mode, target, package}.fl 直接再导出 bindings 绑定，保留读取点源码包身份
pub use bindings.{mode, package_metadata, target}

-- 根导出：library/build/lib.fl（build 不自动向其他包注入 prelude）
pub use .info.{
    Endianness,
    MetadataFeatures,
    PackageMetadata,
    Target,
    TargetFeatures,
    mode,
    package_metadata,
    target,
}
pub use .package;
pub use .targets;
pub use .tasks;
pub use .generated;
pub use .util;
pub mod prelude {}
```

#### 3.2.2 `build.package`（包清单、依赖别名与语义包身份）

```flurry
-- 包：build.package
use builtin.{Symbol, Tuple, bool, str}
use meta.data.List;

pub newtype PackageId = str;

pub enum PackageKind {
    binary_app,
    library_pkg,
    macro_pkg,
    pipeline_backend_pkg,
    builtin_sysroot_pkg,
}

pub enum PackageSource {
    workspace_member(str),
    local_path(str),
    sysroot_builtin(Symbol),
    locked_dependency {
        locator: str,
        content_digest: str,
    },
}

pub struct DependencySpec {
    alias: str,
    package_id: PackageId,
    required_features: List<Symbol>,
    target_override_mode: ?Symbol,
}

pub struct PackageManifest {
    id: PackageId,
    name: Symbol,
    version: str,
    kind: PackageKind,
    source: PackageSource,
    root_dir: str,
    entry_file: str,
    dependencies: List<DependencySpec>,
}

pub struct PackageGraph {
    root_package: PackageId,
    packages: List<PackageManifest>,
    edges: List<Tuple<PackageId, DependencySpec>>,
}

impl PackageGraph {
    pub pure comptime fn find_package(self, id: PackageId) -> ?PackageManifest;
    pub pure comptime fn direct_dependencies(self, id: PackageId) -> List<DependencySpec>;
}
```

#### 3.2.3 `build.targets`（构建目标、管线与产物分类）

```flurry
-- 包：build.targets
use build.info.{Endianness, Target}
use builtin.{Symbol, bool, str, usize}
use meta.data.List;

pub enum PipelineKind {
    host_native,
    cuda_device,
    spirv_shader,
    wgsl_shader,
    custom_pipeline(Symbol),
}

pub enum ArtifactKind {
    executable,
    static_lib,
    shared_lib,
    object_file,
    ir_bitcode,
    ptx_kernel,
    spirv_module,
    wgsl_source,
    generated_source,
    metadata_bundle,
}

pub enum LinkMode {
    static_link,
    dynamic_link,
}

pub struct LinkConfig {
    mode: LinkMode,
    search_paths: List<str>,
    libraries: List<str>,
    exported_symbols: List<Symbol>,
}

pub struct TargetSpec {
    id: Symbol,
    pipeline: PipelineKind,
    target_info: Target,
    artifact_kind: ArtifactKind,
    link_config: LinkConfig,
    is_default_host: bool,
}

impl TargetSpec {
    pub pure comptime fn native_host(id: Symbol, target_info: Target, artifact_kind: ArtifactKind) -> TargetSpec;
    pub pure comptime fn device_projection(
        id: Symbol,
        pipeline: PipelineKind,
        artifact_kind: ArtifactKind,
        endianness: Endianness,
        pointer_bytes: usize,
    ) -> TargetSpec;
}
```

#### 3.2.4 `build.tasks`（`luna` 单二进制 DAG、缓存键与失效契约）

```flurry
-- 包：build.tasks
use build.package.PackageId;
use build.targets.{ArtifactKind, TargetSpec}
use builtin.{Symbol, bool, str}
use meta.data.{Integer, List}

pub enum BuildTaskErr {
    CyclicDependency,
    MissingInput,
    TaskFailed(str),
    InvalidArtifactPath,
    CacheKeyMismatch,
    SandboxViolation,
    UnknownPackageId,
}

pub newtype TaskId = str;

pub enum TaskStatus {
    pending,
    cache_hit,
    executed_ok,
    failed,
}

-- 精确重编失效依赖（绝不复用进程句柄或未校验路径假设）
pub enum InvalidationDep {
    file_content_hash {
        path: str,
        sha256_hex: str,
    },
    env_var_value {
        name: str,
        observed: ?str,
    },
    target_feature_flag {
        package_id: PackageId,
        feature: Symbol,
        observed: ?bool,
    },
    package_mode_flag {
        package_id: PackageId,
        mode: Symbol,
    },
    tool_version_stamp {
        tool_name: str,
        stamp: str,
    },
}

pub struct CacheKey {
    digest_hex: str,
    request_fingerprint: str,
    deps: List<InvalidationDep>,
}

pub struct Artifact {
    name: str,
    kind: ArtifactKind,
    output_path: str,
    content_hash_hex: str,
    size_bytes: Integer,
}

pub struct TaskSpec {
    id: TaskId,
    owner_package: PackageId,
    target: TargetSpec,
    depends_on: List<TaskId>,
    declared_inputs: List<str>,
    expected_outputs: List<ArtifactKind>,
}

pub struct TaskResult {
    id: TaskId,
    status: TaskStatus,
    cache_key: CacheKey,
    artifacts: List<Artifact>,
    diagnostics: List<str>,
}

pub struct BuildDag {
    tasks: List<TaskSpec>,
}

-- 所有修改构建图的方法在编译期采用 self 值引用语义（无 *self）
impl BuildDag {
    pub pure comptime fn empty() -> BuildDag;
    pub comptime fn add_task(self, spec: TaskSpec) -> !BuildTaskErr void;
    pub comptime fn add_edge(self, downstream: TaskId, upstream: TaskId) -> !BuildTaskErr void;
    pub pure comptime fn topological_order(self) -> !BuildTaskErr List<TaskId>;
    pub comptime fn execute_with_luna(self) -> !BuildTaskErr List<TaskResult>;
}
```

#### 3.2.5 `build.generated` 与 `build.util`（沙盒登记与受追踪构建工具）

```flurry
-- 包：build.generated
use build.package.PackageId;
use build.tasks.{BuildTaskErr, InvalidationDep}
use builtin.{Symbol, bool, str, u8}
use meta.ast.SourceSpan;
use meta.data.List;

pub enum GeneratedKind {
    flurry_source,
    config_module,
    binary_asset,
    linker_script,
}

pub struct GeneratedEntry {
    logical_module_path: List<Symbol>,
    relative_out_path: str,
    kind: GeneratedKind,
    owner_package: PackageId,
    origin_span: SourceSpan,
    deps: List<InvalidationDep>,
}

-- 将生成文件登记至包隔离的 build out_dir，禁止向源码树外任意路径写入
pub comptime fn register_generated_source(
    logical_module: List<Symbol>,
    relative_path: str,
    content: str,
    origin: SourceSpan,
) -> !BuildTaskErr GeneratedEntry;

pub comptime fn register_generated_bytes(
    relative_path: str,
    bytes: List<u8>,
    kind: GeneratedKind,
    origin: SourceSpan,
) -> !BuildTaskErr GeneratedEntry;

pub comptime fn emit_config_module(
    module_name: Symbol,
    flurry_source: str,
    origin: SourceSpan,
) -> !BuildTaskErr GeneratedEntry;
```

```flurry
-- 包：build.util
use build.tasks.{BuildTaskErr, InvalidationDep}
use builtin.{Symbol, bool, i32, str, u8}
use meta.data.List;

pub newtype BuildPath = str;

pub struct ToolOutput {
    exit_code: i32,
    stdout_text: str,
    stderr_text: str,
    recorded_deps: List<InvalidationDep>,
}

-- 所有 I/O 与环境读取强制向当前构建任务登记精确 InvalidationDep；禁止隐式读宿主环境
pub pure comptime fn out_dir() -> BuildPath;
pub pure comptime fn package_manifest_dir() -> BuildPath;
pub comptime fn read_tracked_file(relative_path: BuildPath) -> !BuildTaskErr str;
pub comptime fn read_tracked_bytes(relative_path: BuildPath) -> !BuildTaskErr List<u8>;
pub comptime fn tracked_env_var(name: str) -> ?str;
pub comptime fn file_exists(relative_path: BuildPath) -> !BuildTaskErr bool;
pub comptime fn run_tracked_tool(
    tool_name: Symbol,
    args: List<str>,
    declared_reads: List<BuildPath>,
    declared_writes: List<BuildPath>,
) -> !BuildTaskErr ToolOutput;
```

#### 3.2.6 `build` 拒绝例

```flurry
-- [拒绝例 3.2-A] 禁止认为 GPU 投影会隐式改变 build.target
-- 若源码位于宿主主包下，即使带投影属性，build.target 仍代表宿主平台（非 device）：
comptime fn wrong_target_assumption() -> bool {
    -- 错误意图：在依赖包未配置 cuda metadata 时直接假定 build.target.arch == .nvptx64
    build.info.target.arch == .x86_64  -- 仅反映所属语义包的 host target 配置
}

-- [拒绝例 3.2-B] 禁止向包生成输出目录之外写入生成文件
comptime fn illegal_gen_escape() -> !build.tasks.BuildTaskErr build.generated.GeneratedEntry {
    build.generated.register_generated_bytes(
        "../../outside_workspace.rs",
        [],
        .binary_asset,
        meta.ast.SourceSpan.synthetic(),
    )  -- 拒绝：SandboxViolation / InvalidArtifactPath
}
```

---

### 3.3 `pipeline-projection` 完整协议栈

依据 [pipeline-projection/index.md](file:///home/fin/luna/docs/pipeline-projection/index.md)、[declarations-and-interfaces.md](file:///home/fin/luna/docs/pipeline-projection/declarations-and-interfaces.md)、[intrinsics.md](file:///home/fin/luna/docs/pipeline-projection/intrinsics.md)、[target-validation-and-assembly.md](file:///home/fin/luna/docs/pipeline-projection/target-validation-and-assembly.md) 与 [backends-and-artifacts.md](file:///home/fin/luna/docs/pipeline-projection/backends-and-artifacts.md)，Flurry 将“声明的类型与调用接口”和“目标指令生成管线”正交拆离：任何硬件后端（CUDA、SPIR-V、WGSL、定制加速卡）都以库协议和编译期 IIR 投影接入，无需修改编译器前端语法。

#### 3.3.1 投影接口、`PipelineProtocol` 与 `LaunchConfig`（Tier 1 核心抽象）

```flurry
-- 集中绑定层：library/builtin/bindings.fl（仅此边界可直写 .symbol'builtin；外部库如 cuda 亦可自定义库级 Kernel 头）
-- pub const Kernel: pure comptime fn<Args: Type, Env: Type, Tag: Any> -> Type = .Kernel'builtin;

-- 归属：builtin.ops（library/builtin/ops.fl，与 VariadicCollector 对称）
use attrs;
use builtin.{Tuple, Type}

pub trait ExtendedVariadicCollector {
    assoc Tmp: Type;
    fn extended_variadic_collect(comptime Ps: Type, properties: Ps) -> Tuple<Tmp, Self>;

    ^attrs.comptime_phase
    assoc TmpComptime: Type;

    ^attrs.comptime_phase
    fn extended_variadic_collect_comptime(Ps: Type, properties: Ps) -> Tuple<TmpComptime, Self>;
}
```

```flurry
-- 归属：meta.compilation（library/meta/compilation/pipeline.fl，编译期投影接口校验与 IIR 编译契约；避免 builtin -> build 反向依赖）
use build.info.PackageMetadata;
use build.tasks.{Artifact, InvalidationDep}
use builtin.{Symbol, Type, str, usize}
use meta.compilation.IIR;
use meta.data.{List, Object}
use meta.reflection.FunctionInfo;

pub enum PipelineErr {
    UnsupportedTarget,
    InvalidKernelSignature,
    MissingDeviceMetadata,
    BackendCompilationFailed(str),
    IncompatibleLaunchConfig,
    NonDeviceCopyPayload,
    DirectHostCallForbidden,
}

-- 声明级接口描述符：宿主侧类型检查仅依赖签名与必选元数据，不等待完整后端代码生成
pub struct KernelInterface {
    entry_symbol: Symbol,
    arg_types: List<Type>,
    required_shared_mem_bytes: usize,
    recorded_deps: List<InvalidationDep>,
}

-- 供 ^attrs.compilation.project(handler) 消费的编译期投影协议 trait
pub pure comptime trait PipelineProtocol {
    pub pure comptime fn verify_interface(
        signature: FunctionInfo,
        metadata: Object,
        pkg_cfg: PackageMetadata,
    ) -> !PipelineErr KernelInterface;

    pub comptime fn compile_projection(
        ir: IIR.Func,
        interface: KernelInterface,
        pkg_cfg: PackageMetadata,
    ) -> !PipelineErr Artifact;
}
```

```flurry
-- 归属：builtin.pipeline（library/builtin/pipeline.fl，Tier 1 零循环依赖基础类型接口；具体驱动归 Tier 2 tensor_protocol / Tier 3 cuda）
use attrs;
use bindings;
use builtin.{Any, Tuple, Type, u32, void}
use builtin.ops.ExtendedVariadicCollector;

pub use bindings.Kernel;

-- 通用启动配置契约骨架（与 docs/pipeline-projection/declarations-and-interfaces.md:87-103 对齐）
pub struct LaunchConfig
where
    F: Type,
    A: Type,
    requires F matches Kernel<A, Any, any>
{
    kernel: *F,
    grid_dim: Tuple<u32, u32, u32>,
    block_dim: Tuple<u32, u32, u32>,
    args: A,
    shared_mem_bytes: u32 = 0u32,
}

-- 为 LaunchConfig<F, A> 实现两阶段 ExtendedVariadicCollector 骨架（与 OQ-6.6 对齐）
impl ExtendedVariadicCollector for LaunchConfig<F, A>
where
    F: Type,
    A: Type,
    requires F matches Kernel<A, Any, any>
{
    assoc Tmp: Type = void;

    fn extended_variadic_collect(comptime Ps: Type, properties: Ps) -> Tuple<void, Self>
    where
        requires Type.predicates.is_anonymous_record(Ps),
        requires Type.predicates.has_field(Ps, .kernel),
        requires Type.predicates.has_field(Ps, .grid_dim),
        requires Type.predicates.has_field(Ps, .block_dim),
        requires Type.predicates.has_field(Ps, .args)
    {
        let smem = inline if Type.predicates.has_field(Ps, .shared_mem_bytes) {
            properties.shared_mem_bytes
        } else {
            0u32
        }
        ((), Self {
            kernel: properties.kernel,
            grid_dim: properties.grid_dim,
            block_dim: properties.block_dim,
            args: properties.args,
            shared_mem_bytes: smem,
        })
    }

    ^attrs.comptime_phase
    assoc TmpComptime: Type = void;

    ^attrs.comptime_phase
    fn extended_variadic_collect_comptime(Ps: Type, properties: Ps) -> Tuple<void, Self>
    where
        requires Type.predicates.is_anonymous_record(Ps),
        requires Type.predicates.has_field(Ps, .kernel),
        requires Type.predicates.has_field(Ps, .grid_dim),
        requires Type.predicates.has_field(Ps, .block_dim),
        requires Type.predicates.has_field(Ps, .args)
    {
        let smem = inline if Type.predicates.has_field(Ps, .shared_mem_bytes) {
            properties.shared_mem_bytes
        } else {
            0u32
        }
        ((), Self {
            kernel: properties.kernel,
            grid_dim: properties.grid_dim,
            block_dim: properties.block_dim,
            args: properties.args,
            shared_mem_bytes: smem,
        })
    }
}

pub fn take_launch_arguments(config: LaunchConfig<F, A>) -> A
where
    F: Type,
    A: Type,
    requires F matches Kernel<A, Any, any>
{
    config.args
}
```

#### 3.3.2 Host-Device 数据边界：`DeviceCopy`、`DeviceBuffer` 与 `SharedMem`

依据 [declarations-and-interfaces.md](file:///home/fin/luna/docs/pipeline-projection/declarations-and-interfaces.md) 与 [atoms.md](file:///home/fin/luna/docs/type/propositional-constraints/atoms.md)，“按位可安全跨宿主-设备边界传输”必须由明确接口 `DeviceCopy` 授权，**绝不能仅凭 `Type.size_of(T)` 已知或不含填充就默认推断允许设备传输**；完整运行时 GPU 存储与张量算子则在 Tier 2 [tensor_protocol](file:///home/fin/luna/tmp/wishlist/10-official-protocol-packages-and-boundary.md) 与 Tier 3 `cuda` 包中基于该基线展开：

```flurry
-- 归属：builtin.pipeline（library/builtin/pipeline.fl）
use builtin.{Type, usize}

-- 标记允许按 POD 位模式在宿主与异构设备间复制的类型（禁止含未转换宿主裸指针或宿主 Drop 义务）
pub trait DeviceCopy {}

-- 设备缓冲非自有轻句柄/视图协议：通过位置子句 quote device: lift *D 静态区分不同设备实例上的缓冲而不增加存储开销；
-- 实际设备内存所有权、分配与 Drop 释放契约归属 Tier 2 tensor_protocol.TensorStorage 与 Tier 3 设备库（见 10 WL-10-18）
pub struct DeviceBuffer
where
    T :- DeviceCopy,
    D: Type,
    quote device: lift *D
{
    device_ptr_raw: usize,
    len_elems: usize,
}

impl DeviceBuffer<T, D, device>
where
    T :- DeviceCopy,
    D: Type,
    quote device: lift *D
{
    pub pure fn len(*self) -> usize {
        self.len_elems.*
    }

    pub unsafe fn raw_device_addr(*self) -> usize {
        self.device_ptr_raw.*
    }

    pub fn device_handle(*self) -> *D {
        device
    }
}

-- 线程块/工作组片上共享内存描述符（仅允许在投影设备管线主体内静态分配并经 unsafe 读写）
pub struct SharedMem
where
    T :- DeviceCopy,
    count: usize
{
    private_tag: usize,
}

impl SharedMem<T, count>
where
    T :- DeviceCopy,
    count: usize
{
    pub unsafe fn as_mut_ptr(*self) -> *T;
}
```

#### 3.3.3 `intrinsic` 枚举分类学与 `.as(T)` 开放输出契约

依据 [intrinsics.md](file:///home/fin/luna/docs/pipeline-projection/intrinsics.md) 与 [2026-8-24-intrinsic.fl](file:///home/fin/luna/proposal_trace/2026-8-24-intrinsic.fl)：
1. **属性标准化**：一律使用 `^attrs.compilation.intrinsic_enum`（替代旧文档中的裸 `^intrinsic_enum`）。不同枚举内的同名变体拥有独立操作身份，绝不按名字字符串混同。
2. **输出类型硬规则**：
   - `Op'intrinsic(args...).as(T)` 向目标管线请求交付类型为 `T` 的返回值；外围 `let x: T = ...` 或函数返回类型**绝不能替代紧邻调用的 `.as(T)`**。
   - 不带 `.as(T)` 的 `Op'intrinsic(args...);` 类型固定为 `void`。
   - 返回 `void` 的屏障/存储操作仍可具备不可观察或硬件同步副作用，通用优化器禁止仅因返回 `void` 或处于 `unsafe` 中而删除、复制或跨栅栏重排。
3. **跨层归属分类学**：

| 命名空间 / 族 | 归属层级 | 代表性指令变体 | 消费管线与依赖要求 |
|---|---|---|---|
| `target.Scalar` | Tier 1（`builtin`/`std` 目标支持库） | `clz`, `ctz`, `popcnt`, `bswap`, `fadd_strict`, `fence` | LLVM 通用标量 / `builtin.math` 底层支撑。 |
| `target.Simd` | Tier 1（`std.simd` 底层目标族） | `vec_load`, `vec_store`, `fma_vec`, `shuffle`, `mask_reduce` | 与 `build.info.target.features`（`.avx2`, `.neon`, `.sve`）及 `^attrs.compilation.target_feature` 联动。 |
| `target.Cuda` | Tier 2/3（`cuda` 生态包） | `thread_idx_x`, `block_idx_x`, `block_dim_x`, `syncthreads`, `wmma.m16n16k16.mma` | 仅供 `^attrs.compilation.project(cuda.pipelines.*)` 内部使用，Tier 1 不耦合。 |
| `target.SpirV` | Tier 2/3（`spirv` / `vulkan` 生态包） | `global_invocation_id`, `workgroup_barrier`, `subgroup_add`, `image_sample` | 供 SPIR-V 投影管线验证与降码。 |
| `target.Wgsl` | Tier 2/3（`wgpu` / `webgpu` 生态包） | `workgroup_barrier`, `storage_barrier`, `atomic_add_u32` | 供 WGSL 着色器管线降码与 WebGPU 兼容性验证。 |

```flurry
use attrs;
use builtin.{Tuple, f32, u16, u32, u64, void}

-- 示例：基于 Flurry Tree Enum 组织多层级指令标签族
^attrs.compilation.intrinsic_enum
pub enum ScalarTargetInst {
    fma,
    barrier,
    split_u64,
}

pub unsafe fn scalar_fma(a: f32, b: f32, c: f32) -> f32 {
    ScalarTargetInst.fma'intrinsic(a, b, c).as(f32)
}

pub unsafe fn hardware_barrier(scope: u16) -> void {
    ScalarTargetInst.barrier'intrinsic(scope);
}

pub unsafe fn split_word_to_pair(value: u64) -> Tuple<u32, u32> {
    ScalarTargetInst.split_u64'intrinsic(value).as(Tuple<u32, u32>)
}
```

#### 3.3.4 受检内联汇编包装与目标验证（无字符串 `asm!` 宏）

依据 [target-validation-and-assembly.md](file:///home/fin/luna/docs/pipeline-projection/target-validation-and-assembly.md) 与 [2026-9-7-inlining-asm.fl](file:///home/fin/luna/proposal_trace/2026-9-7-inlining-asm.fl)，Flurry 以 **`^attrs.compilation.intrinsic_enum` + 编译期 `build.info.target` 条件守卫 + `^attrs.compilation.target_feature`** 表达内联指令封装。需要读写内存操作数校验时，读取/写入权限由 **`verification` 层**（或 `unsafe` 前提契约）而非 PTC 承担：

```flurry
use attrs;
use build.info.target;
use builtin.{Array, f32}
use meta.compilation;

^attrs.compilation.intrinsic_enum
pub enum X86Inst {
    fma_f32,
    avx2.{
        loadu_ps,
        add_ps,
        storeu_ps,
    },
}

^attrs.compilation.inline_mode(.always)
^attrs.compilation.target_feature("+fma")
pub unsafe fn x86_fma_f32(a: f32, b: f32, c: f32) -> f32 {
    inline if target.arch != .x86_64 {
        meta.compilation.err("x86_fma_f32 requires x86_64 target architecture");
    }
    X86Inst.fma_f32'intrinsic(a, b, c).as(f32)
}

^attrs.compilation.target_feature("+avx2")
pub unsafe fn add_arrays_avx2(
    xs: *Array<f32, 8usize>,
    ys: *Array<f32, 8usize>,
    zs: *Array<f32, 8usize>,
) {
    inline if target.arch != .x86_64 or not target.features.is_enabled(.avx2) {
        meta.compilation.err("add_arrays_avx2 requires x86_64 target with +avx2 feature enabled");
    }
    let vx = X86Inst.avx2.loadu_ps'intrinsic(xs).as(Array<f32, 8usize>);
    let vy = X86Inst.avx2.loadu_ps'intrinsic(ys).as(Array<f32, 8usize>);
    let vz = X86Inst.avx2.add_ps'intrinsic(vx, vy).as(Array<f32, 8usize>);
    X86Inst.avx2.storeu_ps'intrinsic(zs, vz);
}
```

#### 3.3.5 宿主后端交付闭环（IIR 句柄与 `meta.runtime.invoke`）

依据 [backends-and-artifacts.md](file:///home/fin/luna/docs/pipeline-projection/backends-and-artifacts.md) 与 [2026-8-26-pipeline-projection.fl](file:///home/fin/luna/proposal_trace/2026-8-26-pipeline-projection.fl)，编译期仅充当轻量胶水层，重型代码生成与目标校验交由独立的宿主侧 runtime 包执行；错误通过 `HostInvokeErr.Diagnostic(diag)` 经 `meta.compilation.emit_diagnostic` 返回编译器诊断系统：

```flurry
use attrs;
use builtin.void;
use meta.compilation;
use meta.runtime.{HostInvokeErr, invoke}

-- 说明：cuda / cuda_compile 属于 Tier 3 生态后端包（位于六内置包之外），此处仅示范调用 ^attrs.compilation.pipeline 协议
use cuda_compile;

^attrs.compilation.pipeline
pub pure comptime fn generic_device_pipeline(ir: meta.compilation.IIR.Func) -> void {
    let token = meta.compilation.IIR.access(ir) match {
        tok! => tok,
        error _ => {
            meta.compilation.err("failed to access frozen IIR handle for pipeline projection");
        },
    }
    invoke(cuda_compile.compile_kernel, token) match {
        _! => {},
        error HostInvokeErr.Diagnostic(diag) => meta.compilation.emit_diagnostic(diag),
        error _ => meta.compilation.err("pipeline backend execution failed"),
    }
}
```

#### 3.3.6 `pipeline-projection` 拒绝例

```flurry
-- [拒绝例 3.3-A] 投影设备 kernel 严禁当作普通宿主函数直接调用
-- ^attrs.compilation.project(generic_device_pipeline)
-- unsafe fn device_add(a: *f32, b: *f32, out: *f32) { out.* = a.* + b.*; }
-- unsafe fn bad_host_call(a: *f32, b: *f32, out: *f32) {
--     device_add(a, b, out);  -- 拒绝：DirectHostCallForbidden（必须经库启动协议）
-- }

-- [拒绝例 3.3-B] intrinsic 缺失紧邻的 .as(T) 时返回 void，左侧类型注解不能事后补写
unsafe fn missing_explicit_as(a: f32, b: f32, c: f32) -> f32 {
    -- let res: f32 = ScalarTargetInst.fma'intrinsic(a, b, c);  -- 拒绝：得到 void，与 f32 不兼容
    ScalarTargetInst.fma'intrinsic(a, b, c).as(f32)
}

-- [拒绝例 3.3-C] 非 unsafe 上下文禁止直接调用 'intrinsic
-- fn safe_caller_rejected(a: f32, b: f32, c: f32) -> f32 {
--     ScalarTargetInst.fma'intrinsic(a, b, c).as(f32)  -- 拒绝：intrinsic 调用必须处于 unsafe 块/函数内
-- }
```

---

## 4. 排期优先级与分阶段交付路线

下表将 `WL-06-01`..`WL-06-21` 划分为 4 个纵向交付阶段（`P0`–`P3`），并与 [05-meta-cmtt-macros-reflection.md](file:///home/fin/luna/tmp/wishlist/05-meta-cmtt-macros-reflection.md)、[07-std-mem-collections-io-os.md](file:///home/fin/luna/tmp/wishlist/07-std-mem-collections-io-os.md)、[10-official-protocol-packages-and-boundary.md](file:///home/fin/luna/tmp/wishlist/10-official-protocol-packages-and-boundary.md)、[issues/0003](file:///home/fin/luna/issues/0003-builtin-packages-compiler-blockers.md) 及 [issues/0004](file:///home/fin/luna/issues/0004-macro-system-carbon.md) 协同对齐：

| 阶段 | 包含 Wishlist ID | 核心交付内容与编译器闭环 | 解锁下游与跨包依赖 | 阶段验收门禁 |
|---|---|---|---|---|
| **Phase P0（官方属性规范化、ABI/FFI/对齐地基与 Intrinsic 核心）** | `WL-06-01`, `WL-06-02`, `WL-06-03`, `WL-06-07`, `WL-06-08`, `WL-06-10`, `WL-06-18` | 1. 更新 `official-attribute-keys.md` 与 `library/attrs/`，正式登记 `__flurry_align`（**Silver P0-4**）、`__flurry_ffi_link`/`__flurry_ffi_link_name`、`__flurry_quote_fn_template_*`、`__flurry_pipeline`/`__flurry_pipeline_projection`/`__flurry_intrinsic_enum`、`__flurry_inline`/`__flurry_branch_hint`/`__flurry_target_feature`/`__flurry_caller_location`；2. 清除 `layout_impl` 旧裸 `packed` 回退；3. 放行顶层 `__flurry_macro_*`（`issues/0004`）与扩展 `^attrs.abi(...)` C 导入导出；4. 保持 `build.info` 源码归属快照；5. 接入 `expr'intrinsic(...).as(T)` 核心语义与 `unsafe`/副作用不变量。 | `05 WL-05-04/16/25/27/29`, `07 WL-07-08/17/18/20`（`SoA`/`Simd`/`panic`/`std.ffi`）, `issues/0003`, `issues/0004`。 | 真实 sysroot + `luna` 端到端测试：跨包 `build.mode`/`target` 隔离、非 2 的幂 `align` 拒绝、`^attrs.macros.*` 四类宏注册、`intrinsic` 缺失 `.as(T)` 拒绝与 `void` 屏障保留测试。 |
| **Phase P1（`build` 核心工程图、阶段/效应/稳定性属性、投影接口与内联汇编）** | `WL-06-04`, `WL-06-05`, `WL-06-06`, `WL-06-11`, `WL-06-12`, `WL-06-13`, `WL-06-16`, `WL-06-19`, `WL-06-20`, `WL-06-21` | 1. 接通 `__flurry_comptime_phase`/`__flurry_direct_passing`/`__flurry_effect_strategy`/`__flurry_volatile`/`__flurry_stable_since`（接入 `type_id`）；2. 解决 `default` 关键字冲突导出 `attrs.derivation.default`；3. 在 `build` 中实现 `package`/`targets`/`tasks`（面向单一 `luna` CLI 的构建 DAG 与显式失效依赖）；4. 实现 `PipelineProtocol`/`KernelInterface`/`LaunchConfig`、宿主 `target.Scalar`/`target.Simd` 分层内联汇编封装与 `IIR.access` + `meta.runtime.invoke` 管线闭环。 | `05 WL-05-04/31/32` (`BoundedStackPool` 联动、`type_id` 稳定锚点)、`07` `Zeroize` / `std.simd` / `std.os` 迁移、Tier 2 `tensor_protocol.GpuProjection`。 | `luna` 单二进制多包 DAG 调度与增量缓存失效回归测试；`build.target` 跨目标 feature 校验与 `target_feature` 拒绝测试；`HostInvokeErr.Diagnostic` 诊断链回溯测试；禁止直接调用投影 kernel 负例测试。 |
| **Phase P2（生成文件沙盒、构建脚本工具、`DeviceCopy`/`DeviceBuffer` 与元数据防冲突）** | `WL-06-09`, `WL-06-14`, `WL-06-15`, `WL-06-17` | 1. 完成 `build.generated` 与 `build.util` 沙盒路径校验及自动依赖追踪；2. 落实 `attrs.metadata` 与 Tier 2 非 `__flurry_*` 领域键防覆盖策略；3. 完整接入 `DeviceCopy` 边界检验、`DeviceBuffer` 轻量契约与 `SharedMem` 静态片上存储协议，衔接 Tier 2 `tensor_protocol`。 | 外部代码生成脚本、Tier 2 `tensor_protocol`（`10 WL-10-18`）与 Tier 3 `cuda`/`vulkan`/`wgsl` 生态包。 | 沙盒逃逸路径（`../` 与 包外读写）拦截测试；`DeviceCopy` 拒绝含裸宿主指针或 `Drop` 字段负例测试；非 `^attrs.*` 构造 `__flurry_*` 拦截测试。 |
| **Phase P3（精确字段级增量观察缓存与跨生态管线插件成熟化）** | `WL-06-10/13` 深化、`WL-06-21` 深化 | 1. 从保守的“全请求级缓存失效”演进至基于 `InvalidationDep` 观测集合的精确字段级增量失效；2. 完善外部动态/AOT 后端管线插件签名校验、多产物打包清单与确定性构建重放校验。 | 大型多包工作区极速增量编译、第三方 GPU/FPGA 外部编译器生态。 | 修改未读取 `metadata` 字段时不触发上游重编译验证；修改已读取特征立即精确失效。 |

---

## 5. 待作者裁决的语义空缺与冲突（Open Questions）

### OQ-6.1 `attrs.derivation.default` 与 `attrs.compilation.inline` 受阻于成员定义位关键字规则
- **来源依据**：[library/attrs/derivation/README.md](file:///home/fin/luna/library/attrs/derivation/README.md) 与 [docs/basics/lexical.md:28,37,40-42](file:///home/fin/luna/docs/basics/lexical.md) 显示：`docs/basics/lexical.md:40-42` 已允许在限定成员路径（如 `attrs.derivation.default`）使用保留词，但定义声明位（`pub const default`、`pub const inline`）仍受 `compiler/parse` 关键字检查阻塞。
- **受影响行为**：`^attrs.derivation.default`（默认派生策略）与 `^attrs.compilation.inline`（无参内联提示属性）的合法导出。
- **最小默认建议**：在 parser 的 `mod`/成员定义位置及导出位置统一放行 `default` 与 `inline` 等上下文关键字作为成员名（严禁以 `mark_default` 之类变通命名替代），并在语法稳定前以 `inline_mode(mode: Symbol)` 并行提供细粒度控制。

### OQ-6.2 全库新增 `__flurry_*` 官方键集中登记裁决
- **来源依据**：跨文档核查发现至少 **12 项**被 `docs/`、`05`（`OQ-5.9`）、`07`（`WL-07-08/17/18/20`）与 `tmp/silver-design/04` 引用的核心属性尚未在 [official-attribute-keys.md](file:///home/fin/luna/docs/comptime/official-attribute-keys.md) 及 `library/attrs/` 中正式登记：`__flurry_align`、`__flurry_ffi_link`、`__flurry_ffi_link_name`、`__flurry_effect_strategy`、`__flurry_quote_fn_template_for_match_arms`、`__flurry_quote_fn_template_allow_non_exhaustive_instance`、`__flurry_pipeline`、`__flurry_pipeline_projection`、`__flurry_intrinsic_enum`、`__flurry_inline`、`__flurry_branch_hint`、`__flurry_target_feature`、`__flurry_caller_location`。
- **受影响行为**：`elaboration/src/program/attributes.rs` 对官方键的白名单放行与各后端消费协议一致性。
- **最小默认建议**：按本 Wishlist §3.1.1 注册总表一次性写入 `official-attribute-keys.md` 并在 `library/attrs/` 提供对应导出，同时删除 `layout_impl/constructors.fl:10-14` 的旧 `{ packed: true }` / `packed_layout` 过渡键。

### OQ-6.3 文档中的裸 `^intrinsic_enum` / 裸 `^{ __flurry_pipeline_projection: ... }` 与 `^attrs.*` 规则对齐
- **来源依据**：[docs/pipeline-projection/intrinsics.md:66,150](file:///home/fin/luna/docs/pipeline-projection/intrinsics.md) 与 [2026-8-24-intrinsic.fl:4](file:///home/fin/luna/proposal_trace/2026-8-24-intrinsic.fl)、[2026-9-7-inlining-asm.fl:4](file:///home/fin/luna/proposal_trace/2026-9-7-inlining-asm.fl) 使用裸属性 `^intrinsic_enum`，而 [declarations-and-interfaces.md:160](file:///home/fin/luna/docs/pipeline-projection/declarations-and-interfaces.md) 直接写出 `^{ __flurry_pipeline_projection: ... }`；按 `AGENTS.md` 第 5 节与 `official-attribute-keys.md`，正式属性应统一由 `^attrs.*` 包装而不直接裸写未包装配置。
- **受影响行为**：内联指令枚举与投影声明的标准源码写法。
- **最小默认建议**：统一规范写为 `^attrs.compilation.intrinsic_enum` 与 `^attrs.compilation.project(handler)`（底层生成 `__flurry_intrinsic_enum` 与 `__flurry_pipeline_projection`），同步修订 `docs/pipeline-projection/` 示例。

### OQ-6.4 `^attrs.metadata` 多重叠加语义与非 `__flurry_*` 领域键混用边界
- **来源依据**：[library/attrs/metadata_impl/constructors.fl](file:///home/fin/luna/library/attrs/metadata_impl/constructors.fl) 直接产出 `{ __flurry_metadata: data }`；[10-official-protocol-packages-and-boundary.md](file:///home/fin/luna/tmp/wishlist/10-official-protocol-packages-and-boundary.md)（§1.4 与 §5 Q4）指出若多个自定义属性均通过 `__flurry_metadata` 写入，合并阶段易发生同名外层键浅覆盖。
- **受影响行为**：同一字段/类型同时附加多个反射元数据包（如同时挂接 `serialization` 与 `database` 域配置）时的元数据保留。
- **最小默认建议**：明确区分两条通道：内置元编程官方标签保留 `__flurry_*` 且由 `attrs.*` 唯一定义；Tier 2 领域库元数据**严禁**使用 `__flurry_*` 前缀，亦不强迫塞入单一 `__flurry_metadata` 键，而是使用各自领域前缀普通键（如 `serialization_rename`）并由 `meta.reflection.attributes_of` 按出现序完整保留列表。

### OQ-6.5 `build.tasks` 缓存粒度：完整请求失效 vs 字段级观察依赖失效
- **来源依据**：[library/build/README.md:39, 96, 123](file:///home/fin/luna/library/build/README.md) 记录当前 `luna` 驱动仅按完整构建请求键（whole-request）做保守失效，尚未接通 `TargetFeatures`/`MetadataFeatures` 的按观察字段精准失效。
- **受影响行为**：修改大型工作区某个无关 feature 或 metadata 字段时的重编译范围与 CI 缓存命中率。
- **最小默认建议**：P0/P1 保持保守但 100% 正确的整包/任务请求哈希失效，待 P3 在 elaboration 与 `build.tasks.InvalidationDep` 中完成观测读日志追踪后再启用字段级增量失效。

### OQ-6.6 `ExtendedVariadicCollector` 与 `LaunchConfig` 变参收集契约归属
- **来源依据**：[declarations-and-interfaces.md:38](file:///home/fin/luna/docs/pipeline-projection/declarations-and-interfaces.md) 引用 `ExtendedVariadicCollector` 协调 `cuda.launch_kernel { ... }` 的变参属性收集，[2026-9-8-variadic-argument-collecting-rules.fl:62-71](file:///home/fin/luna/proposal_trace/2026-9-8-variadic-argument-collecting-rules.fl#L62-L71) 与 [2026-5-22-heter-flurry.fl:53](file:///home/fin/luna/proposal_trace/2026-5-22-heter-flurry.fl) 给出两阶段 `assoc Tmp: Type;` + `Tuple<Tmp, Self>` + `^attrs.comptime_phase` `extended_variadic_collect_comptime` 原型；但当前编译器、[`library/language-items.conf`](file:///home/fin/luna/library/language-items.conf) 与 `library/builtin/` 均未定义 `ExtendedVariadicCollector`。
- **受影响行为**：所有支持 `expr { prop: val, ... }` 扩展配置块调用语法的标准库与 Tier 2/3 启动接口。
- **最小默认建议**：按本 Wishlist §3.3.1 将 `ExtendedVariadicCollector` 作为正式编译器耦合 protocol 与 `VariadicCollector`（`01` §3.7.2）一并纳入 `builtin.ops`，并由作者确认 `^attrs.comptime_phase` 双阶段收集与 `language-items.conf` 绑定项名。

### OQ-6.7 `2026-9-7-inlining-asm.fl` 中 `where requires read<xs>, write<zs>` 契约归属
- **来源依据**：[2026-9-7-inlining-asm.fl:31](file:///home/fin/luna/proposal_trace/2026-9-7-inlining-asm.fl#L31) 在内联汇编函数签名中使用 `where requires read<xs>, read<ys>, write<zs>`；而 `AGENTS.md` 第 5 节与 [atoms.md](file:///home/fin/luna/docs/type/propositional-constraints/atoms.md) 明确规定 PTC 仅处理类型命题，内存读写权限与指针别名规格归属程序验证层 `verification`。
- **受影响行为**：内联汇编与 unsafe 指针包装的契约写法及检查阶段分工（PTC vs `verification`）。
- **最小默认建议**：明确 `read`/`write` 访问权限谓词不得进入 PTC 或 `Type.predicates.*`，而是作为 `verification` 包的程序契约形式保留给验证阶段消费。

### OQ-6.8 `backends-and-artifacts.md` `.diagnostic(diag)` 小写变体与 `PascalCase` 错误变体规范冲突
- **来源依据**：[backends-and-artifacts.md:65](file:///home/fin/luna/docs/pipeline-projection/backends-and-artifacts.md) 与 [2026-8-26-pipeline-projection.fl:30](file:///home/fin/luna/proposal_trace/2026-8-26-pipeline-projection.fl#L30) 在捕获 `meta.runtime.invoke(...)` 错误时使用了小写 `.diagnostic(diag)` 分支，与 `AGENTS.md` 第 5 节“错误枚举变体必须为 `PascalCase`”规范冲突（详见 `05 OQ-5.9`）。
- **受影响行为**：`HostInvokeErr` 变体定义及全部后端投影管线的错误匹配分支书写风格。
- **最小默认建议**：将 `HostInvokeErr` 错误变体统一规范为 `PascalCase` 的 `Diagnostic(Diagnostic)`（配套更新 `backends-and-artifacts.md:65` 为 `error Diagnostic(diag) => meta.compilation.emit_diagnostic(diag)`，同时修正该示例笔误 `metacompilation` -> `meta.compilation`）。
