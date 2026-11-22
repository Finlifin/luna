# Silver 反推 `luna` 编译器与内置包能力清单

> **状态**：`Draft v0.1（讨论稿）`  
> **原则**：遵循 Flurry 协作原则，不为 Silver 或某个库类型设置特设语法规则或名称特判；将 Silver 暴露出的缺口抽象为**通用编译器机制**与**标准库/内置包通用协议**。

---

## 1. 优先级总览

| 编号 | 能力项 | 所属层 | 现行文档/实现现状 | 对 Silver 的阻断程度 |
|---|---|---|---|---|
| **P0-1** | `VariadicCollector` & `ExtendedVariadicCollector` elaboration 协议 | `luna` elaboration + `builtin` | 已有 `2026-9-8-variadic-argument-collecting-rules.fl` 设计草稿；当前仅接通默认 tuple / 匿名结构体收集 | **硬阻断** Layer 2 零宏 DSL |
| **P0-2** | 匿名结构体/记录的编译期字段反射与 occurrence typing | `meta` + PTC / `inline` 控制流 | [reflection-and-layout.md](file:///home/fin/luna/docs/type/reflection-and-layout.md) 已接通名义结构体的 `meta.struct_field_names` / `meta.struct_field_types`；尚缺对匿名记录 `Ps` 的统一字段反射及泛型字段名投影收窄 | **硬阻断** 零宏属性分发与拼写检查 |
| **P0-3** | 限定边界静态栈上界查询与可替换 `StackPool` 协议 | `luna` NIR/IIR/LLVM + `std.runtime` | `effect-abi.md` 已定义 `.use(pool)` 概念与捕获栈契约；当前原生执行仅覆盖直接 handler 与 direct passing | **硬阻断** 栈实例模型与有界栈池 |
| **P1-1** | `builtin.Continuation` 捕获/恢复/放弃接通与安全类型化包装 | `luna` effect ABI + `builtin`/`std` | `parameter-elaboration.md` 注明函数签名的 `catch` 绑定与原始显式恢复尚待完整接通 | **硬阻断** 调度器唤醒 `WidgetSlot` |
| **P1-2** | 开放效应行完整统一、转发包装与 `direct_passing` 优化 | `luna` row solver + LLVM codegen | `rows.md` 已描述算法；开放效应行多态与跨嵌套组件的零开销传递仍需完善 | **高优先** 避免组件签名手写繁琐效应行 |
| **P1-3** | `async effect` 调度器接入与结构化取消协议 | `std.runtime` + `luna` | `async-algebraic-effect.md` 定义完整语义；`std/runtime/` 尚待落地真实实现 | **高优先** 组件内异步请求与卸载自动取消 |
| **P1-4** | 运行时依赖类型（`quote scene: lift *LayoutScene`）完整环境交付 | `luna` elaboration + IIR | `runtime-dependent-types.md` 已有设计；跨函数传递、容器封装与隐式环境合成待全通 | **硬阻断 Layer 4** `NodeIndex<scene.ref>` 与 `FrameArena` |
| **P1-5** | 可移植 SIMD（`std.simd`）与显式缓存行对齐属性 | `builtin` / `std` + `attrs` + LLVM | `docs/comptime/attribute.md` 已有属性机制；需标准 `^attrs.layout.align(align_bytes)` 与便携 SIMD 向量原语 | **高优先 Layer 4** 无分支 SoA 布局、颜色空间转换与消除伪共享 |
| **P2-2** | `^attrs.packed.bit` 降码与 `LiteralInto` / `BitVec` 原子人体工学 | `builtin` + `luna` | [structs.md](file:///home/fin/luna/docs/type/user-defined-types/structs.md) 已有 `^attrs.packed.bit` 设计（`library/attrs` 已提供），待接入 `elaboration`/`nir` 到 `BitVec<w>` + `bitvec_opt` 的直接降码 | **中优先** 标志位结构、`12px`/`200ms` 单位与动态 `DirtyMask` |
| **P2-3** | GPU 着色器管线映射（SPIR-V + WGSL 双目标 AOT） | `pipeline-projection` + GPU 库 | [pipeline-projection](file:///home/fin/luna/docs/pipeline-projection/index.md) 已有完整设计框架；GPU Compute/Fragment 目标后端待接入 | **远期升级** 初期用 Slang/WGSL 离线编译，远期纯 Flurry 编写着色器 |

---

## 2. 逐项详细需求与通用化抽象

### P0-1：`VariadicCollector` 与 `ExtendedVariadicCollector` 协议接通

- **Silver 遇到的痛点**：  
  `vbox { gap: 12px, padding: 16px, text { ... }, button { ... } }` 需要把异构位置子视图（或收集进定制容器 `ComponentChildrenList`）和命名属性（校验并转换为 `ComponentPropertySet`）分别收集，且不能把子视图先全盘装进未知大小的堆 `Vec`，而是在调用方栈上用 `assoc Tmp` 暂存后再交付给 `Self`（或直接保持零拷贝静态元组结构）。
- **需要的通用能力**：
  1. 在 `builtin` 定义标准 `VariadicCollector` 与 `ExtendedVariadicCollector` trait（含 `^comptime_phase` 两相版本）；
  2. elaboration 在路由位置收集与扩展属性收集时，优先查找目标参数类型 `T` 是否实现对应 collector trait，若未实现则回落到默认 `Tuple` / 匿名结构体收集；
  3. 严格锚定 `assoc Tmp` 临时存储与 `Self`（如 `Slice<T>`）之间的生命周期/可达依赖（`Tmp` 必须活过整次调用并按普通准备计划清理）。

### P0-2：匿名结构体/记录字段的编译期反射与分支收窄

- **Silver 遇到的痛点**：  
  `extended_variadic_collect(comptime Ps: Type, properties: Ps)` 收到的是用户手写任意字段组合生成的匿名结构体类型 `Ps`。框架需要遍历/检查 `Ps` 拥有的字段（例如是否写了 `padding`、类型是否可转换到 `Pixels`、是否出现了拼错的未知属性 `pading`），同时零运行期开销。
- **需要的通用能力**：
  1. 扩展现有的 `meta.struct_field_names(T)` / `meta.struct_field_types(T)`，使其统一支持属性收集生成的匿名记录 `Ps`（或补齐通用的 `meta.record_fields_of(Ps)` 与 `Type.predicates.has_field(Ps, .padding)`）；
  2. 结合 Flurry PTC / occurrence typing，在 `inline if` 分支确认字段存在后允许直接访问 `properties.padding` 并继续类型检验；
  3. 能够在 `comptime` 收集剩下的未识别字段并在编译期调用 `meta.compilation.err(...)` 给出带属性源码 Span 的精确报错。

### P0-3：限定边界静态栈上界计算与可替换 `StackPool` 协议

- **Silver 遇到的痛点**：  
  Silver 假设组件栈在编译期能确定一个不含界外 handler 的栈帧上界，从而直接落入 `BoundedStackPool<128/256/512/...>`，省掉 64KB 虚拟内存预留和 guard page 系统调用。
- **需要的通用能力**：
  1. **静态栈深分析 Pass**：在 LLVM/后端降码后（或 IIR/MIR 阶段）计算某函数在到达 `effect` 限定边界前、仅计入**本地调用图**（不含穿过 effect 边界向外执行的外层 handler 分支）的最大栈帧字节数（含被调用普通函数的局部帧对齐填充），标记为可静态定界（`?usize`：存在递归/动态未知回调且无法证明上界时返回 `null` 降级到 Tier 2 保护池）；
  2. **暴露可组合的编译期元信息或优化契约**：如 `meta.bounded_stack_bytes(entry_fn)`（或由编译器对 `.use(pool)` 自动验证选定 `BoundedStackPool<slot_bytes>` 的容量 `slot_bytes >= computed_bound`）；
  3. **规范化 `StackPool` trait**：在 `std.runtime`（或 `builtin`）稳定定义 `.use(pool)` 所需的控制后端与栈存储分配协议（支持 `allocate_stack`、`release_stack`、C 帧屏障检测）。

### P1-1：`builtin.Continuation` 捕获、安全封装与取消通道

- **Silver 遇到的痛点**：  
  调度器需要把挂起在 `pull_event()#` 的不同组件放入同构就绪/等待表里统一管理，事件到达时恢复一次，卸载时丢弃（触发隐藏中止通道清理 `Drop`）。裸 `builtin.Continuation` 没有公开类型参数且显式恢复属于 `unsafe` 契约。
- **需要的通用能力**：
  1. 完整接通 `effect ...(catch continuation: builtin.Continuation)` 的捕获生成、隐藏中止通道展开（沿退出帧逆序 `Drop`）及 C 帧越界 trap 检查；
  2. 在 `std.runtime` 或核心库中支持编写薄 `unsafe` 类型安全适配器（例如 `TypedContinuation<Y, B>`），对外暴露**safe 的单次消耗（`Once`）接口**：`resume(self, value: Y)` 与自动卸载 `Drop`。

### P1-2：开放效应行多态与 `direct_passing` 零开销传递

- **Silver 遇到的痛点**：  
  布局容器（如 `scroll_view`、`card`）经常包裹用户传入的任意闭包或子视图，这些子视图可能使用 `#Theme`、`#Locale`、`#CustomDialog` 等外部效应。若语言不能顺畅表达和统一开放效应行尾（row polymorphism），每个包装组件都会被迫写出极其冗长的效应签名。
- **需要的通用能力**：
  1. 完善行变量参数化与自动行扩展（如高阶组件接收 `content: fn() -> #e void where e: Type.vars.EffectRow`）；
  2. 优化 `^attrs.effects.direct_passing` 槽位传递：支持从 `WidgetSlot` 恢复上下文一键重绑外层 direct-passing 指针表，并把无动态覆盖的热点路径内联消去。

### P1-3：`async effect` 运行时与协作取消接线

- **Silver 遇到的痛点**：  
  组件中的防抖定时器、网络请求、文件加载依赖 `async effect` 的结构化范围取消语义。
- **需要的通用能力**：
  1. 在 `std.runtime` 实现基于栈池和任务取消标志的异步运行时实例；
  2. 当外层 `WidgetSlot` 放弃组件 continuation 导致其内部的 `async` 安装范围退出时，能向未完成子工作原子发出取消请求并等待清理结束后再归还底层有界栈槽。

### P1-4：运行时依赖类型完整交付（服务 Layer 4 SoA 句柄与 Arena）

- **Silver 遇到的痛点**：  
  Layer 4 的 SoA 场景树与保留句柄（`NodeIndex<scene.ref>`）以及 `FrameArena` 依赖 `where quote scene: lift *LayoutScene` 让轻量句柄保持 `4B` 本体大小而不逐个附带指针，同时从静态类型上防止跨窗口混用节点索引与旧树释放后悬垂访问。
- **需要的通用能力**：
  1. 完整接通 `quote` 运行时依赖参数在结构体方法、闭包捕获、数组/容器元素（共享外层环境、不增逐元素存储）以及跨函数隐藏环境传递中的 elaboration；
  2. 配合 `*T` / `&r T` 验证容器销毁/重分配时旧 `NodeIndex` 句柄失效的静态诊断。

### P1-5：可移植 SIMD 与显式缓存行对齐属性

- **Silver 遇到的痛点**：  
  SoA `LayoutScene` 遍历包围盒相交、`OKLCH`/`LinearRgba` 批量色域转换以及双线程无锁 `FrameCommit` 头尾指针都需要硬件级吞吐与防伪共享（False Sharing）对齐保障。
- **需要的通用能力**：
  1. 标准化的 `^attrs.layout.align(64)` / `^attrs.layout.align(128)`（覆盖 Apple Silicon 128B 缓存行与 x86 64B 缓存行）结构体及字段对齐属性（支持合法的常量 `align_bytes` 参数）；
  2. 跨架构可移植 SIMD 类型与归约原语（`std.simd`，统一映射到 AVX2/NEON/WASM SIMD），配合无分支位掩码运算加速热路径。

### P2-2：`^attrs.packed.bit` 降码、`LiteralInto` 与 `BitVec` 基础库补齐

- **Silver 遇到的痛点**：  
  Silver 所有具名固定标志位与 GPU POD 标志字（`KeyModifiers`、`NodeFlags`、`NodeDirtyFlags`、`BackendFeatures`、`LayerFlags`）全部采用 Flurry 的 `^attrs.packed.bit` 结构体（避免原始 `BitVec` 魔数下标）；仅组件内按动态槽位编号索引的 `DirtyMask` 使用 `BitVec<n>`，同时 DSL 需要 `12px`、`2.0rem`、`150ms` 单位后缀。
- **需要的通用能力**：
  1. 完成 `elaboration` / `nir` 对 `__flurry_packed_mode: .bit` 的支持（校验 POD 位字段、直接降码为 `BitVec<w>` 并复用 `bitvec_opt` 优化通道，参见 [structs.md](file:///home/fin/luna/docs/type/user-defined-types/structs.md)）；
  2. `LiteralInto` 对整数/浮点后缀的全路径接入；
  3. `builtin` 对动态位集合场景的 `BitVec<n>` 提供便捷的位设置/清空/首个置位查找方法。

### P2-3：GPU 着色器 `pipeline-projection` 双目标 AOT 后端（远期）

- **Silver 遇到的痛点**：  
  Layer 4 严禁运行时着色器编译，需要由单一着色器源在构建期 AOT 生成 Vulkan SPIR-V 与 WGPU WGSL 双产物（Analytic-AA、Compute 矢量光栅化、线性光学混合）。
- **需要的通用能力**：
  1. 初期工程允许通过 Slang/WGSL 离线编译推进渲染原型；
  2. 中长期将 Flurry [pipeline-projection](file:///home/fin/luna/docs/pipeline-projection/index.md) 接入 GPU SPIR-V / WGSL 目标后端，使渲染着色器本身也用安全的 Flurry 语言编写并共享常量/POD 图元布局定义（例如 `QuadPrimitive`）。

---

## 3. 待讨论问题（Open Questions）

- **Q4.1（驱动开发优先级）**：在当前的 `luna` 与 `library` 开发排期里，如果先挑 1–2 项做验证原型（PoC），你是倾向先打通 **P1-1 + P0-3（最小可运行的多组件有界栈 `pull_event` 调度原型）**，还是先打通 **P0-1 + P0-2（`ExtendedVariadicCollector` 零宏 DSL 语法与收集原型）**？
- **Q4.2（Wayland 加速专题增补）**：Layer 4 渲染与文本质量需求已纳入（`P1-4`、`P1-5`、`P2-3`）；后续展开 `07-wayland-accel-protocol.md` 时，如涉及跨进程共享内存安全边界、Wayland XML 绑定生成或 `dma-buf` 零拷贝原语，再增补入本清单。
