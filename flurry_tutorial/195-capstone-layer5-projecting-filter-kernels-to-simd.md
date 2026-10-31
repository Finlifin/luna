# Flurry 演进手记 (195)：综合实战第五层（硬件加速层）：用 Pipeline Projection 与 Intrinsic 将卷积内核投射到向量指令集

在上一篇（第 194 篇）中，我们用 2LTT 与 CMTT 为 **Orion 异构流式信号处理引擎** 构建了 **Layer 4（元编程层）**，在编译期根据滤波器阶数生成了消除零抽头的展开 FIR 卷积表达式。

但是，当 Orion 引擎接入 64 通道超高频射电阵列时，仅靠单线程标量浮点乘加已经无法实时吞吐每秒数 GB 的波形流。我们需要构建 **Layer 5（硬件加速层）**：
1. 在最内层算术上，调用硬件原生的 **融合乘加指令（Fused Multiply-Add, FMA）** 与线程块同步屏障（Barrier），既将乘加延迟砍半，又避免中间浮点舍入误差；
2. 在并行拓扑上，把同一个信号处理卷积核通过 **管线投射（Pipeline Projection）** 直接编译为 GPU 设备核函数（`^cuda.global`），由主机端通过统一类型的 `launch_kernel` 启动大规模并行计算。

```
+=============================================================================+
|           Orion Signal Engine —— Layer 5: 异构管线投射与硬件内联层             |
+=============================================================================+
| Layer 4: CMTT 编译期展开卷积逻辑                                              |
|                  |                                                          |
|                  v                                                          |
| Layer 5: ^intrinsic_enum + 'intrinsic.as(T) + ^cuda.global           <-- 本篇|
|          +-- 1. 强类型内联指令: OrionHwInst.fma_f32'intrinsic(a,b,c).as(f32)  |
|          +-- 2. 异构核函数投射: ^cuda.global unsafe fn orion_fir_device_kernel|
|          +-- 3. 类型安全核启动: cuda.launch_kernel { kernel: ..., args: ... }!|
+=============================================================================+
```

在传统 C++/CUDA 工程中，主机代码（`.cpp`）与设备核函数（`.cu`）往往分裂在两套不同的编译器前端里，内联汇编或编译器内建函数（`__builtin_fmaf` / `__syncthreads`）散落在满屏的 `#ifdef __CUDA_ARCH__` 宏泥潭中。

今天，我们就来看 Flurry 如何用第 161～169 篇建立的 **同源管线投射（Pipeline Projection）** 与 **强类型 `'intrinsic` 视图**，把硬件指令与 GPU 核函数纳入统一的类型系统。

---

## 一、 问题背景：为什么传统的编译器 `__builtin_*` 总是成为类型黑洞？

在许多系统语言里，调用底层硬件指令靠的是几千个硬编码进编译器命名空间的特殊函数（如 `__builtin_ia32_vfmaddps` 或 `llvm.fma.v4f32`）。这种做法有两个严重问题：
1. **符号表污染与不可组合性**：硬件指令没法像普通枚举值那样放入数据结构、作为类型参数传递或在 `comptime` 表驱动逻辑中分发；
2. **返回类型隐式推断陷阱**：很多底层内联指令在 IR 层面是高度重载的，如果依靠外层变量注解隐式猜返回类型，一旦上下文改动就会生成错误的底层机器码。

在 Flurry（见第 168 篇与 `docs/pipeline-projection/intrinsics.md`）中，底层编译器/硬件内建指令通过一套正交的三步协议来表达：
1. **标签枚举声明（`^intrinsic_enum`）**：用普通无载荷 `enum` 配合 `^intrinsic_enum` 属性列出硬件内建原语标签；
2. **`'intrinsic` 视图调用**：必须在 `unsafe` 边界内，通过 `Tag'intrinsic(arg1, arg2, ...)` 发起调用；
3. **显式 `.as(ReturnType)` 封口**：`'intrinsic(...)` 调用如果不跟 `.as(T)`，其返回类型**严格固定为 `void`**（适用于同步屏障或纯写内存指令）；凡是产生返回值的硬件指令，**必须在调用后缀显式紧跟 `.as(T)`**，无法指望外层 `let x: T = ...` 替你倒推返回类型。

---

## 二、 实战第一步：用 `^intrinsic_enum` 与 `.as(f32)` 封装 FMA 与硬件屏障

让我们为 Orion 引擎声明硬件加速原语枚举 `OrionHwInst`，并用安全的窄包装函数封装单精度浮点融合乘加（$\text{fma}(a, b, c) = a \cdot b + c$）与线程块共享内存屏障：

```
+-----------------------------------------------------------------------------+
|            Flurry 'intrinsic 调用与 .as(T) 显式返回类型封口协议                |
+-----------------------------------------------------------------------------+
|  OrionHwInst.fma_f32 'intrinsic (sample, coeff, acc) .as(f32)               |
|  \_________________/ \________/ \__________________/ \______/               |
|           |              |               |              |                   |
|   1. ^intrinsic_enum  2. 视图转换   3. 实参元组      4. 显式返回类型           |
|      编译期常量标签       (需 unsafe)                    (省略则恒为 void)     |
+-----------------------------------------------------------------------------+
```

来看完整的代码实现：

```flurry
-- 1. 声明硬件内建指令标签枚举（免于引入 extern 关键字，完全融入类型系统）
^intrinsic_enum
enum OrionHwInst {
    fma_f32,
    block_barrier,
}

-- 2. 封装硬件单精度融合乘加：在受限的 unsafe 块内使用 'intrinsic(...).as(f32)
pure fn hw_fma_f32(sample: f32, coeff: f32, acc: f32) -> f32 {
    -- 审计职责：OrionHwInst.fma_f32 接收三个 f32 标量并返回融合乘加后的 f32，无内存副作用
    unsafe {
        OrionHwInst.fma_f32'intrinsic(sample, coeff, acc).as(f32)
    }
}

-- 3. 封装线程块同步屏障：无返回值，省略 .as(T) 时返回类型默认为 void
unsafe fn hw_sync_threads() -> void {
    OrionHwInst.block_barrier'intrinsic();
}
```

### 编译器如何拦截遗漏 `.as(T)` 或逃出 `unsafe` 的内联指令？（负向示例）

如果程序员在调用 `fma_f32'intrinsic(sample, coeff, acc)` 时忘了写后缀 `.as(f32)`，以为靠左边的变量类型注解 `const res: f32 = ...` 就能让编译器猜出返回 `f32`，编译器会直接报错：

```flurry
-- ❌ 负例 1：试图用外层 let 类型注解替代显式 .as(f32)
pure fn broken_fma_wrapper(sample: f32, coeff: f32, acc: f32) -> f32 {
    unsafe {
        -- 编译错误：Intrinsic call without explicit `.as(T)` returns `void`;
        -- outer type annotation `f32` does not infer intrinsic return type.
        -- Help: append `.as(f32)` directly to the `'intrinsic(...)` call.
        let res: f32 = OrionHwInst.fma_f32'intrinsic(sample, coeff, acc);
        res
    }
}

-- ❌ 负例 2：在未标记 unsafe 的上下文中直接调用 'intrinsic
pure fn unguarded_intrinsic(a: f32, b: f32, c: f32) -> f32 {
    -- 编译错误：Intrinsic invocation `OrionHwInst.fma_f32'intrinsic(...)`
    -- is an unsafe operation and requires an `unsafe` block or `unsafe fn`.
    OrionHwInst.fma_f32'intrinsic(a, b, c).as(f32)
}
```

---

## 三、 实战第二步：用 `^cuda.global` 将滤波核投射到 GPU 并安全启动

现在，我们把 `hw_fma_f32` 与第 194 篇的展开滤波逻辑结合起来，编写一个直接在 GPU 上并行处理数千个采样点的设备核函数 `orion_fir5_device_kernel`。

在 Flurry 的管线投射（见 `docs/pipeline-projection/index.md`）中：
- 设备入口核函数使用属性 `^cuda.global` 标注（正如 `docs/pipeline-projection/intrinsics.md` 强调的：**函数 ABI 与管线角色统一用属性指定，免于引入 `extern` 关键字**）；
- 主机端通过 `cuda.launch_kernel { kernel: ..., gridDim: ..., blockDim: ..., args: ... }!` 发起异步核启动，并且用错误传播后缀 `!` 显式承接硬件队列提交可能产生的 `CudaError`。

```flurry
use cuda;

-- GPU 设备端并行 5 阶半带 FIR 滤波核函数
^cuda.global
unsafe fn orion_fir5_device_kernel(
    input_wave: *f32,
    output_wave: *f32,
    num_samples: usize,
) -> void {
    let tid = cuda.blockIdx.x * cuda.blockDim.x + cuda.threadIdx.x;

    -- 保证滑动窗口 [tid - 4 .. tid] 严守边界
    if tid >= 4 and tid < num_samples {
        let s0 = input_wave[tid];
        let s2 = input_wave[tid - 2];
        let s4 = input_wave[tid - 4];

        -- 使用硬件 FMA 原语计算：0.25 * s0 + 1.0 * s2 + 0.25 * s4
        let stage1 = hw_fma_f32(s0, 0.25, s2);
        let filtered = hw_fma_f32(s4, 0.25, stage1);

        output_wave[tid] = filtered;
    }

    -- 块内同步屏障（返回 void）
    hw_sync_threads();
}

-- 主机端调度函数：将滤波任务投射到 GPU 流式多处理器
unsafe fn dispatch_orion_gpu_filter(
    d_in: *f32,
    d_out: *f32,
    count: usize,
) -> !cuda.CudaErr void {
    let threads_per_block: u32 = 256;
    let blocks: u32 = ((count + 255) / 256).as(u32);

    -- 强类型核启动：编译器自动校验 args 元组与 orion_fir5_device_kernel 签名完全一致。
    cuda.launch_kernel {
        kernel: orion_fir5_device_kernel,
        gridDim: (blocks, 1, 1),
        blockDim: (threads_per_block, 1, 1),
        args: (d_in, d_out, count),
    }!;
}
```

看看 `cuda.launch_kernel` 的威力：因为 `orion_fir5_device_kernel` 就是当前模块里的一等 Flurry 函数，编译器在主机端编译 `cuda.launch_kernel` 时，会直接对比 `args: (d_in, d_out, count)` 的每个元素类型与核函数的形参列表 `(*const f32, *mut f32, usize)`。一旦少传一个参数或把 `*const f32` 传给 `*mut f32`，编译期立刻报错，告别了传统异构编程里靠 `void**` 数组传参导致 GPU 段错误的难题。

---

## 四、 本层通关小结与下一层预告

在 **Layer 5（硬件加速层）** 中，我们打通了从高级类型论直达硅片指令集的最后一公里：
- 用 `^intrinsic_enum` 与 `Tag'intrinsic(...).as(T)` 将硬件 FMA 与同步屏障封装进受 `unsafe` 契约守卫的窄边界；
- 用 `^cuda.global` 属性与强类型的 `cuda.launch_kernel` 实现了零胶水代码的 GPU 卷积核投射。

然而，在真实的深空遥测与工业现场中，硬件设备并非永远可靠：GPU 核启动可能因为显存瞬时繁忙而超时，网络帧可能损坏，DMA 通道可能发生瞬态抖动。

当这些故障在异步流式管线中发生时，我们如何在完好保留上下文、不把错误处理和代数效应混为一谈的前提下，实现**带追踪上下文的自动有限重试与 CPU 软滤波平滑降级**？

在下一篇 **第 196 篇（Layer 6：控制与效应层）** 中，我们将祭出 **代数效应 `#Async`、直接传递效应 `^direct_passing` 与错误处理的“前后置双 `catch`”模式**，编排一条稳健可靠的容错流式管线。
