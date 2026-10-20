# Flurry 演进手记 (179)：从源码到运行时的阶段全景——定义期检查、调用点实例化与阶段边界

在[上一篇（第 178 篇）](./178-anar-build-scheduling-and-comptime-scripts.md)中，我们了解了工程构建与编译期脚本如何准备源码与配置输入。

从本篇（第 179 篇）开始到第 189 篇，我们将系统梳理 **Flurry 语言的核心语义规则如何协同工作**。
在一门支持第一类类型（First-Class Types）与编译期计算（`comptime`）的语言中，理解“什么在定义阶段检查、什么在调用点实例化时计算、什么保留到运行时执行”至关重要：
- 为什么有些类型函数在定义泛型函数时还没算出具体类型，却能通过定义期类型检查？
- 为什么两个互相递归的普通函数 `is_even` 和 `is_odd` 不需要编译期求值就能通过检查，但如果把运行期 `bool` 传给返回 `Type` 的函数就会被拒绝？
- 编译期类型计算与运行时依赖类型之间的边界究竟在哪里？

本篇我们将通过一段依赖类型代码，梳理从定义检查到具体实例化的完整阶段模型。

---

## 一、 常见误区：以为“先跑完所有编译期代码，再做类型检查”

在许多传统语言的心智模型里，阶段是单向线性的：先展开宏，再做类型检查，最后生成代码。
但在 Flurry 中，**类型本身可以由 `comptime` 函数计算得出**：

```flurry
pure comptime fn SelectType(wide: bool) -> Type {
    inline if wide { u64 } else { u32 }
}

fn keep_selected(comptime wide: bool, value: defer SelectType(wide)) -> SelectType(wide) {
    value
}
```

观察 `keep_selected` 的签名：参数 `value` 的类型是 `defer SelectType(wide)`，返回类型也是 `SelectType(wide)`。
1. 在检查 `keep_selected` 的**函数定义**时，`wide` 只是一个抽象的编译期布尔形参，此时还不知道它是 `true` 还是 `false`。但类型检查器已经可以验证：`SelectType(wide)` 返回的是合法的 `Type`，且函数体返回的 `value` 与声明的返回类型 `SelectType(wide)` 一致。
2. 而在某个具体的**调用点** `keep_selected(true, 42)` 上，为了检查字面量 `42` 是否匹配 `value` 的类型，编译期求值会先根据第一个实参 `true` 计算出 `SelectType(true) == u64`，再以 `u64` 作为期望类型检查第二个实参 `42`。

因此，在 Flurry 中，**类型推导与编译期求值是按需交织推进的**。

---

## 二、 阶段协作全景：从源码定义到目标代码

我们可以从程序员的视角，将 Flurry 代码的检查与执行划分为五个清晰的逻辑阶段：

```text
  [ 1. 语法解析与结构边界 ]
    确定表达式、模式、代码块边界与前缀属性顺序
                     │
                     ▼
  [ 2. 定义期检查 (Normalization) ]  ◀──▶  [ 按需执行已知输入的 comptime 计算 ]
    检查泛型与函数签名契约，验证定义内部一致性，
    保留依赖 comptime 形参的延迟类型表达式 (如 SelectType(wide))
                     │
                     ▼
  [ 3. 调用点实例化 (Instantiation) ] ◀──▶ [ 代入具体 comptime 实参完成类型归约 ]
    为具体调用代入 wide = true，将 SelectType(true) 彻底归约为 u64
                     │
                     ▼
  [ 4. 目标能力与安全契约检查 ]
    核验内存可达性、移动/析构次序、目标硬件指令集约束与验证契约
                     │
                     ▼
  [ 5. 机器码与设备产物生成 ]
    生成宿主机机器码或 GPU/加速器目标产物
```

这五个阶段各自向程序员提供明确的契约保证：

| 阶段 | 完成的检查与计算 | 允许保留到后续阶段的内容 |
| :--- | :--- | :--- |
| **1. 语法与名称绑定** | 确定括号、控制流块边界、`^expr` 属性修饰顺序与 `'view` 视图语法 | 待推导的局部类型、重载与 Trait 候选 |
| **2. 定义期检查** | 验证泛型函数/结构体声明在符号层面自洽；确认所有局部推导变量均已收敛 | 依赖泛型参数或 `comptime` 形参的编译期计算（如 `SelectType(wide)`） |
| **3. 调用点实例化** | 代入具体的 `comptime` 实参，**所有类型的编译期部分必须 100% 归约完成** | 合法的运行期控制流、以及合法的运行期依赖类型绑定（如动态切片长度或存储身份） |
| **4. 目标与资源验证** | 验证局部变量的移动与 `Drop` 顺序、算术与索引边界、目标平台指令合法性 | 具体的运行时执行逻辑 |

---

## 三、 实例追踪：`keep_selected(true, 42)` 的检查过程

让我们逐步跟踪下面这段代码在定义与调用时的检查过程：

```flurry
pure comptime fn SelectType(wide: bool) -> Type {
    inline if wide { u64 } else { u32 }
}

fn keep_selected(comptime wide: bool, value: defer SelectType(wide)) -> SelectType(wide) {
    value
}

pub fn demo() -> u64 {
    keep_selected(true, 42)
}
```

```text
  [ 步骤 1：检查 keep_selected 定义 ]
    - `wide` 是抽象的 `comptime bool` 形参。
    - 验证 `SelectType(wide)` 的结果类型为 `Type`。
    - 函数体返回 `value`（类型为 `SelectType(wide)`），与返回类型声明一致。
    - 定义期检查通过，`SelectType(wide)` 作为待实例化的类型计算保留在函数签名契约中。
          │
          ▼
  [ 步骤 2：处理 demo 中的调用 keep_selected(true, 42) ]
    - 第一个形参是 `comptime wide: bool`，先求值第一个实参得到编译期常量 `wide = true`。
          │
          ▼
  [ 步骤 3：归约延迟的参数与返回类型 ]
    - 既然 `wide = true` 已知，计算 `SelectType(true)`，走 `inline if` 的 `true` 分支得到 `u64`。
          │
          ▼
  [ 步骤 4：完成实参检查与实例生成 ]
    - 用期望类型 `u64` 检查第二个实参 `42`，字面量适配为 `42u64`。
    - 整个调用表达式的类型确定为 `u64`，满足 `demo() -> u64`。
    - 生成专门针对 `u64` 的运行时代码实例。
```

---

## 四、 两个关键边界：普通互递归 vs 非法运行期类型构造

理解了定义期检查与编译期求值的分工，就能明白为什么下面两段代码会有截然不同的结果：

### 1. 普通互递归函数不需要在编译期展开函数体

```flurry
fn is_even(n: u32) -> bool {
    if n == 0 { true } else { is_odd(n - 1) }
}

fn is_odd(n: u32) -> bool {
    if n == 0 { false } else { is_even(n - 1) }
}
```

在检查 `is_even` 的函数体时，它调用了 `is_odd(n - 1)`。此时不需要执行 `is_odd` 的函数体，因为 `is_odd` 已经显式声明了参数类型 `u32` 和返回类型 `bool`。类型检查只需核对签名契约即可通过，不会因为互递归而陷入编译期死循环。

### 2. 拒绝例：不能把运行期变量传给编译期类型构造函数

如果去掉 `keep_selected` 参数里的 `comptime` 修饰，试图在运行期根据动态输入决定返回 `u64` 还是 `u32`，会在阶段检查时被直接拒绝：

```flurry
pure comptime fn SelectType(wide: bool) -> Type {
    inline if wide { u64 } else { u32 }
}

-- ❌ 编译期拒绝：runtime_wide 没有标注 comptime，是运行期才产生的 bool
fn broken_runtime_type_factory(runtime_wide: bool) -> SelectType(runtime_wide) {
    -- 返回类型位置要求在编译期确定具体的物理类型布局，
    -- 不能将运行期布尔值 runtime_wide 传给 pure comptime fn SelectType。
    0
}
```

每个值在内存中的物理表示类型（到底是 4 字节的 `u32` 还是 8 字节的 `u64`）必须在编译期实例化完成时完全确定。如果需要在运行期返回两种可能的数据表示，应当使用枚举类型（`enum`）而非编译期类型函数。

---

## 五、 本篇小结

| 概念 | 核心规则 | 工程意义 |
| :--- | :--- | :--- |
| **定义期检查** | 验证函数与类型定义在符号层面自洽，保留依赖 `comptime` 形参的类型表达式 | 在编写泛型库时即可发现定义内部的逻辑与类型错误 |
| **按需交织求值** | 遇到具体 `comptime` 实参时立即计算类型函数（如 `SelectType(true)`） | 支持参数类型依赖前序 `comptime` 参数的灵活签名 |
| **调用点实例化** | 代入具体编译期实参后，所有类型的 `comptime` 部分必须彻底归约完毕 | 消除运行时类型分派开销，生成确定的目标机器码 |
| **阶段隔离边界** | 普通运行期变量不能直接流入 `comptime` 类型构造位置（除非使用 `lift` 依赖类型） | 保证每个运行时值的内存布局与调用约定在编译期完全确定 |

---

## 六、 下一篇预告

在熟悉了从定义到实例化的整体阶段边界后，下一篇我们将聚焦于日常编写复杂 Flurry 表达式时的三条实用语法与语义规则：
1. 在 `if enabled { 1 } else { 0 }` 和 `if (message matches .data { value: _ }) { true }` 中，控制流代码块 `{ ... }` 与结构体/模式花括号 `{ ... }` 的边界是如何划分的？
2. 当我们在实参或声明前连续标注多个属性 `^attr1 ^attr2 expr` 时，修饰顺序与求值次数遵循什么规则？
3. 当我们链式书写后缀视图（如 `value'bitvec'type`）时，每一步视图转换是如何组合的？

下一篇 **[Flurry 演进手记 (180)：复杂表达式的边界法则——多属性前缀、条件块边界与链式后缀视图](./180-parsing-outer-boundaries-and-hir-lowering.md)**，我们将逐一拆解这些规则。
