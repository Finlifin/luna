# Flurry 演进手记 (173)：直接调用底层硬件指令——expr'intrinsic(args).as(T) 与输出类型安全契约

在上一回（172）中，我们学会了如何声明一个投射到设备管线的 Kernel，以及如何让普通 Helper 函数跟随入口一起进入目标管线。

当我们深入到设备 Kernel 或高性能数学库（如 `math` 包）的底层实现时，不可避免地要与硬件指令集正面交锋：
- 在科学计算中，我们需要直接触发硬件的**单指令融合乘加（FMA，Fused Multiply-Add）**，既把两次浮点运算合成一条指令，又避免中间步骤的二次舍入误差；
- 在 GPU 共享内存协作中，我们需要发出**硬件线程块屏障（Barrier）**指令；
- 在底层字操作中，我们需要调用目标管线特有的多结果拆分指令。

如果让你为 Flurry 设计硬件内建指令（Intrinsic）的语法，你会怎么设计？
今天，我们就来解读 `docs/pipeline-projection/intrinsics.md` 与 `AGENTS.md` §3，看看 Flurry 是如何做到**不增加任何一个新关键字、不引入易碎的指令字符串语法，仅凭 `'intrinsic` 视图与 `.as(T)` 就把开放硬件指令关进强类型契约笼子**的！

---

## 一、 破局与痛点：为什么拒绝“魔法函数名字符串”与“特权关键字”？

在 C/C++ 或传统编译器中，暴露硬件内建指令通常有两种做法（`v0` 痛点）：

1. **魔法全局符号洪流**：编译器在全局命名空间里塞进成千上万个以 `__builtin_...` 或 `__nv_...` 开头的特殊函数。这不仅污染了符号表，而且对于支持开放操作数（如可变宽度向量或目标专属寄存器组合）的管线指令，固定函数签名根本表达不过来！
2. **弱类型字符串调用**：有些语言干脆搞一个 `asm("fma.rn.f32 ...", a, b, c)`，把指令名写成字符串字面量。一旦拼错一个字母，或者两个不同硬件包里有同名指令，连静态作用域和 IDE 跳转都彻底瘫痪！

Flurry 的解法堪称教科书级的“正交复用”：
1. **指令身份用枚举（`^intrinsic_enum`）表示**：每一族硬件指令都是一个标注了 `^intrinsic_enum` 的正经 Flurry `enum`！它的每个变体（如 `Inst.fma`、`Inst.barrier`）都拥有独立的名义类型身份，支持正常的 `use` 导入、模块可见性与 IDE 补全！
2. **调用语法复用 Take View + Application + `.as(T)`**：在语法层面（`docs/grammar/syntax` 第 165–168 行），调用一条硬件指令直接写作：

$$\text{typed\_intrinsic\_application} \rightarrow expr\text{\texttt{'intrinsic}}(argument^*)\text{\texttt{.as}}(expr)$$

---

## 二、 核心心智模型：`^intrinsic_enum` 与开放输入 / 显式输出边界

让我们用一张图看清一次 `Inst.fma'intrinsic(a, b, c).as(f32)` 调用中，**通用语言前端（Frontend）**与**目标编译管线（Target Pipeline）**是如何分工协作的：

```
+-----------------------------------------------------------------------------+
|           Inst.fma 'intrinsic (a, b, c) .as(f32) 的职责分界线                |
+-----------------------------------------------------------------------------+
|                                                                             |
|  1. 指令标签：`Inst.fma` (来自 `^intrinsic_enum enum Inst`)                  |
|     • 名义身份：不同枚举里的同名 `.fma` 绝不会混淆！                         |
|                                                                             |
|  2. 转换视图与开放实参：`'intrinsic(a, b, c)`                                |
|     • 通用前端责任：检查 `unsafe` 权限，按源码顺序求值实参 `a, b, c`，        |
|       确保变量存在、编译期计算已完成。                                       |
|     • 目标管线责任：操作数接口对前端开放；由具体 Pipeline 核验 `fma` 是否     |
|       恰好接受 3 个 `f32` 操作数以及是否满足目标硬件能力！                   |
|                                                                             |
|  3. 显式输出类型接口：`.as(f32)` (若省略则默认为 `void`！)                    |
|     • 通用前端责任：无需等待后端跑完，外围表达式当场获知该操作返回 `f32`！    |
|     • 目标管线责任：核验 `Inst.fma` 确实能够交付 `f32` 表示！                 |
+-----------------------------------------------------------------------------+
```

### 2.1 为什么说“`math` 中提供的函数是 LLVM intrinsic 的泛型包装”？

在 `AGENTS.md` §3 中有一条重要的跨角色协作约定：**“`math` 中提供的函数是 LLVM intrinsic 的泛型包装，库接口与编译器接线按此约定协调。”**

理解了上面的分工图，你就瞬间明白为什么标准库 `math` 要这样设计了：
- 直接调 `'intrinsic` 是**开放输入且要求 `unsafe`** 的——因为通用前端不替每条硬件指令硬编码操作数个数检查；
- 而标准库 `math`（或像下面的 `fused_multiply_add` 包装函数）在外部提供了一层**带有明确泛型约束、确定参数个数和清晰安全契约的 Flurry 函数签名**，在内部封装 `unsafe` 的 `'intrinsic` 调用！这样业务程序员调用 `math.fma(a, b, c)` 时既享受 100% 的安全类型提示，底层又能零损耗地直通 LLVM / 硬件 Intrinsic！

---

## 三、 `v1` 实战：有结果的 `fma`、无结果的 `barrier` 与多步串联

让我们把 `docs/pipeline-projection/intrinsics.md` 中的经典指令族 `Inst` 写出来，对比有返回值的算术指令 `fma` 与无返回值的同步屏障指令 `barrier`：

```flurry
-- 1. 用 ^intrinsic_enum 定义教学指令族枚举
^intrinsic_enum
enum Inst {
    fma,
    barrier,
}

-- 2. 有结果的硬件指令：紧接 `.as(f32)` 声明该操作向外围交付 f32 结果
unsafe fn fused_multiply_add(a: f32, b: f32, c: f32) -> f32 {
    return Inst.fma'intrinsic(a, b, c).as(f32);
}

-- 3. 无结果的硬件指令：不接 `.as(...)`，该调用的类型即为 void
unsafe fn wait_group() {
    Inst.barrier'intrinsic(0u16);
}

-- 4. 多步串联：前一条 intrinsic 的显式 `.as(f32)` 让 `first` 立即获得确定的 f32 类型，
--    可直接作为下一条 intrinsic 的输入！
unsafe fn two_steps(a: f32, b: f32, c: f32, d: f32) -> f32 {
    let first = Inst.fma'intrinsic(a, b, c).as(f32);
    return Inst.fma'intrinsic(first, d, c).as(f32);
}

test "hardware intrinsics fma and two_steps v1" {
    let single = unsafe {
        fused_multiply_add(3.0f32, 4.0f32, 5.0f32)
    };
    -- 3.0 * 4.0 + 5.0 = 17.0
    assert(single == 17.0f32);

    let chained = unsafe {
        two_steps(2.0f32, 3.0f32, 1.0f32, 4.0f32)
    };
    -- first = 2.0 * 3.0 + 1.0 = 7.0; second = 7.0 * 4.0 + 1.0 = 29.0
    assert(chained == 29.0f32);
}
```

> **💡 核心心智模型：返回 `void` 绝不代表“没有副作用、可以被优化器删掉”！**
> 请看上面的 `Inst.barrier'intrinsic(0u16);`：它没有写 `.as(...)`，因此它的表达式结果类型是 `void`。
> 但 `docs/pipeline-projection/intrinsics.md` 明确规定：**“输出类型不描述行为。void 操作可以写内存、同步或改变可观察状态。结果未被使用的操作也可以有副作用。通用优化不能仅凭返回 void、调用位于 unsafe 或结果没有使用，就删除、复制或重排操作。”**

---

## 四、 `v2` 进阶：交付聚合结果（`Tuple`）与开放输入的目标拦截

如果某条底层指令一次会产出两个寄存器结果（比如把一个 64 位无符号整数 `u64` 拆成低 32 位和高 32 位两个 `u32`），Flurry 需要专门为“多结果指令”发明一套特殊语法吗？

**完全不需要！** 直接在 `.as(...)` 里指定该操作协议支持的聚合类型（如 `Tuple<u32, u32>`）即可：

```flurry
^intrinsic_enum
enum WordInst {
    split_word,
}

-- 通过 `.as(Tuple<u32, u32>)` 接收多结果指令交付的低/高 32 位元组！
unsafe fn split_word(value: u64) -> Tuple<u32, u32> {
    return WordInst.split_word'intrinsic(value).as(Tuple<u32, u32>);
}
```

同时，千万不要误以为“Intrinsic 的输入接口对通用前端开放”就意味着你可以乱传参数而不被发现！
一旦进入目标管线的 **目标验证（Target Validation）** 阶段，管线会严格核对每条指令标签的操作数个数、操作数类型表示以及输出类型。如果给需要 3 个 `f32` 的 `Inst.fma` 只传了 2 个参数，目标检查会当场报错拒绝：

```flurry
-- ❌ 编译期（目标检查阶段）直接拒绝！Inst.fma 契约要求 3 个 f32 操作数，仅传入 2 个操作数非法
unsafe fn reject_missing_intrinsic_operand(a: f32, b: f32) -> f32 {
    return Inst.fma'intrinsic(a, b).as(f32);
}
```

---

## 五、 本篇小结

| 特性维度 | 语法与机制 | 核心语义契约 |
| :--- | :--- | :--- |
| **指令集声明** | `^intrinsic_enum enum Inst { ... }` | 每个枚举变体拥有独立名义身份，同名不同枚举绝不混淆 |
| **无结果指令调用** | ` tag'intrinsic(args) ` | 要求 `unsafe` 权限；外围类型为 `void`，优化器严禁无故删除或重排 |
| **有结果指令调用** | ` tag'intrinsic(args).as(T) ` | 向目标 Pipeline 请求交付类型 `T`（支持标量、向量或 `Tuple` 等聚合类型） |
| **标准库 `math` 定位** | 泛型安全函数包装内部 `'intrinsic` | 对外提供强类型安全签名，对内直通 LLVM / 目标硬件 Intrinsic |

---

## 六、 留给下一篇的悬念

在阅读本篇代码时，有一个极其关键的细节值得我们单独拿出一整篇来彻底讲透：
在 `Inst.fma'intrinsic(a, b, c).as(f32)` 中，最后那个 **`.as(f32)`** 长得跟普通数值转换 `x.as(f32)` 一模一样！

这很容易引发两个致命的误解：
1. **误解一**：“既然我已经写了 `let result: f32 = Inst.fma'intrinsic(a, b, c);`，左边变量已经标了 `: f32`，外层函数的返回类型也标了 `-> f32`，我是不是就可以偷懒省掉后面的 `.as(f32)` 了？”
2. **误解二**：“既然 `.as(T)` 是类型转换，那如果 `Inst.fma` 本来产生 `f32`，我写成 `Inst.fma'intrinsic(a, b, c).as(u32)`，它是不是就会自动先算出 `f32` 再帮我把浮点数转换成整数 `u32`？”

这两个直觉**全都大错特错**！
下一回 **《Flurry 演进手记 (174)：为什么 Intrinsic 省略 .as(T) 默认是 void——拒绝隐式返回类型猜测》**，我们将深入解析 `docs/grammar/syntax` 第 168 行的铁律，彻底看清紧接 Intrinsic 的 `.as(T)` 与普通类型转换的本质区别！
