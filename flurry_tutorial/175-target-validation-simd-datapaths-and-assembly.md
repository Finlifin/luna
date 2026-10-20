# Flurry 演进手记 (175)：把 SIMD 向量数据路径关进安全笼子——目标约束检查、Helper 闭包与内联汇编封装

在[上一篇（第 174 篇）](./174-why-intrinsics-require-explicit-as-output.md)中，我们拆解了为什么 `'intrinsic(args).as(T)` 必须把显式类型契约钉在每一个调用点。

但在真实的高性能信号处理与张量引擎里，一个硬件加速算子绝不仅仅是一条孤零零的指令调用。
今天，我们要编写两条工业级硬件加速原语：
1. 一个在 GPU 与通用计算单元之间复用的核函数 `write_sum`，它内部调用了普通辅助函数 `add_values`；
2. 一条面向 `x86_64` 的 **FMA（融合乘加）** 原语，以及一条完整的 **AVX2 8 路单精度浮点向量数据路径（SIMD Datapath）**——把两块连续的 8 元素数组加载进 256-bit 向量寄存器、执行并行加法、再写回内存。

在这个过程中，我们将直面目标验证（Target Validation）的三大核心工程问题：
- 当被投射的入口函数调用了普通辅助函数（Helper）时，目标管线检查的范围到底有多大？如果某个泛型辅助函数以不同的类型参数被实例化了两次，检查器会不会漏掉第二次？
- 在 AVX2 向量数据路径中，像 `_mm256_storeu_ps` 这种没有返回值（返回 `void`）的内存写出指令，用不用写 `.as(...)`？如果传错了向量类型表示，谁来拦截？
- 每次调用底层指令都要写 `unsafe { X86Instructions.fma'intrinsic(a, b, c).as(f32) }` 太啰嗦了，我们该用 `quote fn` 还是 `inline fn` 把它关进安全笼子？

---

## 一、 传统的陷阱：只检查入口函数表面，还是把运行时死分支当成豁免凭证？

假设一位习惯了传统脚本或松散宏系统的工程师来设计目标管线的合法性检查器。他很容易犯下两个致命错误：

### 错误假想 1：只检查带 `^pipeline` 属性的入口函数本身，或者只按“函数名”去重
看下面这段完全合法的跨设备复用代码：

```flurry
use cuda;

fn add_values(a: i32, b: i32) -> i32 {
    a + b
}

^cuda.pipelines.global
fn write_sum(out: *i32, a: i32, b: i32) -> void {
    unsafe {
        out.* = add_values(a, b);
    }
}
```

请注意：`add_values` 是一个**没有**挂载 `^cuda.pipelines.global` 属性的普通函数。
如果编译器只检查 `write_sum` 的表面语法，而不顺着调用图把 `add_values` 納入检查范围，那么一旦有人在 `add_values` 里面偷偷写了一条宿主机专用的系统调用或 AVX2 指令，编译期就会漏网，直到 PTX 汇编器崩溃才报错。

更隐蔽的是：**如果 `add_values<T>` 是一个泛型函数**，在同一个 Kernel 里先被调用了 `add_values<i32>`（目标支持），后被调用了 `add_values<HostOnlySocket>`（目标不支持）。如果检查器的 `visited` 集合只按函数声明的名字 `"add_values"` 去重，第二次调用就会被直接跳过。

### 错误假想 2：以为运行时 `if false` 里的非法硬件指令可以被放过
有些工程师会写这样的代码：

```flurry
-- ❌ 极其危险的妄想：试图用运行期条件分支包裹非目标架构指令
fn broken_runtime_branch(flag: bool, a: f32, b: f32, c: f32) -> f32 {
    if false {
        -- “反正运行期永远走不到这里，编译器在 ARM 上别报错行不行？”
        unsafe { std.asm.x86.X86Instructions.fma'intrinsic(a, b, c).as(f32) }
    } else {
        a * b + c
    }
}
```

**绝对不行。** 运行期条件 `if cond` 的两个分支都会进入当前目标的执行依赖闭包（Execution Dependency Closure）。因为编译器在把函数降级为目标机器码时，必须为所有运行期可达的语法分支生成合法的目标指令；目标能力检查是一道严肃的编译期契约，绝不依赖后端的死代码消除（DCE）来侥幸逃脱。

---

## 二、 升级：基于具体实例（Concrete Instance）的目标依赖闭包

在 Flurry 中，当一个入口函数被投射到目标管线（如 `^cuda.pipelines.global` 或当前宿主目标）时，编译器会以该入口的**具体实例（Concrete Instance）**为起点，启动一个严密的工作列表（Worklist）算法，验证整棵**必需执行依赖闭包（Required Dependency Closure）**：

```text
               ┌──────────────────────────────────────────────────┐
               │   Entry Instance: (write_sum, subst=∅, target)   │
               └────────────────────────┬─────────────────────────┘
                                        │ 加入 pending 队列
                                        ▼
               ┌──────────────────────────────────────────────────┐
               │          Worklist 循环 (pending -> visited)       │
               │                                                  │
               │  唯一身份键 (Instance Identity):                 │
               │    (DeclId, ConcreteSubstitutions, TargetConfig) │
               ├──────────────────────────────────────────────────┤
               │  1. 检查当前实例签名与函数体中的类型表示布局       │
               │  2. 检查所有直接使用的硬件 Intrinsics / 特殊操作 │
               │  3. 收集运行期 if/match 所有分支调用的 Helper 实例 │
               │     (注：inline if 淘汰的分支不进入执行闭包  )     │
               └────────────────────────┬─────────────────────────┘
                                        │ 发现未访问的具体实例
                                        ▼
               ┌──────────────────────────────────────────────────┐
               │  Helper Instance: (add_values, subst=∅, target)  │
               └──────────────────────────────────────────────────┘
```

> **💡 核心心智模型：区分 `inline if`（编译期裁剪）与普通 `if`（运行期分支）**
> - **普通 `if` / `match`**：所有分支都属于目标执行代码的一部分，里面的每一个 Helper、类型表示和 Intrinsic 都必须通过当前目标的合法性验证。
> - **`inline if` / `inline match`**：条件在编译期求值。**未被选中的分支不会进入目标执行代码，也不会把其中的平台专属 Intrinsic 拉进当前目标的执行依赖闭包**（但注意：未选中的分支依然要通过前期的语法解析和基础名称/类型结构检查，不能塞入语法垃圾）。

---

## 三、 实战：构建受目标守卫保护的 `x86_64` FMA 与 AVX2 向量数据路径

掌握了 `inline if` 与目标验证闭包的配合后，我们就可以写出既严密又零开销的硬件加速代码了。

### 1. 用 `inline if` + `build.target` 守卫 `x86_64` FMA 指令

我们先看单条指令级的融合乘加 `fused_multiply_add`（源自 `docs/pipeline-projection/target-validation-and-assembly.md`）：

```flurry
use std.asm.x86.X86Instructions;

fn fused_multiply_add(a: f32, b: f32, c: f32) -> f32 {
    -- 1. 编译期架构守卫：如果不是 x86_64，直接通过编译期诊断拒绝
    inline if build.target.arch != .x86_64 {
        meta.compilation.err("fused_multiply_add requires an x86_64 target");
    }

    -- 2. 在 unsafe 块中发起带显式 .as(f32) 的底层指令调用
    unsafe {
        X86Instructions.fma'intrinsic(a, b, c).as(f32)
    }
}
```

### 2. 串联完整的 AVX2 256-bit SIMD 向量数据路径

接下来是重头戏：我们要用 AVX2 的 `_mm256_loadu_ps`、`_mm256_add_ps` 和 `_mm256_storeu_ps`，把两个 `*Array<f32, 8>` 相加并写回第三个数组指针 `zs`。

请特别观察这四条指令在**有无返回值**与**副作用保持**上的区别：

```flurry
use std.asm.x86.{
    _mm256_add_ps,
    _mm256_loadu_ps,
    _mm256_storeu_ps,
}

fn add_arrays_avx2(
    comptime Vector: Type,
    xs: *Array<f32, 8>,
    ys: *Array<f32, 8>,
    zs: *Array<f32, 8>,
) -> void {
    -- 1. 同时检查目标架构 (.x86_64) 与目标指令集特性 (.avx2)
    inline if build.target.arch != .x86_64
        or not (build.target.features.get(.avx2) matches true?)
    {
        meta.compilation.err("add_arrays_avx2 requires x86_64 with avx2");
    }

    -- 2. 向量寄存器数据流：Load -> Load -> Add -> Store
    unsafe {
        -- 有值产出的指令：必须紧跟 .as(Vector) 固定 256-bit 向量寄存器类型
        let left = _mm256_loadu_ps'intrinsic(xs).as(Vector);
        let right = _mm256_loadu_ps'intrinsic(ys).as(Vector);
        let sum = _mm256_add_ps'intrinsic(left, right).as(Vector);

        -- 无返回值的存储指令：直接返回 void，不需要也不允许写 .as(...)。
        _mm256_storeu_ps'intrinsic(zs, sum);
    }
}
```

```text
  内存输入 xs: *Array<f32, 8>          内存输入 ys: *Array<f32, 8>
              │                                    │
              ▼ _mm256_loadu_ps'intrinsic(xs)      ▼ _mm256_loadu_ps'intrinsic(ys)
         .as(Vector)                          .as(Vector)
              │                                    │
              ▼                                    ▼
      ┌───────────────┐                    ┌───────────────┐
      │ left : Vector │                    │ right: Vector │ (256-bit YMM 寄存器视图)
      └───────┬───────┘                    └───────┬───────┘
              └──────────────┐      ┌──────────────┘
                             ▼      ▼
                  _mm256_add_ps'intrinsic(left, right)
                            .as(Vector)
                                 │
                                 ▼
                         ┌───────────────┐
                         │  sum : Vector │
                         └───────┬───────┘
                                 │
                                 ▼ _mm256_storeu_ps'intrinsic(zs, sum)
                    内存输出 zs: *Array<f32, 8>  (返回 void，具内存写副作用，禁删。)
```

> **🔍 语法与语义小贴士：为什么 `_mm256_storeu_ps'intrinsic(zs, sum)` 不能被优化器删掉？**
> 1. **关于 `.as(T)`**：`_mm256_loadu_ps` 和 `_mm256_add_ps` 产出硬件寄存器中的向量值，因此属于“按输出类型多态的指令”，必须跟 `.as(Vector)`；而 `_mm256_storeu_ps` 的职责是把 `sum` 写进 `zs` 指向的内存，输出类型固定为 `void`，所以直接调用即可。
> 2. **关于副作用（Side Effects）**：虽然 `_mm256_storeu_ps` 返回 `void`，但它绝不是无意义的空操作。编译器会严格保留它的内存写出副作用与求值顺序，绝不会因为“没有使用返回值”而把它当成死代码删掉。

### 3. 反例警告：把普通标量 `u32` 冒充为 256-bit 向量类型 `Vector`

既然 `add_arrays_avx2` 把 `comptime Vector: Type` 作为参数传入，如果调用者胡乱传一个 `u32` 进去，试图把 256-bit 的加载结果塞进 32-bit 标量里，会发生什么？

```flurry
-- ❌ 编译期直接拒绝。
unsafe fn reject_invalid_avx2_vector_repr(
    xs: *Array<f32, 8>,
    ys: *Array<f32, 8>,
    zs: *Array<f32, 8>,
) -> void {
    -- 试图用 32 位标量 u32 充当 256 位 AVX2 向量类型：
    add_arrays_avx2(u32, xs, ys, zs);
    --              ^^^
    -- 编译期直接拒绝（Target / Intrinsic Representation Error）：
    -- 在实例化 add_arrays_avx2(u32, ...) 时，目标与 Intrinsic 验证器检查到
    -- _mm256_loadu_ps 和 _mm256_add_ps 要求与 8x f32 (256-bit) 兼容的向量表示，
    -- 标量 `u32` 不满足该硬件指令的输入/输出表示契约。
}
```

---

## 四、 把 `unsafe` 关进笼子：`quote fn` 与 `inline fn` 的分工

在业务代码里，我们肯定不希望每个调用者都手写 `unsafe { X86Instructions.fma'intrinsic(a, b, c).as(f32) }`。我们想提供一个安全、干净的封装函数。
这时候，Flurry 提供了两种常见的封装手段：`quote fn` 和 `inline fn`。它们有什么本质区别？

```flurry
use std.asm.x86.X86Instructions;

-- 方式 A：使用 quote fn 进行确定性的调用点展开
quote fn fused_multiply_add_expanded(a: f32, b: f32, c: f32) -> f32 {
    unsafe {
        X86Instructions.fma'intrinsic(a, b, c).as(f32)
    }
}

-- 方式 B：使用 inline fn 表达普通函数的内联意图
inline fn fused_multiply_add_inlined(a: f32, b: f32, c: f32) -> f32 {
    unsafe {
        X86Instructions.fma'intrinsic(a, b, c).as(f32)
    }
}
```

让我们回忆并巩固 目标约束与内建指令的核心规则：
1. **`quote fn` 不属于 `comptime fn`，普通实参依然只求值一次**：当你调用 `fused_multiply_add_expanded(calc_a(), calc_b(), calc_c())` 时，三个实参表达式在调用点被正常求值一次，随后函数体确定性地在调用点展开。
2. **`inline fn` 是带内联提示的普通函数**：它表达了向调用点内联的优化意图，但语义上依然是一个普通函数定义。
3. **两者都不能用来“绕过”目标验证**：无论你用 `quote fn` 还是 `inline fn` 把 `unsafe` 和 `'intrinsic` 包起来，展开或调用后的指令依然处于调用者的目标执行依赖闭包中。如果当前目标不是 `x86_64`，目标验证器照样会在编译期精准拦截。

---

## 本篇小结

| 工程维度 | Flurry 的编译期契约与机制 | 解决的核心问题 |
| :--- | :--- | :--- |
| **目标依赖闭包 (`pending / visited`)** | 从投射入口的具体实例出发，递归验证所有可达的普通 Helper、类型表示与 Intrinsic | 无需给每个内部小辅助函数重复贴 `^pipeline` 属性，同时杜绝漏检 |
| **实例唯一标识 (Instance Identity)** | 包含 `(声明身份, 具体泛型替换/编译期实参, 目标配置)` | 防止泛型辅助函数在不同类型实例化下因为“同名”而跳过二次验证 |
| **条件分支与目标验证** | 普通运行期 `if` 的所有分支都必须通过目标检查；`inline if` 裁剪掉的分支不进入执行闭包 | 严禁依赖后端死代码消除逃避架构检查，同时支持零成本跨平台条件编译 |
| **SIMD 数据路径 (`loadu` / `add` / `storeu`)** | 产出值的指令必须接 `.as(Vector)`；返回 `void` 的 `storeu` 不接 `.as` 且严格保留内存写副作用 | 向量寄存器位宽在每一步都受静态约束，写出操作绝不会被优化器误删 |
| **安全封装 (`quote fn` vs `inline fn`)** | `quote fn` 提供实参单次求值的确定性展开；`inline fn` 提供内联意图，将 `unsafe` 收敛在高层原语内部 | 上层业务享受零开销的安全函数接口，底层硬件约束丝毫不打折 |

---

## 下一篇预告

到现在为止，我们已经走完了管线投射的前半程：
声明 `^cuda.pipelines.global` -> 校验跨边界类型与引用 -> 固定硬件 Intrinsic 的 `.as(T)` 输出 -> 验证整棵目标执行依赖闭包。

但是，当编译器前端完成了这一系列检查、完成了类型检查与目标约束验证之后，**宿主机上的编译任务是如何通过 Pipeline 回调把待编译函数交给 `cuda_compile` 后端去生成设备代码的？**
- 为什么 `pub pure comptime fn global(ir: meta.compilation.IIR.Func) -> void` 的返回类型是 `void`？生成的机器码去哪儿了？
- 那个神秘的 `meta.compilation.IIR.Func` 句柄（Handle）为什么绝对不能被缓存到任务之外？
- 如果构建中途崩溃，或者磁盘上的 `.cubin` 产物被意外删掉，增量构建缓存（Artifact Cache）如何保证绝不发布半成品？

下一篇 **[Flurry 演进手记 (176)：宿主后端与设备产物交付——Pipeline 回调、受控句柄与缓存恢复](./176-host-backends-iir-handles-and-artifact-delivery.md)**，我们将探讨多后端编译任务的产物交付与状态管理机制。
