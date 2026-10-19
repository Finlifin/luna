# 第 186 期：运行时依赖类型的编译器表示——`TypeCore`、空洞替换与环境计划

> **系列导航**：[Flurry 语言漫游（200 期规划）](./README.md)  
> **当前卷目**：第八卷 · 编译器原理与全栈实战（181–200）  
> **上一期**：[第 185 期：交织的舞台——Elaboration、Comptime 与区域冻结（Freezing）](./185-interleaving-elaboration-comptime-and-freezing.md)  
> **下一期**：[第 187 期：细粒度增量查询（Incremental Queries）、环路检测与错误恢复](./187-fine-grained-incremental-queries-and-cycle-recovery.md)

---

## 一、开篇痛点：当类型依赖运行时变量，如何兼顾“结构共享”与“身份隔离”？

在前面的卷目中，我们学过 Flurry 极具特色的**运行时依赖类型（Runtime-Dependent Types）**：一个类型可以参数化于某个运行时的值或引用（通过 `lift` 提升进入类型位置）。

让我们看一段非常典型的传感器校准代码：

```flurry
struct Calibration {
    scale: i32,
}

struct Sensor<cal: lift *Calibration> {
    raw: i32,
}

struct SensorPair<first: Calibration, second: Calibration> {
    left: Sensor<first.ref>,
    right: Sensor<second.ref>,
}

fn read_scaled(
    cal: *Calibration,
    sensor: Sensor<cal>,
) -> i32 {
    return sensor.raw * cal.*.scale;
}
```

请仔细观察 `SensorPair` 结构体里的两个字段：
- `left` 的类型是 `Sensor<first.ref>`，它绑定的是第一个校准器 `first` 的地址；
- `right` 的类型是 `Sensor<second.ref>`，它绑定的是第二个校准器 `second` 的地址。

站在编译器设计者的角度，这两个类型给我们出了一道两难的选择题：
1. **从内存布局与类型模板来看**：无论是 `Sensor<first.ref>`、`Sensor<second.ref>` 还是 `Sensor<cal>`，它们在物理内存中都只包含一个 `raw: i32` 字段（4 字节）！如果程序里有一万个不同变量绑定的 `Sensor<...>`，我们显然希望它们在编译器的全局类型池（Type Interner）里**共享同一个底层类型骨架**，而不是复制一万份结构定义。
2. **从类型安全与语义身份来看**：`Sensor<first.ref>` 和 `Sensor<second.ref>` **绝对不是同一个类型**！你绝不能把绑定了 `second.ref` 的 `right` 传感器当成 `Sensor<first.ref>` 传给 `read_scaled(first.ref, ...)`，否则就会用错校准系数！

### 初代尝试（`v0`）：把运行时变量直接塞进全局驻留池，或者干脆抹平运行时依赖

在早期的尝试（`v0`）中，两种极端做法都栽了跟头：
- **做法一（把局部运行时变量直接塞进全局 Interner）**：导致全局类型池被海量的局部栈帧变量 ID 塞满，函数一退出，全局池里全是指向已销毁局部作用域的悬垂变量引用，根本无法跨模块缓存！
- **做法二（在类型表示里擦除运行时参数，只看 `Sensor` 名字）**：虽然全局池干净了，但编译器把 `Sensor<first.ref>` 和 `Sensor<second.ref>` 当成了完全相等的类型，类型安全性瞬间崩塌！

还有第三个实际问题：在 `SensorPair` 里，为什么 `Sensor<first.ref>` 只需要占 4 个字节（`raw: i32`），而不需要在每个 `Sensor` 实例内部冗余地存一个 8 字节的 `*Calibration` 指针？当代码真正需要读取 `cal.*.scale` 时，编译器怎么知道去哪里拿到那个运行时指针？

---

## 二、机制迭代（`v0 -> v1 -> v2`）：`TypeCore` + `TypeRef` + `EnvironmentPlan` 三位一体

为了完美拆解这个难题，Luna 编译器严格遵循 `AGENTS.md` §5 的设计准则：

> **运行时依赖类型使用可 intern 的带洞结构与引用端 substitution；共享 core 不等于完整类型相等，环境取得另行保持。**

让我们用一张架构图，看清 Luna 是如何将运行时依赖类型拆分为三个正交层面的：

```
  1. 全局可驻留的核心层 (Interned TypeCore) —— 跨全程序唯一共享！
  +-------------------------------------------------------------------+
  | TypeCore #42: Sensor<H0>                                          |
  | - 不可变类型项模板: Struct("Sensor", fields: [raw: i32])          |
  | - 有序空洞签名 (Ordered Hole Signature):                          |
  |     H0 : stage = runtime_dependency, ty = lift *Calibration       |
  +-------------------------------------------------------------------+
                     ^                               ^
                    /                                 \
  2. 引用端替换层 (TypeRef = Core + Substitution) —— 区分完整类型身份！
  +-----------------------------------+   +-----------------------------------+
  | TypeRef A (字段 left 的类型)      |   | TypeRef B (字段 right 的类型)     |
  | - core:  TypeCore #42             |   | - core:  TypeCore #42             |
  | - subst: { H0 -> R_first_ref }    |   | - subst: { H0 -> R_second_ref }   |
  +-----------------------------------+   +-----------------------------------+
         (TypeRef A != TypeRef B！共享 Core 绝不等于完整类型相等！)

  3. 环境取得计划 (EnvironmentPlan) —— 按需获取运行时值，无需膨胀值布局！
  +-------------------------------------------------------------------+
  | EnvironmentPlan (在 SensorPair 上下文中):                         |
  | - R_first_ref  ==> FieldAddr(self, .first)                        |
  | - R_second_ref ==> FieldAddr(self, .second)                       |
  +-------------------------------------------------------------------+
```

### 第一层：`TypeCore`（全局驻留的带洞不可变结构）

在全局类型驻留池（Global Interner）中，编译器绝不存储任何具体的局部运行时变量 ID。相反，它把所有的运行时依赖位置抽象为标准化的**模板空洞（Template Holes：`H0`, `H1`, ...）**。
- `Sensor<first.ref>` 和 `Sensor<second.ref>` 剥离掉具体的引用后，长相完全一致，都归结为同一个 `TypeCore`：`Sensor<H0>`（其中 `H0` 的空洞签名是 `lift *Calibration`）。
- 因为 `TypeCore` 不含任何局部栈帧信息，它可以 100% 安全地进行全局哈希驻留（Interning）、跨函数共享以及增量编译持久化！

### 第二层：`TypeRef`（携带引用端 Substitution 的完整类型引用）

每当编译器在具体的外层结构、函数签名或表达式上引用这个类型时，使用的是轻量级的 `TypeRef`：

$$\text{TypeRef} = (\text{interned\_core},\ \sigma_{\text{subst}})$$

- 对于 `left` 字段，它的 `TypeRef` 是 `(Sensor<H0>, [H0 ↦ R_first_ref])`；
- 对于 `right` 字段，它的 `TypeRef` 是 `(Sensor<H0>, [H0 ↦ R_second_ref])`。

当编译器判断两个类型 `T1` 和 `T2` 是否相等时：
1. 首先快速比对 `T1.core == T2.core`（仅需一次指针/ID 比较，$O(1)$）；
2. **绝不就此止步！** 接着必须在当前作用域下逐一比对两者的替换表 `T1.subst == T2.subst`。因为 `R_first_ref != R_second_ref`，编译器立刻判定 `Sensor<first.ref>` 与 `Sensor<second.ref>` 类型不相等！

### 第三层：`EnvironmentPlan`（环境取得计划与布局解耦）

为什么我们还要把 `EnvironmentPlan[dependency_identity] = legal_access_path` 单独拎出来维护？

看看 `SensorPair` 和 `read_scaled`：
- 在 `SensorPair` 中，`first` 和 `second` 已经作为同级字段存储在 `SensorPair` 内部了。通过 `EnvironmentPlan`，编译器记录下：`R_first_ref` 可以通过外层容器的 `.first.ref` 路径直接获取，因此 `left: Sensor<first.ref>` 的物理内存布局只需要保存 4 字节的 `raw: i32`，完全不需要在 `left` 里面再塞一个 8 字节的指针！
- 而在 `read_scaled(cal: *Calibration, sensor: Sensor<cal>)` 中，`EnvironmentPlan` 记录下依赖项 `R_cal` 直接取自第 0 个形参寄存器 `cal`！

---

## 三、深度法则：严格区分三种“占位符”与避捕获替换复合

在实现运行时依赖类型时，最容易犯的错误就是把编译器里的三种“占位符”混为一谈。在 Luna 内部，它们有着严格隔离的表示与权限边界：

| 占位符种类 | 编译器内部记号 | 语义角色 | 求解器（Unifier）能否对其赋值？ |
| :--- | :--- | :--- | :--- |
| **1. 模板空洞 (Template Hole)** | `H0`, `H1` | `TypeCore` 上的形式接口槽位，只存在于驻留核心内部 | **否**（由外层 `TypeRef` 的 `subst` 表填入实参） |
| **2. 运行时绑定项 (Runtime Binder Term)** | `R_cal`, `R_first` | 代表程序在运行时才会产生的固定值/地址身份 | **绝对禁止！** 它是刚性的运行时身份，绝不是未知推断变量 |
| **3. 推断元变量 (Inference Metavariable)** | `?M0`, `?T1` | 当前调用点或沙盒探测中等待推导的编译期未知量 | **是**（在当前推断区域内可被合一算法赋值收窄） |

此外，当一个依赖类型嵌套在另一个依赖类型内部时（例如外层泛型的替换表 $\theta$ 作用于内层类型的替换表 $\sigma$），编译器执行**避捕获替换复合（Capture-Avoiding Substitution Composition）**：

$$\text{compose}(\sigma, \theta)[h] = \text{substitute\_capture\_avoiding}(\sigma[h], \theta)$$

如果外层重命名了某个运行时绑定变量（比如在内联或投影重写时将 `cal` 重命名为 `cal_inlined`），编译器会自动在必要时对内部局部绑定做 $\alpha$-重命名，彻底杜绝外层同名变量意外捕获（Variable Capture）内层依赖项的事故！

让我们看看 `SensorPair` 在冻结归一化后的 S-Expression 转储：

```scheme
(type-core :id tc_sensor :name "Sensor"
  :holes ((hole H0 :stage runtime-dep :ty (lift (ptr Calibration))))
  :layout (struct-layout :size 4 :align 4
            (field "raw" :offset 0 :ty i32)))

(frozen-struct :name "SensorPair"
  :binders (
    (binder R_first :name "first" :ty Calibration)
    (binder R_second :name "second" :ty Calibration))
  :fields (
    (field "left"
      :ty (type-ref :core tc_sensor :subst ((H0 (addr-of R_first))))
      :env-plan ((H0 (container-field-addr "first"))))
    (field "right"
      :ty (type-ref :core tc_sensor :subst ((H0 (addr-of R_second))))
      :env-plan ((H0 (container-field-addr "second"))))))
```

---

## 四、边界与编译期拒绝：共享 `TypeCore` 绝不能蒙混通过不同依赖身份的检查

让我们看看如果程序员试图把绑定了 `second.ref` 的 `right` 传感器传给期待 `first.ref` 的 `read_scaled` 函数，Luna 编译器是如何精准拦截的：

```flurry
fn mismatched_calibration(pair: *SensorPair<c1, c2>) -> i32
where
    c1: Calibration,
    c2: Calibration,
{
    -- 第 1 个参数传入了 first 的地址，因此第 2 个参数要求类型必须是 Sensor<pair.*.first.ref>
    -- 但这里却传入了类型为 Sensor<pair.*.second.ref> 的 pair.*.right！
    return read_scaled(pair.*.first.ref, pair.*.right); -- ❌ 编译期直接拒绝！虽然两个 Sensor 共享同一个 TypeCore `Sensor<H0>`，但它们的引用端替换 `H0 -> R_first_ref` 与 `H0 -> R_second_ref` 不相等！
}
```

再看另一个边界：如果某个函数试图返回一个依赖**即将销毁的局部栈变量**的类型，而调用点又无法在外部合法取得该环境路径，`EnvironmentPlan` 检查也会当场拦截：

```flurry
fn leaked_local_dependency() -> Sensor<local_cal.ref> {
    let local_cal = Calibration { scale: 2 };
    -- ❌ 编译期直接拒绝！返回类型中的运行时依赖项 `local_cal.ref` 引用了函数内部的局部绑定，在函数返回后无法建立合法的外部 EnvironmentPlan 访问路径
    return Sensor<local_cal.ref> { raw: 10 };
}
```

---

## 五、本期小结

| 核心层级 | 数据结构 | 核心职责与不变量 |
| :--- | :--- | :--- |
| **核心模板层** | `TypeCore` | 将运行时依赖抽象为有序空洞签名（`H0, H1`），不含局部栈帧状态，全局安全 Intern 驻留 |
| **引用替换层** | `TypeRef = (Core, Subst)` | 在使用点为每个空洞填入具体的运行时依赖身份（`R_first` 等）；**共享 Core 绝不等于完整类型相等** |
| **环境取得层** | `EnvironmentPlan` | 显式记录每个运行时依赖项在当前上下文中的合法获取路径（形参、外层字段等），避免值布局膨胀 |
| **替换复合算子** | `compose(σ, θ)` | 执行避捕获替换（Capture-Avoiding Substitution），在变量重命名与内联时保持依赖身份一致 |

现在，我们的编译器已经拥有了完整的精化、候选求解、区域冻结与依赖类型表示。但在真实的大型工程开发中，没人愿意每次只改一行代码（比如把一个函数体里的 `+ 1` 改成 `+ 2`），就让编译器把整个工程几千个函数全部从头推导一遍！

编译器怎样才能知道：把 `adjust` 函数体里的 `value + 1` 改成 `value + 2` 后，调用它的 `show_adjusted` 函数**不需要重新做类型检查**，但**需要重新生成内联机器码**？如果程序员不小心写出了 `const FIRST: u32 = SECOND + 1; const SECOND: u32 = FIRST + 1;` 这样的常量死循环，增量查询引擎又该如何确定性地捕获环路并让 IDE 继续正常工作？

在下一期（**第 187 期**）中，我们将揭秘 **细粒度增量查询（Incremental Queries）、接口隔离、确定性环路检测与错误恢复**！我们下期见！
