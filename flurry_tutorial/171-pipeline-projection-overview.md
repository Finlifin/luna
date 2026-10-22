# Flurry 演进手记 (171)：一份源码，跨越 CPU 与加速器——流水线投影（Pipeline Projection）总览

欢迎来到《Flurry 演进手记》的最后一卷——**第八卷：打通异构硬件与编译器心脏（流水线投影与 Luna 编译架构）**。

在前面的七卷旅程中，我们已经从基础语法（第 1 卷）、Trait 与错误模型（第 2 卷）、所有权与程序验证（第 3 卷）、2LTT 与 PTC 逻辑求解器（第 4 卷）、编译期派生与 BitVec 合成（第 5 卷）、CMTT 与宏系统（第 6 卷），一路走到了代数效应与异步/STM 并发（第 7 卷）。

现在，我们的高性能实时传感器信号处理与网关系统迎来了最终挑战：
单靠主机（Host）CPU 的核心已经难以企及每秒数千万次的高维张量滤波与雷达回波相关性计算了。我们需要把最密集的计算核卸载（Offload）到 **GPU（如 CUDA）、专用 NPU 或 SIMD 协处理器** 上执行。

但一提到异构硬件编程，几乎所有系统工程师都会倒吸一口凉气：
- 难道我们必须把好好的一份 Flurry 工程劈成两半，去用 CUDA C++、OpenCL C 或各种割裂的着色器方言重写一遍计算核？
- 难道我们辛苦建立的 2LTT 类型安全、结构体布局契约和错误传播模型，在跨越 CPU 与 GPU 边界的瞬间就要退化成毫无类型保护的 `void*` 数组指针和手写字节偏移？

在 Flurry 中，答案是响亮的 **“不。”**。
通过 **流水线投影（Pipeline Projection，`docs/pipeline-projection/index.md`）**，我们可以用同一份 Flurry 源码描述设备计算核（Kernel），通过普通属性（如 `^cuda.global`）将其直接投射到目标硬件编译管线，同时让宿主程序立即获得严格可检查的**先行类型接口**。今天，我们就来拉开异构系统统一编程的序幕。

---

## 一、 问题背景与设计思路：传统异构编程的“双语言撕裂症”

在传统的 CPU + GPU 异构开发中，主机代码和设备代码往往生活在两个语言平行宇宙里：

```
  [ ❌ 传统方案 传统异构开发的“双语言撕裂” ]
  ┌──────────────────────────────┐         ┌──────────────────────────────┐
  │ 主机端 (Host: Rust / Python) │         │ 设备端 (Device: CUDA C / PTX)│
  │ • 维护一套结构体与参数打包   │  void*  │ • 另写一套 C 方言的 kernel   │
  │ • 靠字符串名字查找 "add_k"   │ ──────► │ • 无法直接复用主机的纯辅助函数│
  │ • 参数个数或类型传错？       │ 盲目传参│ • 编译器特制语法 <<<grid,blk>>>│
  │   只有跑到显卡驱动崩溃才发现  │         │   侵入全局语言文法           │
  └──────────────────────────────┘         └──────────────────────────────┘
```

这种割裂带来了三大严重问题背景：
1. **失去了跨边界的静态类型检查**：当你通过驱动 API 按字符串名字加载 `"add_kernel"` 并把参数塞进 `void**` 数组时，哪怕宿主端少传了一个指针，或者把 `*i32` 传给了期待 `*f32` 的设备核，编译器在编译期也完全是个瞎子。
2. **特权语法污染主语言**：像 CUDA C++ 那样为了启动一个 Kernel 专门在语法分析器里硬塞一套 `kernel<<<grid, block>>>(args)` 的三尖括号特权语法，既破坏了语言的统一性，又无法推广到其他加速器管线。
3. **辅助函数（Helper）被迫到处打标签**：在 CUDA C++ 里，一个简单的纯数学函数 `add_values` 如果不手动标上 `__device__` 或 `__host__ __device__`，设备核根本没法调用它。

---

## 二、 核心心智模型：什么是流水线投影（Pipeline Projection）？

Flurry 的 **流水线投影（Pipeline Projection）** 用两条正交的设计化解了上述所有问题背景：
1. **表面语法零特权扩展**：声明设备入口直接复用第 077 回学过的标准前缀属性 **`^expr`**（例如 `^cuda.global`），启动设备工作直接复用标准库函数与扩展调用语法（`cuda.launch_kernel { ... }`），底层硬件指令直接复用视图调用 **`expr'intrinsic(args).as(T)`**。
2. **先行类型接口（Upfront Type Interface）与后端产物交付解耦**：当你在函数头顶标上 `^cuda.global` 时，投影协议立刻根据函数签名和属性向宿主程序提供一个**可静态检查的类型接口**（宿主无需苦等整个 GPU 后端把 PTX/机器码生成完，就能当场做完启动实参的类型检查。）；与此同时，函数主体及其实际调用的普通 Helper 通过受控句柄交给目标管线后端去验证和生成设备代码。

请看这张来自 `docs/pipeline-projection/index.md` 的核心架构分流图：

```
+-----------------------------------------------------------------------------+
|                    Flurry 流水线投影 (Pipeline Projection) 架构               |
+-----------------------------------------------------------------------------+
|                                                                             |
|   源码声明：`^cuda.global unsafe fn add_kernel(left: *f32, ...)`             |
|                         │                                                   |
|                         ▼                                                   |
|                    [ 投影协议 ]                                              |
|           ┌─────────────┴─────────────────────────┐                         |
|           │ (路径 A：立即对宿主公开)               │ (路径 B：深入目标后端)   |
|           ▼                                       ▼                         |
|   [ 可检查的公开类型接口 ]                 [ 主体与实际依赖闭包 ]             |
|   • 约束 `cuda.launch_kernel` 的实参类型   • 连同调用的普通 helper 一起打包   |
|   • 严禁将 kernel 当作普通宿主函数直接调用 • 交付给目标管线后端编译           |
|                                                   │                         |
|   [ 显式读取的包配置 `build.package_metadata` ] ──┤                         |
|                                                   ▼                         |
|                                             [ 目标设备后端 ]                 |
|                                                   │                         |
|                                                   ▼                         |
|                                    [ 目标验证、设备机器码与产物交付 ]         |
+-----------------------------------------------------------------------------+
```

---

## 三、 实战：用 `^cuda.global` 声明设备核，并在宿主端类型安全地启动

让我们直接来看 `docs/pipeline-projection/index.md` 中的基准示例。
我们要编写一个在 GPU 上执行一次单精度浮点加法的核函数 `add_kernel`，并在宿主函数 `submit_add` 中通过 `cuda.launch_kernel` 提交它：

```flurry
use cuda;
use cuda.prelude.*;

-- 1. 使用 ^cuda.global 属性将函数投射到 CUDA 设备编译管线
--    注意：三个指针参数必须满足 CUDA 库对设备可访问存储（Device Memory）的契约
^cuda.global
unsafe fn add_kernel(left: *f32, right: *f32, output: *f32) {
    output.* = left.* + right.*;
}

-- 2. 宿主侧通过启动接口提交该 kernel，并通过 `!` 传播可能的启动错误 !CudaErr
unsafe fn submit_add(
    left: *f32,
    right: *f32,
    output: *f32,
) -> !CudaErr void {
    cuda.launch_kernel {
        kernel: add_kernel,
        gridDim: (1u32, 1u32, 1u32),
        blockDim: (1u32, 1u32, 1u32),
        args: (left, right, output),
    }!;
}
```

请仔细观察这段代码中的三个关键细节：
1. **无需特制语法**：`^cuda.global` 只是普普通通的前缀属性表达式，而 `cuda.launch_kernel { ... }` 只是普普通通的扩展调用（Extended Application）加 `!` 错误传播。
2. **提交有别于完成**：`submit_add` 返回表示启动请求已按接口协议提交；设备写入何时完成，严格遵循库公开的流（Stream）或同步（Synchronization）契约，语言不会自作主张把异步提交解释成同步阻塞完成。
3. **各司其职的五大对象责任表**：

| 参与对象 | 调用者/外围代码可以依赖的契约 | 管线与后端进一步处理的责任 |
| :--- | :--- | :--- |
| **投影声明 (`add_kernel`)** | 所选投影协议提供的公开类型与启动接口 | 目标 Pipeline 交付符合该接口的设备代码与入口产物 |
| **普通 Helper 函数** | 自身声明的参数、返回类型与契约 | 所在目标管线验证其实际依赖在设备上是否全部可用 |
| **Intrinsic 调用** | 紧接调用的 `.as(T)` 显式结果类型（缺省为 `void`） | Pipeline 解释指令标签、操作数合法性与目标硬件约束 |
| **Host Pipeline** | `build` 默认描述的宿主编译管线环境 | 普通宿主程序的编译与宿主目标检查 |
| **其他 Pipeline 配置** | 显式查询 `build.package_metadata` 取得的信息 | 对应设备库检查配置并驱动自己的目标后端 |

---

## 四、 进阶：为什么决禁止将设备 Kernel 当成普通宿主函数直接调用？

很多初学者看到 `add_kernel` 是用 `unsafe fn add_kernel(...)` 写出来的，就会产生一个危险的错觉：“既然它是用 `fn` 写的，而且我已经包在 `unsafe { ... }` 块里了，我能否在 CPU 宿主代码里直接写 `add_kernel(left, right, output)` 调用它？”

**不行。编译期直接报错。**

```
  [ 为什么机器字长相同 ≠ 调用协议相同？ ]
  • 宿主函数指针：指向宿主 CPU 虚拟地址空间中的 x86_64 / ARM64 指令流，遵循 System V / C ABI 栈帧调用。
  • 设备入口引用 (`add_kernel`)：代表投射到 GPU 管线的 Kernel 句柄，需要网格维度 (`gridDim`/`blockDim`)、
    参数序列化缓冲与硬件队列提交协议。
```

> **💡 核心心智模型：主体采用 `fn` 写法，非自动承诺生成一个可按宿主函数 ABI 直接调用的入口。**
> 哪怕代码地址、设备入口句柄和工作状态在底层都碰巧能装进一个 64 位机器字（`usize`）里，它们在 Flurry 的类型系统中也是迥然各异的实体。`unsafe` 关键字只用来授权那些需要调用者保证前置条件的底层接口，**免将一个 GPU 设备入口变成 CPU 普通函数**。

来看这个铁面无私的编译期负例（源自 `docs/pipeline-projection/declarations-and-interfaces.md`）：

```flurry
use cuda;
use cuda.prelude.*;

^cuda.global
unsafe fn add_kernel(left: *f32, right: *f32, output: *f32) {
    output.* = left.* + right.*;
}

-- ❌ 编译期直接报错：add_kernel 是投影到 CUDA 管线的设备入口，未提供普通宿主函数调用接口；
-- 哪怕外层加了 unsafe，也禁止绕过启动协议直接按宿主函数调用。
unsafe fn invalid_host_call(left: *f32, right: *f32, output: *f32) {
    add_kernel(left, right, output);
}
```

---

## 五、 本篇小结

| 核心概念 | Flurry 的设计抉择 | 解决的异构核心问题 |
| :--- | :--- | :--- |
| **投影入口声明** | 标准前缀属性 `^expr`（如 `^cuda.global`）修饰 `fn` | 无需发明特权语法，主机与设备共享同一套词法、语法与类型系统 |
| **先行类型接口** | 无须等设备后端生成完机器码即可供宿主做类型检查 | 编译期立即拦截启动参数类型或个数失配的 Bug |
| **调用协议隔离** | 设备 Kernel 只能通过协议启动接口使用，禁止直接作宿主调用 | 避免将设备句柄当成 CPU 函数指针直接跳转的问题 |
| **三道检查不变量** | 接口不被后端篡改、无未支持操作或伪造宿主地址、产物完整可恢复 | 确保跨管线编译的端到端确定性与类型安全 |

---

## 六、 下一篇预告

在上面的 `submit_add` 中，当我们写下：
```flurry
unsafe {
    cuda.launch_kernel {
        kernel: add_kernel,
        gridDim: (1u32, 1u32, 1u32),
        blockDim: (1u32, 1u32, 1u32),
        args: (left, right, output),
    }!
}
```
你一定很好奇三个深层问题：
1. `cuda.launch_kernel` 的配置类型 `LaunchConfig` 到底是怎么在类型系统里把 `kernel: add_kernel` 的参数签名与 `args: (left, right, output)` 的元组类型死死绑在一起的？
2. 如果 `add_kernel` 内部调用了一个普通的纯数学辅助函数 `fn add_values(a: f32, b: f32) -> f32`，我们需要给 `add_values` 也标上 `^cuda.global` 吗？
3. 当我们在包含 `^cuda.global` 的文件里读取 `build.target` 时，它返回的是宿主（Host）的目标配置，还是 GPU 设备的目标配置？怎么正确读取 CUDA 包自己的架构配置（如 `target_arch`）？

下一回 **《Flurry 演进手记 (172)：声明一个 GPU / 设备 Kernel——投影属性、先行类型接口与主机/设备参数契约》**，我们将揭晓投影声明与配置读取的底层契约。
