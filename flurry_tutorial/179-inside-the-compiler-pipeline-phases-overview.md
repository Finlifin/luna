# Flurry 演进手记 (179)：掀开 Flurry 编译器的引擎盖——从源码到机器码的完整流水线（Phases）全景图

在[上一篇（第 178 篇）](./178-anar-build-scheduling-and-comptime-scripts.md)中，构建调度器 `anar` 已经完成了包依赖解析、脚本执行与宏接口准备，把完整的源码与构建实例配置交到了 Flurry 编译器——**`luna`** 的手中。

从本篇（第 179 篇）开始，直到第 185 篇，我们将深入 **`luna` 编译器的内部架构**。
你可能会问：*“我是一个用 Flurry 写业务或底层库的程序员，了解编译器内部的流水线（Phases）对我有什么用？”*

用处太大了！
在支持第一公民类型（First-Class Types）和编译期计算（`comptime`）的语言里，如果你不清楚编译器在哪个阶段算什么，你就会经常困惑：
- 为什么有些类型函数在定义泛型函数时还没算出具体类型，编译器却能通过它的类型检查？
- 为什么两个互相递归的普通函数 `is_even` 和 `is_odd` 不需要编译期求值就能通过检查，但如果把运行期 `bool` 传给返回 `Type` 的函数就会被当场拦截？
- 为什么编译器报错说“不能冻结未决推断变量”，或者说“IIR 中所有类型的编译期部分已经全部收敛”？

今天，我们就画出一张完整的 **`luna` 编译器流水线全景图**，并跟踪一段真实的依赖类型代码走完从源码到 IIR 的全过程！

---

## 一、 `v0` 的认知误区：以为编译器是“先跑完所有宏 -> 再做完所有类型检查 -> 最后跑编译期代码”

在传统 C++ 或 Rust 的心智模型里，编译器的阶段是单向线性的：
1. 先展开宏；
2. 再做类型检查（Type Checking）；
3. 再做单态化（Monomorphization）和常量折叠（Const Eval）。

但在 Flurry 中，**类型本身就是用 Flurry 函数算出来的**！
请看这段极其经典的 Flurry 代码（源自 `docs/compilation/phases.md`）：

```flurry
pure comptime fn SelectType(wide: bool) -> Type {
    inline if wide { u64 } else { u32 }
}

fn keep_selected(comptime wide: bool, value: defer SelectType(wide)) -> SelectType(wide) {
    value
}
```

仔细看 `keep_selected` 的签名：参数 `value` 的类型是 `defer SelectType(wide)`，返回类型也是 `SelectType(wide)`。
为了对 `keep_selected` 进行类型检查（Elaboration），编译器必须知道 `SelectType(wide)` 是不是一个合法的 `Type`；而在某个具体调用点 `keep_selected(true, 42)` 上，为了检查字面量 `42` 能不能放进 `value`，编译器必须**当场调用编译期解释器去执行 `SelectType(true)`**，算出结果 `u64`，然后再回头继续做 `42 : u64` 的双向类型精化！

这意味着：**在 Flurry 编译器中，类型精化（Elaboration / Normalization）与编译期求值（Comptime Interpretation / Invocation）绝不是前后割裂的两个阶段，而是实时交织（Interleaved）在一起的双螺旋！**

---

## 二、 `v1` 架构全景：Luna 编译器的五层核心交付物与双螺旋引擎

让我们把 `docs/compilation/phases.md` 中的权威编译器流水线绘制成一张高清工程架构图：

```text
  [ 工程输入与宏准备 (由 anar 调度就绪) ]
                    │
                    ▼
  ┌─────────────────────────────────────────┐
  │ 1. AST (抽象语法树)                      │ ── 语法边界确定、外层块边界与属性链保真
  └────────────────────┬────────────────────┘
                       │ AST Lowering (Take View 进入专用节点)
                       ▼
  ┌─────────────────────────────────────────┐
  │ 2. HIR (高层中间表示)                    │ ── 绑定局部作用域、保留语法意图，准备精化
  └────────────────────┬────────────────────┘
                       │
                       ▼
  ┌─────────────────────────────────────────┐      ┌──────────────────────────────────┐
  │ 3. Elaboration 与 Normalization         │◀────▶│ Comptime Interpretation /        │
  │    (双向类型精化、局部依赖唤醒、类型归约) │ 交织 │ Invocation (编译期解释与函数调用) │
  └────────────────────┬────────────────────┘      └─────────────────▲────────────────┘
                       │                                             │
                       ▼                                             │
  ┌─────────────────────────────────────────┐                        │
  │ 4. 冻结检查结果与合法残余计算 (Freeze)   │                        │
  │    (消除活动推断变量/等待者，固化泛型残余)│                        │
  └────────────────────┬────────────────────┘                        │
                       │ 按具体调用点传入编译期实参                   │
                       ▼                                             │
  ┌─────────────────────────────────────────┐                        │
  │ 5. Instantiation (具体实例化)            │◀───────────────────────┘
  └────────────────────┬────────────────────┘    (代入具体实参，跑完残余编译期类型计算)
                       │
                       ▼
  ┌─────────────────────────────────────────┐
  │ 6. IIR (高级具体实例 Implementation IR)  │ ── 类型的 comptime 部分全部完成，非预设 SSA
  └────────────────────┬────────────────────┘
                       │
                       ▼
  ┌─────────────────────────────────────────┐
  │ 7. 目标验证 (Target Validation) 与降码   │ ── 验证执行依赖闭包，交付宿主机或设备后端
  └────────────────────┬────────────────────┘
                       ▼
              [ 最终代码与设备产物 ]
```

为了让你一眼看清每一层表示（IR）的职责边界，我们把这五层核心交付物的“必须提供（Must Provide）”与“合法保留（May Legally Retain）”浓缩为一张对照表：

| 阶段交付物 | 必须已经完成 / 提供的保证 | 允许合法保留的未决部分 |
| :--- | :--- | :--- |
| **1. AST** | 完整的词法/语法树结构、准确的源码 Span、正确的外层块边界与 `^expr` 属性链顺序 | 未解析的名称、表面语法糖、`expr'id` 原始取视图语法 |
| **2. HIR** | 局部名称绑定与模块路径解析准备；**所有 Take View（`'type` / `'bitvec` / `'intrinsic`）已降级为专用 HIR 节点** | 未推导的类型洞、重载/Trait 候选、未执行的 `comptime` 与 `defer` 类型表达式 |
| **3. Normalization 结果（冻结后）** | 声明级双向类型检查通过；**所有活动推断变量（Active Metavars）、等待队列与临时候选事务已彻底清空** | 依赖泛型参数或 `comptime` 形参的**合法编译期残余计算（Residual Computation）**（如 `SelectType(wide)`） |
| **4. 具体实例 IIR** | **所有类型的 `comptime` 部分 100% 归约完成**；泛型与 `inline if` 分支已按具体实参实例化；保留高级语义特征 | 合法的运行期制御流、**合法的运行期依赖维度/借用信息**（注：**IIR 不预设为底层 SSA！**） |
| **5. 后端产物** | 通过目标执行依赖闭包验证、满足 ABI 与表示布局的机器码 / PTX / 库产物 | 仅保留运行期链接与重定位符号 |

> **💡 核心架构红线（`AGENTS.md` §5）：为什么 IIR 不预设为 SSA？**
> 很多传统编译器一进入中端就把所有代码拍平为底层 SSA（Static Single Assignment）基本块，导致高级的结构化控制流、切片边界、资源析构作用域和管线投射信息全部丢失。
> 在 `luna` 中，**IIR 是一层保留高级语言特征的具体实例表示**：它的类型中所有编译期计算都已算完（比如 `SelectType(true)` 已经变成了具体的 `u64`），但它依然保留高级结构特征，既方便自定义管线（如 `^cuda.pipelines.global`）进行分析与转换，又能在后续按需降级到 LLVM SSA 或其他后端！

---

## 三、 实战追踪：跟着 `keep_selected(true, 42)` 走完 5 步生命周期

现在，让我们用显微镜跟踪本文开头的 `SelectType` 与 `keep_selected`，看看它在上面那张流水线图里经历了哪 5 个关键时刻：

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
  [ Step 1: 定义期检查 (Definition Elaboration & Freeze) ]
    检查 `keep_selected` 定义：
    - `wide` 是一个抽象的编译期形参 (`comptime wide: bool`)，此时还不知道它是 true 还是 false。
    - 编译器验证 `SelectType(wide)` 的类型确实是 `Type`。
    - 函数体返回 `value`（类型为 `SelectType(wide)`），与返回类型 `SelectType(wide)` 在归一化形式下相等！
    - 冻结（Freeze）定义：将 `SelectType(wide)` 作为合法的【编译期残余计算】保存下来。
          │
          ▼
  [ Step 2: 调用点准备 (Call Preparation in `demo`) ]
    看到调用 `keep_selected(true, 42)`：
    - 第一个参数对应 `comptime wide: bool`，编译器先行精化并求值第一个实参，得到编译期常量 `wide = true`。
          │
          ▼
  [ Step 3: 唤醒延迟类型计算 (Deferred Type Computation) ]
    第二个参数签名是 `value: defer SelectType(wide)`：
    - 既然 `wide = true` 已经就绪，Elaborator 唤醒等待 `wide` 的 `defer SelectType(wide)`；
    - 交织调用编译期解释器执行 `SelectType(true)` -> 走 `inline if` 的 true 分支 -> 产出具体类型 `u64`！
          │
          ▼
  [ Step 4: 完成调用点双向检查 (Call Completion) ]
    - 用刚刚算出的期望类型 `u64` 去检查第二个实参字面量 `42`（`42 <= u64` 通过！）；
    - 同时算出整个调用表达式的返回类型也是 `SelectType(true)` = `u64`，满足 `demo() -> u64`。
          │
          ▼
  [ Step 5: 具体实例化并交付 IIR (Instantiation -> IIR Delivery) ]
    - 为 `keep_selected(true, ...)` 生成具体实例 IIR：`fn keep_selected$true(value: u64) -> u64 { value }`。
    - 此时 IIR 中已经没有任何 `SelectType` 调用或 `comptime wide` 参数，只剩下纯正的 `u64`！
```

---

## 四、 两个关键边界：普通互递归 vs 非法运行期类型工厂

理解了这条流水线，你就能瞬间明白为什么下面这两种代码会有截然不同的命运：

### 1. 为什么普通的互递归函数（`is_even` / `is_odd`）不需要在编译期展开函数体？

```flurry
fn is_even(n: u32) -> bool {
    if n == 0 { true } else { is_odd(n - 1) }
}

fn is_odd(n: u32) -> bool {
    if n == 0 { false } else { is_even(n - 1) }
}
```

在检查 `is_even` 的函数体时，它调用了 `is_odd(n - 1)`。编译器需要去执行 `is_odd` 的函数体吗？**完全不需要！**
因为 `is_odd` 的返回类型已经显式声明为具体的 `bool`，不依赖任何编译期函数求值。Elaborator 只需要读取模块作用域里提前收集好的函数签名表，核对 `n - 1 : u32` 和返回值 `bool`，就能瞬间完成两个互递归函数的类型检查，绝不会陷入死循环。

### 2. 反例警告：为什么绝对不能把运行期 `bool` 传给 `SelectType` 来造类型？

如果有人去掉了 `keep_selected` 参数里的 `comptime` 关键字，试图在运行期根据用户输入动态决定返回 `u64` 还是 `u32`，流水线会在哪一步拦住它？

```flurry
-- ❌ 编译期直接拒绝！
pure comptime fn SelectType(wide: bool) -> Type {
    inline if wide { u64 } else { u32 }
}

-- 注意：这里的 runtime_wide 没有标注 `comptime`！它是一个运行期才能知道的 bool：
fn broken_runtime_type_factory(runtime_wide: bool) -> SelectType(runtime_wide) {
    --                                                ^^^^^^^^^^^^^^^^^^^^^^^^
    -- 编译期直接拒绝（Phase / Comptime Evaluation Error）：
    -- 返回类型位置要求在编译期（Elaboration / Instantiation 阶段，进入 IIR 之前）
    -- 将类型表达式归约为确定的 `Type`。
    -- 但 `runtime_wide` 是运行期值，无法提供给 `pure comptime fn SelectType` 进行编译期求值！
    -- Flurry 的类型只能在编译期计算，绝不存在“运行期动态造出新类型”的类型工厂。
    0
}
```

这正是前文表格里那句铁律的体现：**在进入 IIR 时，所有类型的 `comptime` 部分必须 100% 完成！** 运行期依赖类型只允许在合法的带洞结构（如切片长度、依赖索引等）中保留运行期值引用，绝不允许让“一个值到底是 `u32` 还是 `u64`”留到运行期再去猜。

---

## 本篇小结

| 编译阶段 / 概念 | 核心职责 | 架构级关键不变量 |
| :--- | :--- | :--- |
| **AST -> HIR** | 解析语法、确定外层结构边界、将 Take View 降级为专用节点 | 保持源码语义意图，不把编译器内部专用视图混同为普通字段访问 |
| **Elaboration <-> Comptime** | 双向类型检查、局部依赖按需挂起/唤醒、与编译期解释器交织求值 | 按需驱动类型函数（如 `SelectType`）求值，普通函数调用仅查签名不盲目展开 |
| **Freeze（冻结阶段）** | 封存声明级检查结果，固化泛型/编译期残余计算 | **严禁冻结任何未决的活动推断变量、等待者队列或临时候选事务** |
| **Instantiation -> IIR** | 代入具体编译期实参，消除所有编译期类型计算，生成高级具体实例 | **IIR 中类型的 `comptime` 部分必须完成**；IIR 保留高级特征，不预设为 SSA |
| **Target Validation -> 产物** | 验证具体实例的执行依赖闭包，通过宿主后端或自定义管线生成产物 | 任何未通过目标能力检查的实例绝不进入代码生成 |

---

## 留给下一篇的悬念

看完这张波澜壮阔的流水线全景图后，让我们把镜头推到流水线的**第一站：`AST -> HIR`**（语法解析与高层中间表示降级）。

你有没有想过，当你写出下面这几行看似简单的 Flurry 代码时，解析器（Parser）和 HIR Lowering 在幕后克服了什么棘手的语法歧义？
1. 在 `if enabled { 1 } else { 0 }` 中，既然 Flurry 支持 `expr { ... }`（带尾随代码块的扩展应用语法），为什么解析器没有把 `enabled { 1 }` 误当成一个叫 `enabled` 的函数带着 `{ 1 }` 代码块调用，从而导致 `if` 找不到自己的条件体？
2. 在 `if message matches .empty { true } else { false }` 中，为什么模式 `.empty` 没有把后面的 `{ true }` 当成结构体模式 `.empty { ... }` 吞掉？
3. 当你在函数实参前连写好几个属性 `keep_flag(^{ some_attr: 1 } ^{ some_attr: 2 } true)`，或者写下 `value'bitvec'type` 时，AST 是怎么保证不爆栈、不重算，并且把每个 `'view` 精准映射成专用 HIR 节点的？

下一篇 **[Flurry 演进手记 (180)：语法解析与 HIR 降级——Multi 属性链、外层结构边界与 Take View 专用节点](./180-parsing-outer-boundaries-and-hir-lowering.md)**，我们将深入编译器前端的第一道关卡！
