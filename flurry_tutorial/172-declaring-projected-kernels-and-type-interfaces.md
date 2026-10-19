# Flurry 演进手记 (172)：声明一个 GPU / 设备 Kernel——投影属性、先行类型接口与主机/设备参数契约

在上一回（171）中，我们初识了 Flurry 的**流水线投影（Pipeline Projection）**，看到了如何用一行清爽的 `^cuda.global` 将 `add_kernel` 投射到 GPU 设备编译管线，并通过 `cuda.launch_kernel` 在宿主端启动它。

今天，我们要深入 `docs/pipeline-projection/declarations-and-interfaces.md`，解开上一回留下的三个核心工程问题：
1. **普通辅助函数（Helper）的免标记复用**：当 `add_kernel` 调用一个普通函数 `add_values` 时，为什么不需要像 CUDA C++ 那样满世界贴 `__device__` 标签？
2. **启动参数与 Kernel 签名的类型绑定**：`LaunchConfig<F, A>` 是如何利用类型模式匹配 `requires F matches cuda.Kernel<A, Any, any>`，把 `kernel` 的参数列表类型 `A` 与启动配置里的 `args: A` 严丝合缝地锁在一起的？
3. **宿主配置 `build.target` vs 设备包配置 `build.package_metadata`**：为什么投影属性绝对不会偷偷篡改 `build.target` 的含义？

让我们逐一击破这三个设计重点！

---

## 一、 破局与升级：普通 Helper 函数自动纳入目标依赖，拒绝标签传染！

在 CUDA C++ 等传统异构语言中，最让库作者抓狂的就是**“`__device__` 标签传染”**（`v0` 痛点）：
如果你想在 GPU Kernel 里复用标准数学库或自己写的一个三行辅助函数 `add_values(left: f32, right: f32) -> f32`，一旦那个函数忘了写 `__host__ __device__`，编译器就会无情报错；结果逼得所有人把每个普通函数头顶都糊满平台宏标签！

而在 Flurry 中，**声明的形状、公开接口与实际目标依赖闭包是分开管理的**：
- 只有真正作为**设备启动入口**的 Kernel 函数 `add_kernel`，才需要标注 `^cuda.global`；
- 至于 `add_kernel` 在函数体内部调用的普通辅助函数 `add_values`，**完全不需要重复标注任何投影属性**！
- 当编译器处理 `add_kernel` 时，会自动顺着调用图把 `add_values` 收集进该设备入口的**实际依赖集合（Required Dependency Closure）**，交由 CUDA 管线后端去验证和编译；与此同时，如果宿主 CPU 代码也调用了 `add_values`，宿主端依然按最普通的 CPU 函数调用它！

```flurry
use cuda;
use cuda.prelude.*;

-- 普通辅助函数：无需标注 ^cuda.global！
-- 既可被宿主 CPU 代码按普通函数调用，也可被下方设备 Kernel 自动纳入目标依赖集合！
fn add_values(left: f32, right: f32) -> f32 {
    return left + right;
}

-- 只有真正的设备入口才标注 ^cuda.global
^cuda.global
unsafe fn add_kernel(left: *f32, right: *f32, output: *f32) {
    output.* = add_values(left.*, right.*);
}
```

### 1.1 投影属性背后的真相：`^cuda.global` 是如何接入编译器的？

回忆第 077 回，Flurry 的属性 `^expr` 遵循 100% 标准的词法名称解析，在编译期求值为一个 `Object`。
编译器之所以知道 `^cuda.global` 是一个流水线投影属性，**绝不是因为编译器硬编码判断了“属性名字最后一段叫 `global`”**，而是因为 `cuda.global` 求值出的属性对象里，明确编码了编译器约定的投影回调字段 `__flurry_pipeline_projection`！

在内部语义上，写 `^cuda.global` 等价于显式写出下面这个属性对象：

```flurry
-- 与 ^cuda.global 对应的显式底层属性对象表示
^{ __flurry_pipeline_projection: cuda.pipelines.global }
unsafe fn add_kernel_explicit(left: *f32, right: *f32, output: *f32) {
    output.* = left.* + right.*;
}
```

---

## 二、 核心机制：`LaunchConfig` 如何锁死 Kernel 与 `args` 的类型关系？

现在来看第二个关键问题：当我们在宿主端启动 `add_kernel` 时，怎么保证传入的 `args` 元组跟 `add_kernel` 声明的形参列表 `(left: *f32, right: *f32, output: *f32)` 一分不差？

请看这张**先行类型接口（Upfront Type Interface）与启动配置匹配图**：

```
+-------------------------------------------------------------------------+
|            投影入口 F 与启动实参包 A 的编译期静态绑定机制                  |
+-------------------------------------------------------------------------+
|                                                                         |
|  1. 投影协议提取先行接口 (`projection_interface`)：                      |
|     `add_kernel` 的签名 `(left: *f32, right: *f32, output: *f32)`       |
|     ──► 生成类型描述，其身份 F 匹配模式：                                 |
|         `cuda.Kernel<Tuple<*f32, *f32, *f32>, Any, any>`                |
|                      ───────────┬───────────                            |
|                                 │ 提取出参数包类型 A                     |
|                                 ▼                                       |
|  2. 启动配置结构体约束 (`LaunchConfig<F, A>`)：                          |
|     where F, A, requires F matches cuda.Kernel<A, Any, any>             |
|     • 字段 `kernel: *F`                                                 |
|     • 字段 `args: A`  <-- 强制要求 args 的类型必须严格等于提取出的 A！    |
+-------------------------------------------------------------------------+
```

注意看约束 `requires F matches cuda.Kernel<A, Any, any>` 中的精妙之处（结合我们在第 079、083 回学过的知识）：
- 这里的 `A` 捕获了该 Kernel 要求的参数包类型；
- 后面两个槽位的 `Any` 和 `any` 只是用来通配 Kernel 类型描述中其他与参数包无关的元数据槽位——**它们绝不会把 `F` 变成一个可以随便乱传参的动态对象，也绝不会削弱 `A` 对 `args: A` 的严格静态约束**！

让我们写出这段教学版 `LaunchConfig`（源自 `docs/pipeline-projection/declarations-and-interfaces.md`）：

```flurry
use cuda;

-- 通过 `requires F matches cuda.Kernel<A, Any, any>`，
-- 将 kernel 入口类型 F 的参数描述 A 与配置中的 args: A 静态绑定！
struct LaunchConfig
where
    F,
    A,
    requires F matches cuda.Kernel<A, Any, any>
{
    kernel: *F,
    grid_dim: Tuple<u32, u32, u32>,
    block_dim: Tuple<u32, u32, u32>,
    args: A,
    shared_mem_bytes: u32 = 0,
}

fn take_arguments(config: LaunchConfig<F, A>) -> A
where
    F,
    A,
    requires F matches cuda.Kernel<A, Any, any>
{
    return config.args;
}
```

如果调用者在构造 `LaunchConfig` 或调用 `cuda.launch_kernel` 时，传错了 `args` 的类型（比如把第 3 个参数从 `*f32` 传成了 `*i32`，或者少传了一个参数），根本不需要启动 GPU 后端，宿主端的类型检查器当场就会报错拦截：

```flurry
-- ❌ 编译期直接拒绝！add_kernel 的参数包类型 A 为 Tuple<*f32, *f32, *f32>，
-- 传入只含两个元素的元组 (left, right) 违反了 `F matches cuda.Kernel<A, Any, any>` 约束！
unsafe fn reject_mismatched_launch_args(
    left: *f32,
    right: *f32,
) -> !CudaErr void {
    cuda.launch_kernel {
        kernel: add_kernel,
        gridDim: (1u32, 1u32, 1u32),
        blockDim: (1u32, 1u32, 1u32),
        args: (left, right),
    }!;
}
```

> **💡 核心心智模型：接口查询与目标代码生成解耦！**
> 在编译器内部，`projection_interface(declaration)` 只需要读取 `add_kernel` 的签名、属性和必要的编译期输入，就能立刻算出它的公开类型接口供宿主检查 `launch_kernel`；而真正耗时的设备代码生成 `generate_projection(declaration)` 则可以通过依赖图并行或延后调度，且生成结果必须通过 `check_delivery_matches(interface, result)` 核验，**绝不允许后端在生成代码后反过来篡改已经向宿主承诺的类型接口**！

---

## 三、 `v2` 进阶：宿主的 `build.target` vs 设备包的 `build.package_metadata`

在编写跨 CPU 与 GPU 的异构代码时，还有一个极易踩坑的配置读取问题（`AGENTS.md` §5 & `docs/pipeline-projection/declarations-and-interfaces.md`）：

假设我们在编译期想知道当前目标的硬件架构。
- **什么是 `build.target`？** `build` 内置包中的内容**永远默认描述 Host Pipeline（宿主程序编译管线）**！（顺便注意：Host Pipeline 指的是所构建的宿主程序面向的目标平台，而不是“运行编译器 binary 的这台开发机”）。
- **给函数标上 `^cuda.global`，会让它内部的 `build.target` 自动变成 GPU 架构吗？**
  **绝对不会！** 投影到 CUDA 等其他管线，绝不会隐式篡改 `build.target` 的含义；一个普通 Helper 函数被 Kernel 调用或内联，也绝不会因此去读取另一套动态变化的全局 `build` 对象！
- **那怎么读取 CUDA 设备管线自己的目标配置（比如 SM 架构版本 `TargetArch`）？**
  必须**显式查询对应包的元数据：`build.package_metadata("cuda").features`**！

```
  [ 宿主管线配置查询 ]
  build.target.arch  ──► 永远描述 Host Pipeline（如 .x86_64 或 .aarch64）

  [ 设备管线配置查询 ]
  build.package_metadata("cuda").features.get(.target_arch, cuda.config.TargetArch)
                     ──► 显式按 CUDA 库定义的类型读取设备目标配置 `?TargetArch`！
```

来看规范中的标准读取方式：

```flurry
use cuda;

comptime {
    -- 显式读取 "cuda" 包的编译期配置元数据，绝不污染或篡改默认 build.target！
    const features = build.package_metadata("cuda").features;
    const arch: ?cuda.config.TargetArch =
        features.get(.target_arch, cuda.config.TargetArch);
}
```

注意看 `arch` 的类型是可选类型 **`?cuda.config.TargetArch`**：
如果工程配置里没有指定 `.target_arch`，查询得到的是 `null`。此时究竟是报错要求用户显式指定，还是采用某个默认架构，完全由 `cuda` 库的接口契约决定——**Flurry 编译器绝不会自作聪明地去探测当前开发机插了什么显卡来悄悄补齐这个值**（否则会彻底破坏跨机器构建的可重现性与缓存一致性）！

---

## 四、 本篇小结

| 契约维度 | 规则与行为 | 防范的工程风险 |
| :--- | :--- | :--- |
| **普通 Helper 复用** | Kernel 调用的普通函数无需重复标注 `^cuda.global` | 消除 `__device__` 标签传染，同一 Helper 可同时服务宿主与设备 |
| **先行接口与生成分离** | `projection_interface` 仅依赖签名与属性即可完成宿主检查 | 避免宿主检查阻塞在设备代码生成上，且严禁后端事后篡改接口 |
| **Kernel 与 `args` 绑定** | `requires F matches cuda.Kernel<A, Any, any>` | 在编译期静态锁死启动参数包 `args: A` 的元素个数与类型 |
| **`build.target` 语义** | 默认固定描述 Host Pipeline，不因投影属性而隐式改变 | 防止 Helper 函数在不同调用点读到分裂的全局 `build.target` |
| **设备配置读取** | 显式调用 `build.package_metadata("cuda").features.get(...)` | 明确记录包配置依赖指纹，缺省返回 `null` 而非偷取开发机硬件信息 |

---

## 五、 留给下一篇的悬念

现在，我们已经能把函数投射到 GPU 或 SIMD 管线，也能通过 `LaunchConfig` 安全地向它传递参数了。

但是，一旦进入到设备 Kernel 内部，或者进入到 CPU 的极致性能算子内部，光靠普通的 `+ - * /` 算术运算符是不够的！我们需要直接调用目标硬件的**底层内建指令（Hardware Intrinsics）**——比如：
- 单指令融合乘加（FMA：`a * b + c` 单次舍入）；
- GPU 线程块同步屏障（Barrier：`__syncthreads()`）；
- 将一个 64 位机器字单指令拆分成高低两个 32 位字。

在其他语言里，你往往要面对几千个像 `__builtin_ia32_vfmaddps256` 这样丑陋的编译器特权内置函数名，或者写极度危险的字符串内联汇编。
而在 Flurry 中，调用硬件 Intrinsic 居然没有增加任何新关键字，而是巧妙地复用了我们已经无比熟悉的 **视图（Take View `'intrinsic`）、函数调用与类型转换后缀 `.as(T)`**！

下一回 **《Flurry 演进手记 (173)：直接调用底层硬件指令——expr'intrinsic(args).as(T) 与输出类型安全契约》**，我们将揭开硬件内建指令与 `math` 泛型包装的奥秘！
