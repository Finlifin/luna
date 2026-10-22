# Flurry 演进手记 (176)：宿主编译器如何交付设备产物——IIR Handle、多后端协同与增量缓存恢复

在[上一篇（第 175 篇）](./175-target-validation-simd-datapaths-and-assembly.md)中，我们看到编译器如何顺着具体实例的调用图，把整棵目标执行依赖闭包（Required Dependency Closure）验证得滴水不漏。

当类型检查、编译期归一化（Normalization）、具体实例化（Instantiation）和目标能力检查全部通过后，编译器手里已经有了一份干净、确定、去除了所有未决类型变量的 **IIR（Implementation IR，实现层中间表示）**。

现在，真正的跨平台交付时刻到了：
我们在业务代码里只在函数头上贴了一句 `^cuda.pipelines.global`。**这句属性到底是如何触发宿主机上的 `cuda_compile` 编译器后端、把 IIR 变成 GPU 可执行的设备产物（Device Artifact）、并把编译错误原原本本传回给 IDE 的？**
更关键的是：在大型工程的增量构建中，如果源码没变、但磁盘上的 `.cubin` 文件坏了，或者 `build.package_metadata("cuda")` 里的目标架构从 `sm_80` 改成了 `sm_90`，编译器如何保证不交付错误或残缺的产物？

今天，我们就来拆解管线投射的最后一块拼图：**后端接入、IIR 受控句柄与产物状态机**。

---

## 一、 异构后端桥接的设计挑战：如何在零拷贝与类型安全之间取得平衡？

在设计自定义编译管线接口时，语言设计者通常会面临两个糟糕的特殊：

1. **方案 A（纯编译期解释器硬算）**：试图把整棵函数的控制流图、基本块、指令树全部拷贝成编译期解释器里的数据结构（比如几万个嵌套的 `meta` 结构体），让用户在 `comptime fn` 里手写寄存器分配和 PTX 生成。这非但会让编译期内存瞬间爆炸、编译速度慢上百倍，而且也没法复用成熟的外部原生编译工具链（如 LLVM、NVVM、SPIR-V 工具链）。
2. **方案 B（外部 Shell 脚本黑盒）**：放弃在语言内定义管线，退回到写一堆混乱的 Makefile / Python 脚本去调外部命令行。结果就是：设备端的编译报错丢失了源码位置（Span），无法融入宿主编译器的结构化诊断流；增量缓存无从得知设备后端读了哪些配置，一改配置就链接出诡异的崩溃。

Flurry 的答案是：**用 `comptime fn` 作为强类型的“管线粘合层（Glue Layer）”，通过受控的 `IIR Handle` 与 `meta.runtime.invoke`，在宿主编译任务通道中安全驱动原生后端程序。**

---

## 二、 Pipeline 回调接口：`cuda.pipelines.global` 的工作原理

让我们直接掀开 `cuda` 库的面纱，看看在第 171～175 篇里反复出现的 `^cuda.pipelines.global` 到底是怎么写出来的（源自 `docs/pipeline-projection/backends-and-artifacts.md`）：

```flurry
use cuda_compile;

^attrs.compilation.pipeline
pub pure comptime fn global(ir: meta.compilation.IIR.Func) -> void {
    meta.runtime.invoke(
        cuda_compile.compile_kernel,
        meta.compilation.IIR.access(ir),
    )! {
        .diagnostic(diag) => meta.compilation.emit_diagnostic(diag),
    }
}
```

这段只有短短十几行的代码，浓缩了 Flurry 编译器架构的三项核心设计：

### 1. 为什么 `global` 的返回类型是 `void`？产物去哪儿了？
初看这段代码，你可能会惊讶：`global` 接收了 `ir: meta.compilation.IIR.Func`，返回类型却是 `void`。那编译出来的 GPU 机器码和符号表去哪儿了？

答案是：**管线函数本身只负责建立并触发编译任务，真正的设备产物、入口映射和诊断信息，全部通过宿主编译器托管的“编译任务通道（Compilation Task Channel）”交付。**
如果让 `global` 直接返回一个庞大的字节数组 `Array<u8, n>` 回填到 AST 里，非但会污染原函数的类型签名，还会破坏多目标并发编译的流水线调度。

### 2. `meta.runtime.invoke`：连接编译期粘合层与宿主机后端的桥梁
请注意 `global` 是一个 `pure comptime fn`，它运行在编译期环境中；而 `cuda_compile.compile_kernel` 是一个真正执行重度代码生成、调用底层设备工具链的**宿主机后端程序**。
- `meta.compilation.IIR.access(ir)` 为当前任务申请一个受控的 IIR 只读访问视图；
- `meta.runtime.invoke(...)` 将这个视图移交给宿主机上的 `cuda_compile.compile_kernel` 执行；
- 如果后端在降级或编译过程中发现了硬件特有的错误（例如寄存器溢出或暂未涵盖的内联汇编约束），它会通过错误分支返回结构化的 `.diagnostic(diag)`，再由 `meta.compilation.emit_diagnostic(diag)` 精准挂载回 Flurry 的编译期诊断面板。

```text
  [ 被投射的源码函数 ]
  ^cuda.pipelines.global
  fn saxpy(...) -> void
         │
         │ 1. 前端 Elaboration -> Normalization -> Instantiation
         ▼
  ┌────────────────────────────────────────────────────────┐
  │ 宿主编译器产生具体实例句柄: ir : meta.compilation.IIR.Func │ (生命周期严格绑定当前任务)
  └──────────────────────────┬─────────────────────────────┘
                             │
                             │ 2. 调用编译期管线回调 global(ir)
                             ▼
  ┌────────────────────────────────────────────────────────┐
  │ meta.runtime.invoke(cuda_compile.compile_kernel,       │
  │                     meta.compilation.IIR.access(ir))   │
  └────────────┬─────────────────────────────┬─────────────┘
               │ 成功交付                     │ 失败返回 .diagnostic(diag)
               ▼                             ▼
  ┌──────────────────────────┐  ┌──────────────────────────┐
  │ 编译任务通道登记：        │  │ meta.compilation         │
  │ - 设备二进制产物 (Cubin) │  │   .emit_diagnostic(diag) │
  │ - 宿主与设备入口关联表   │  │ (停止发布，保留错误源码Span)│
  └──────────────────────────┘  └──────────────────────────┘
```

---

## 三、 受控句柄契约：为什么 `IIR.Func` 绑定于编译任务生命周期？

在 `global(ir: meta.compilation.IIR.Func)` 中，`ir` **并非**一整棵拷贝出来的语法树，亦非一个可以随便转换成整数存进全局缓存的裸指针。它是一个**受宿主编译任务生命周期严格约束的句柄（Scoped Handle）**。

为什么要设计成受控句柄？
1. **零拷贝的高效访问**：宿主编译器内部维护着高度紧凑的 IIR Arena。通过 `meta.compilation.IIR.access(ir)`，后端可以在授权窗口内直接读取所需的结构化数据，无需把百万行 IR 序列化成编译期对象。
2. **严禁跨任务悬垂引用**：一旦当前 `global(ir)` 编译任务结束，`ir` 句柄及其派生的访问视图立即失效。
3. **严禁将句柄存入持久化缓存**：进程内的句柄 ID 在下一次启动编译器时毫无意义。增量构建恢复必须走正规的**产物与依赖验证协议**，无法试图从缓存里“复活”一个旧的 `IIR.Func` 句柄。

让我们看一个试图把 `IIR.Func` 逃逸出授权作用域的反例：

```flurry
-- ❌ 编译期直接报错。
-- 试图把受任务作用域限制的 IIR 访问结果逃逸或在非法的上下文中伪造句柄：
^attrs.compilation.pipeline
pub pure comptime fn illegal_pipeline_escape(ir: meta.compilation.IIR.Func) -> meta.compilation.IIR.Func {
    -- 编译期直接报错：
    -- 1. 管线入口函数必须满足编译管线回调签名契约（返回 void）；
    -- 2. `meta.compilation.IIR.Func` 及其 `IIR.access(ir)` 视图受限于当前编译任务生命周期，
    --    禁止跨越任务边界驻留、逃逸为持久化值或直接作为运行时对象使用。
    ir
}
```

---

## 四、 编译任务状态机与增量缓存恢复（Cache Recovery）

当一个工程里有成百上千个 CUDA 或 SIMD 投射核函数时，每次全量重新运行 `cuda_compile` 会非常耗时。因此，宿主编译器与构建系统（`anar`）共同维护了一套严密的**管线编译任务状态机**：

```text
                     ┌──────────────────┐
                     │   发现投射任务    │
                     └────────┬─────────┘
                              │
                   检查增量缓存指纹 (Cache Lookup)
                 ┌────────────┴────────────┐
                 │ 未命中 (Miss)            │ 命中缓存记录 (Hit)
                 ▼                         ▼
         ┌───────────────┐         ┌──────────────────────────────┐
         │   Prepared    │         │ VerifyDependenciesAndArtifacts│
         └───────┬───────┘         │ 1. 源码与依赖实例 IIR 未变？ │
                 │                 │ 2. 目标配置 (如 sm_80) 未变？ │
                 ▼                 │ 3. 磁盘产物文件存在且完整？  │
         ┌───────────────┐         └───────┬──────────────┬───────┘
         │    Running    │                 │ 全部满足      │ 任一失效 (如文件丢失)
         └───────┬───────┘                 │              └───────────┐
            ┌────┴────┐                    │                          │
            │         │ 报错/异常中断      │                          │
            │         ▼                    │                          │
            │  ┌──────────────┐            │                          │
            │  │    Failed    │            │                          │
            │  │(清理残留临时文件│            │                          │
            │  │ 不发布产物!  )│            │                          │
            │  └──────────────┘            │                          │
            ▼                              │                          │
    ┌───────────────┐                      │                          │
    │   Validated   │                      │                          │
    └───────┬───────┘                      │                          │
            │                              │                          │
            ▼                              ▼                          │
    ┌──────────────────────────────────────────────┐                  │
    │                  Published                   │                  │
    │    (正式向宿主链接器与运行期分发层交付产物)    │                  │
    └──────────────────────────────────────────────┘                  │
            ▲                                                         │
            └────────────────── 重新进入 Prepared -> Running ──────────┘
```

请牢记这套状态机守护的 **4 条工程底线**：

1. **后端失败时不发布残缺产物（No Partial Publish）**：如果 `cuda_compile` 在写出一半临时文件后返回了 `.diagnostic(diag)` 或异常退出，任务进入 `Failed` 状态，临时文件被隔离清理，跳过进入 `Published`。
2. **缓存记录存在 $\neq$ 产物可用**：就算缓存数据库里写着“上次编译成功”，在 `VerifyDependenciesAndArtifacts` 阶段，编译器依然会核对磁盘上的实际产物文件。如果 `.cubin` 文件被用户误删或损坏，状态机会自动回退（Fallback）到 `Prepared -> Running` 重新生成，免将一个空路径交给链接器。
3. **目标配置与元数据纳入缓存指纹**：如果核函数或管线读取了 `build.package_metadata("cuda")`（比如把目标微架构从 `sm_80` 改成了 `sm_90`），即使业务 `.fl` 源码一个字符都没改，缓存指纹也会精准失效并触发重编。
4. **跨入口独立隔离**：同一个模块里如果有两个核函数 `kernel_ok` 和 `kernel_bad`，`kernel_bad` 的后端失败只会让它自己的任务进入 `Failed` 并精准报错，免致 `kernel_ok` 的状态发生错乱或张冠李戴。

---

## 本篇小结

| 核心概念 / 机制 | 语法与接口形态 | 架构职责与安全保证 |
| :--- | :--- | :--- |
| **管线定义属性** | `^attrs.compilation.pipeline` | 将一个 `pure comptime fn(ir: meta.compilation.IIR.Func) -> void` 注册为编译器管线入口 |
| **受控 IIR 句柄** | `meta.compilation.IIR.Func` 与 `IIR.access(ir)` | 仅在当前编译任务授权窗口内有效，零拷贝提供具体实例 IR，严禁逃逸或持久化 |
| **宿主机后端调用** | `meta.runtime.invoke(backend_fn, access_view)` | 从编译期粘合层安全触发宿主机原生编译后端，无需在编译期解释器内硬算机器码 |
| **结构化诊断回传** | `.diagnostic(diag) => meta.compilation.emit_diagnostic(diag)` | 将设备后端的硬件级错误（寄存器、指令约束等）带源码 Span 原路汇入宿主诊断流 |
| **产物状态机与缓存恢复** | `Prepared -> Running -> Validated -> Published` | 校验源码、依赖实例、目标配置（如 `build.package_metadata`）与磁盘产物完整性，避免发布半成品 |

---

## 下一篇预告

读到这里，我们已经完整走通了单段代码如何跨越 CPU 与 GPU 管线的全过程。
但是，请跳出单个文件往外看一眼：
在上面的例子里，我们用到了：
- 业务自己的包（Package）；
- 提供管线粘合接口的 `cuda` 包；
- 在宿主机上执行设备编译的 `cuda_compile` 包；
- 以及管理它们共享配置和构建调度的工作区。

在真实的工业级大型项目里，这几十个包是怎么组织在一起的？
- 什么是 **`package`**？什么是 **`umbrella`（聚合包 / 工作区）**？把两个包放进同一个 `umbrella` 里，它们能互相访问对方的内部私有函数吗？
- 为什么 Flurry 的默认可见性（省略 `pub` 也省略 `private`）既非“仅当前文件可见”，亦非“全世界可见”，而是**“当前 Package 内可见”**？
- 当你用 `pub use` 把内部模块的类型重导出（Re-export）为公共 API 时，类型的真实名义身份（Nominal Identity）会发生改变吗？

下一篇 **[Flurry 演进手记 (177)：管理大型多包工程——Package、Umbrella 聚合包与最小可见性边界](./177-packages-umbrellas-and-architecture-boundaries.md)**，我们将从单核函数视角跃升到百万行级多包工程的架构组织。
