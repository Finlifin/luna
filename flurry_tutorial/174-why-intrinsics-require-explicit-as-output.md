# Flurry 演进手记 (174)：为什么 Intrinsic 省略 .as(T) 默认是 void——拒绝隐式返回类型猜测

在上一回（173）中，我们学会了用 `Inst.fma'intrinsic(a, b, c).as(f32)` 调用底层硬件内建指令，并在文末抛出了两个极易踩坑的直觉误区：
1. 如果外围已经有了明确的类型注解（比如 `let result: f32 = ...` 或函数返回值 `-> f32`），为什么省略 `.as(f32)` 写 `Inst.fma'intrinsic(a, b, c)` 会当场报错？
2. 为什么写 `Inst.fma'intrinsic(a, b, c).as(u32)` 不会像普通表达式那样“先算出 `f32` 再转成 `u32`”，而是会被目标管线直接拒绝？

在 Flurry 的全局语法规范（`docs/grammar/syntax` 第 166–168 行）中，专门用注释钉死了这条铁律：
```text
intrinsic_application -> expr'intrinsic(argument*)
typed_intrinsic_application -> intrinsic_application.as(expr)
-- 紧接 intrinsic 调用的 as 指定输出类型；缺省为 void，外围期望类型不替代它。
```
今天，我们就来拆解这条语法与 Elaboration 规则背后的深层设计哲学，彻底搞懂**“紧接 Intrinsic 的 `.as(T)`”**与**“普通表达式的 `.as(T)`”**在编译器内部有何天壤之别！

---

## 一、 破局与痛点：如果允许靠“外围上下文”去猜 Intrinsic 的返回类型会怎样？

假设你是编译器设计师。在面对一条开放接口的硬件指令 `Inst.load_vec'intrinsic(ptr)` 时，如果在它本身不带 `.as(T)` 的情况下，你试图让类型推导器去根据“外围上下文”猜测它的输出类型（`v0` 设想），会发生什么灾难？

```
  [ ❌ v0 灾难场景：试图靠外围上下文推导 Intrinsic 输出类型 ]
  unsafe fn ambiguous_pipeline(ptr: *f32) {
      -- 痛点 1：当作为中间操作数传给泛型函数或另一个开放输入的 intrinsic 时：
      let x = Inst.load_vec'intrinsic(ptr);
      Inst.shuffle'intrinsic(x, 0);
      -- 糟糕！`x` 没有任何类型注解，而 `shuffle'intrinsic` 的输入也是开放的！
      -- 此时 `x` 到底是 128 位向量 Vec4f、256 位向量 Vec8f，还是单个 f32？
      -- 类型检查器在前端直接卡死在“未决推导元变量 ?T”上！

      -- 痛点 2：如果丢弃了返回值没用：
      Inst.atomic_add'intrinsic(ptr, 1.0f32);
      -- 此时外围期望是语句位置的 void，难道同一个调用换个位置就偷偷改变指令形态吗？
  }
```

看明白了吗？
因为 Intrinsic 的本质是**把操作解释权交给后续的目标 Pipeline**，而外围 Flurry 程序在前端 Elaboration 阶段就必须立刻确定每一个表达式的静态类型（否则连后续的方法查找、重载消解和 IIR 构建都无法推进）！
如果允许省略 `.as(T)` 去靠外围上下文反向猜测，一旦遇到 `let first = Inst.fma'intrinsic(a, b, c); Inst.fma'intrinsic(first, d, c)` 这样中间变量连续传递的场景，外围根本提供不了唯一起点类型！

因此，Flurry 做出了一个极其干脆利落的决断：**不在 Intrinsic 调用与外围上下文之间搞双向猜测！有输出就必须在调用屁股后面紧跟 `.as(T)` 显式定型；没跟 `.as(T)` 就一律定型为 `void`！**

---

## 二、 核心心智模型：两个 `.as(T)` 的本质区别！

在表面语法上，`Inst.fma'intrinsic(a, b, c).as(f32)` 和 `(3.0f32).as(u32)` 都用了后缀 `.as(...)`。但在编译器内部的 Elaboration 算法（`docs/pipeline-projection/intrinsics.md`）中，它们走的是完全不同的两条路径：

```text
elaborate_intrinsic(expression):
    if expression directly supplies as(T) to an intrinsic call:
        output = check_type_expression(T)
        call = that intrinsic call
    else:
        output = void
        call = expression

    tag = resolve_intrinsic_variant(call)
    inputs = prepare_input_expressions_in_source_order(call)
    require_unsafe_permission(call)
    return Intrinsic(tag, inputs, output, source_origin)
```

让我们把这两条路径画成直观的对比图：

```
+-----------------------------------------------------------------------------+
|       紧接 Intrinsic 的 `.as(T)`   vs   普通表达式的 `.as(T)`                 |
+-----------------------------------------------------------------------------+
|                                                                             |
|  [ 形态 A：紧接 Intrinsic 调用 (`Inst.fma'intrinsic(a, b, c).as(f32)`) ]    |
|  • AST/HIR 组合识别：`.as(f32)` 直接被吸收进 `Intrinsic` 节点的 `output` 槽！|
|  • 真实语义：“向目标 Pipeline 请求一条直接产出 `f32` 的 `Inst.fma` 指令！”   |
|  • 绝不是：❌ 先算出一个 `void` 值，再对 `void` 调用类型转换转成 `f32`！     |
|                                                                             |
|  [ 形态 B：普通类型转换 (`(Inst.fma'intrinsic(a, b, c).as(f32)).as(u32)`) ] |
|  • 内层 `.as(f32)`：紧接 Intrinsic，指定硬件指令产出 `f32` 寄存器值；         |
|  • 外层 `.as(u32)`：作用在已经求出的 `f32` 普通值上，执行浮点到整数转换！    |
+-----------------------------------------------------------------------------+
```

> **🔍 语法小贴士：加一层括号会发生什么？**
> 注意 `docs/pipeline-projection/intrinsics.md` 中的精确表述：**“识别此组合时保留普通分组括号的表达式范围。只有直接提供给该调用的输出指定被写入节点；经变量读取或普通函数调用产生的外围期望不能事后改写它。”**
> 这意味着：
> - 如果你写 `(Inst.fma'intrinsic(a, b, c)).as(f32)`，括号把里面的 `Inst.fma'intrinsic(a, b, c)` 封闭成了一个没有直接 `.as` 的独立表达式（其输出类型当场固定为 `void`），外面的 `.as(f32)` 就会变成“试图把 `void` 转成 `f32`”而报错！
> - 因此，指定 Intrinsic 输出类型的 `.as(T)` **必须紧贴在 `'intrinsic(...)` 的右括号后面**！

---

## 三、 `v1` 实战：缺少 `.as(T)` 时外围类型注解无法挽救！

让我们用代码直接验证第一条铁律：**左侧变量类型注解 `: f32` 或函数返回类型 `-> f32`，绝不能替代紧接调用的 `.as(f32)`**！

先看正确显式指定输出类型的写法：

```flurry
^intrinsic_enum
enum Inst {
    fma,
}

-- ✅ 正确写法：在每个产生结果的 intrinsic 调用处紧接 `.as(f32)` 显式指定输出类型
unsafe fn explicit_result_type(a: f32, b: f32, c: f32) -> f32 {
    let result = Inst.fma'intrinsic(a, b, c).as(f32);
    return result;
}

test "explicit as on intrinsic call v1" {
    let v = unsafe {
        explicit_result_type(2.0f32, 5.0f32, 3.0f32)
    };
    assert(v == 13.0f32);
}
```

再看省略了 `.as(f32)`、妄图靠 `let result: f32 = ...` 推导的拒绝例（源自 `docs/pipeline-projection/intrinsics.md`）：

```flurry
-- ❌ 编译期直接拒绝！Inst.fma'intrinsic(a, b, c) 未紧接 .as(f32)，
-- 因此其输出类型直接定型为 void，无法赋给左侧要求 f32 的变量 result！
unsafe fn missing_result_type(a: f32, b: f32, c: f32) -> f32 {
    let result: f32 = Inst.fma'intrinsic(a, b, c);
    return result;
}
```

编译器给出的诊断还会贴心地专门区分：**これは“因缺少 `.as(T)` 输出指定而产生 `void`，导致与外围期望类型 `f32` 不符”**，一目了然地提醒你补上 `.as(f32)`！

---

## 四、 `v2` 进阶：为什么 `.as(u32)` 不是“先算 `f32` 再转 `u32`”？

现在来看第二个极具启发性的问题：
假设我们的目标硬件只支持输入为三个 `f32`、输出也为 `f32` 的单精度浮点乘加指令 `Inst.fma`。
如果一个程序员想把乘加算出来的 `f32` 结果转成整数 `u32`，他偷懒少写了一步，直接写成了：
```flurry
Inst.fma'intrinsic(a, b, c).as(u32)
```
这行代码会发生什么？

```
  [ 程序员以为的含义 ]
  先调用 fma(a, b, c) 算出 f32 ──► 再把 f32 转成 u32 ❌（错！）

  [ 编译器与 Pipeline 实际看到的含义 ]
  因为 `.as(u32)` 紧接在 `'intrinsic(...)` 后面，
  它代表向目标后端请求：“请给我一条接收三个 f32、但直接输出 u32 整数寄存器的 fma 硬件指令！”
  ──► 目标后端查表：我的 f32 fma 指令只能输出 f32，不支持直接输出 u32！
  ──► ❌ 目标检查直接报错拒绝！
```

如果要先执行 `f32` 硬件乘加，再将所得的 `f32` 数值转换为 `u32`，正确的两步写法与错误的单步写法对比如下：

```flurry
-- ✅ 正确写法（两步法）：
-- 第 1 步：通过紧接的 `.as(f32)` 向硬件请求 `Inst.fma` 支持的原生 `f32` 输出；
-- 第 2 步：对已经取得的 `f32` 值调用普通类型转换 `.as(u32)`！
unsafe fn fma_then_convert_to_u32(a: f32, b: f32, c: f32) -> u32 {
    let float_res = Inst.fma'intrinsic(a, b, c).as(f32);
    return float_res.as(u32);
}

test "intrinsic output type vs ordinary value cast v2" {
    let int_res = unsafe {
        fma_then_convert_to_u32(3.0f32, 4.0f32, 5.0f32)
    };
    assert(int_res == 17u32);
}
```

而如果直接把紧接 Intrinsic 的 `.as(...)` 写成该硬件操作不支持的 `.as(u32)`：

```flurry
-- ❌ 编译期（目标验证阶段）直接拒绝！
-- 紧接 intrinsic 的 `.as(u32)` 请求该硬件指令直接产出 u32，而非先产出 f32 再做数值转换；
-- 目标管线因三输入 f32 的 Inst.fma 不支持 u32 输出表示而拒绝！
unsafe fn wrong_output(a: f32, b: f32, c: f32) -> u32 {
    return Inst.fma'intrinsic(a, b, c).as(u32);
}
```

---

## 五、 本篇小结

| 写法形式 | 编译器解析归属 | 交给 Pipeline 的输出要求 | 外围表达式看到的类型 |
| :--- | :--- | :--- | :--- |
| `tag'intrinsic(args)` | 无输出指定的 `Intrinsic` 节点 | `void`（不交付值，但保留全部副作用与同步契约） | `void` |
| `tag'intrinsic(args).as(T)` | 带显式输出的 `Intrinsic` 节点 | 请求该硬件指令直接交付类型 `T` 的表示 | `T` |
| `let x: T = tag'intrinsic(args);` | ❌ 静态类型错误 | 节点输出仍为 `void`，左侧 `: T` 绝不反向改写节点 | 类型冲突：`void` vs `T` |
| `tag'intrinsic(args).as(f32).as(u32)` | 内层为 `Intrinsic` 输出，外层为普通转换 | 硬件指令交付 `f32`，随后对 `f32` 执行到 `u32` 的转换 | `u32` |

---

## 六、 留给下一篇的悬念

掌握了 `expr'intrinsic(args).as(T)` 的精确契约后，我们已经能够在函数里自由组合硬件指令了。

接下来，让我们把镜头拉到真实的 **x86_64 AVX2 向量数据路径（Vector Datapath）** 与 **跨管线目标验证（Target Validation）** 现场：
1. 当我们要在 CPU 上用 AVX2 256 位向量指令（`_mm256_loadu_ps` $\rightarrow$ `_mm256_add_ps` $\rightarrow$ `_mm256_storeu_ps`）一次性并行相加 8 个 `f32` 时，怎么用 `inline if build.target.arch != .x86_64` 在编译期守住目标架构与特性门禁？
2. 如果一个投射到 GPU 的 Kernel 调用了一个普通 Helper，而这个 Helper 的某个 `if` 分支里不小心用到了目标硬件不支持的操作（比如宿主专属的指令或无目标协议的函数指针间接调用），哪怕测试时没走到那个分支，编译器会放过它吗？
3. 当我们想把一条需要 `unsafe` 的硬件指令封装成零调用开销的安全原语时，为什么应该选 `quote fn` 而不是仅仅标 `inline fn`？

下一回 **《Flurry 演进手记 (175)：把 SIMD 向量数据路径关进安全笼子——目标约束检查、Helper 闭包与内联汇编封装》**，我们将亲手组装一条完整的 AVX2 SIMD 向量流水线！
