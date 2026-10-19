# 第 185 期：交织的舞台——Elaboration、Comptime 与区域冻结（Freezing）

> **系列导航**：[Flurry 语言漫游（200 期规划）](./README.md)  
> **当前卷目**：第八卷 · 编译器原理与全栈实战（181–200）  
> **上一期**：[第 184 期：候选求解、沙盒隔离探测（Isolated Trial）与事务提交](./184-candidate-solving-isolated-probing-and-commit.md)  
> **下一期**：[第 186 期：运行时依赖类型的编译器表示——`TypeCore`、空洞替换与环境计划](./186-representing-runtime-dependent-types-in-the-compiler.md)

---

## 一、开篇痛点：当类型检查需要运行代码，而运行代码又依赖类型检查，谁先谁后？

在 C++ 或传统语言中，“模板定义检查”、“常量折叠”和“代码生成”往往被生硬地切成互不往来的独立流水线阶段。但在 Flurry 中，类型本身就是第一类值，函数签名里可以直接调用编译期函数计算类型！

请看下面这两段经典的 Flurry 代码：

```flurry
pure comptime fn Identity<T: Type>() -> Type {
    return T;
}

pure comptime fn SelectType(wide: bool) -> Type {
    return if wide { i64 } else { i32 };
}

fn keep_selected(
    wide: comptime bool,
    value: SelectType(wide),
) -> SelectType(wide) {
    return value;
}

const SMALL: i32 = keep_selected(false, 3i32);
const LARGE: i64 = keep_selected(true, 7i64);
```

仔细体会一下编译器在处理这段代码时面临的挑战：
1. 在检查 `const SMALL: i32 = keep_selected(false, 3i32)` 时，为了知道第二个参数 `3i32` 的形参类型是什么，Elaboration 必须当场把 `wide = false` 代入 `SelectType(false)` 进行编译期求值，算出结果类型 `i32`，然后再继续做类型合一！
2. 而在先检查 `keep_selected` 的**函数定义本身**时，`wide: comptime bool` 的具体布尔值还不知道！此时 `SelectType(wide)` 还不能算出具体的 `i32` 或 `i64`，只能作为一个**受控的延迟类型计算残余（Deferred Residual）**保留在函数契约中。

### 初代尝试（`v0`）：把“活跃求解器状态”直接当成函数定义缓存下来

在早期的原型设计（`v0`）中，为了省事，开发者在检查完 `keep_selected` 的定义后，直接把当前 Elaboration 区域的内存结构（里面还挂着半截未决的推断变量槽 `?T`、局部等待队列和求解器指针）原封不动地存进了全局定义表。

结果在编译 `SMALL` 和 `LARGE` 两个调用点时，灾难发生了：

```
  [ v0 未做区域冻结导致的状态串扰灾难 ]
  定义点缓存: keep_selected 内部直接留着可变的求解器指针 SolverCell(#99)
  
  调用点 1: keep_selected(false, 3i32) ---> 把共享的 SolverCell(#99) 改写成了 i32！
  调用点 2: keep_selected(true,  7i64) ---> 读取 SolverCell(#99) 发现已经是 i32，与 7i64 冲突报错！
```

更糟糕的是，有些初代实现为了“化简类型”，在看到 `pure comptime fn Identity<T: Type>() -> Type { return T; }` 时，不管三七二十一就把所有的 `Identity<T>` 暴力展开替换成了 `T`——这直接违背了 Flurry 语言的核心语义红线！

---

## 二、机制迭代（`v0 -> v1 -> v2`）：交织执行、双载体引擎与区域冻结（Freezing）

为了在支持编译期与类型推导无缝交织的同时，保证定义边界的绝对纯净，Luna 编译器构建了 **交织执行 + 双编译期载体 + 区域冻结（`freeze_definition`）** 架构。

### 铁律一：Normal Form 的头绝不越权展开（`Identity<T>` $\neq$ `T`）

在进入执行机制之前，我们先牢记 `AGENTS.md` §5 中的第一条红线：

> **Normal form 的头不展开；`Identity<T>` 不因函数体返回 `T` 就与 `T` 相等。**

为什么？在 Flurry 的类型系统中，一个带类型构造头的 Normal Form（范式头）拥有自己独立的**结构身份（Structural Identity）**。当 `Identity<T>` 作为类型构造头出现时，它保持 `Identity<T>` 的范式头身份，而不会因为它的函数体写了 `return T;` 就自动退化成 `T` 的透明别名。只有当编译期求值器在明确需要调用求值的语义点执行该调用时，才会按求值规则推进，绝不在类型等价判定时越权把范式头抹平！

### 铁律二：Elaboration 与 Comptime 的两种执行载体共享同一套语言语义

在编译期间，当 Elaboration 遇到需要立即求值的编译期表达式时，Luna 提供了两种内部执行载体（Carriers）：

```
                 +--------------------------------------------+
                 | 统一语义契约 (Stage / Eval / Type / Effect) |
                 +--------------------------------------------+
                        /                              \
                       v                                v
  [ 载体 A: Comptime Interpretation ]        [ 载体 B: Comptime Invocation ]
  适用：内嵌在定义/类型中的小段混合上下文      适用：已独立编译完成的纯 comptime fn
  方式：直接在归一化 IR 树上就地解释执行        方式：编译为紧凑的引擎字节码 (Bytecode) 高速调用
```

无论编译器底层走的是就地树解释（`comptime interpretation`）还是字节码调用（`comptime invocation`），对程序员而言是完全透明的——它们遵守 100% 相同的求值顺序、类型相等规则与诊断契约。

### 铁律三：区域冻结（`freeze_definition`）——绝不冻结任何活动推断变量或等待者！

当一个函数或类型定义（例如 `keep_selected`）完成了定义期的 Elaboration 后，在它离开局部推断区域、成为可供全程序复用的归一化产物（Frozen Normalization Result）之前，必须通过一道严格的安检门——**`freeze_definition(region)`**：

```
  [ 活跃推断区域 (Live Inference Region) ]
  - 局部推断变量槽 (?T0, ?T1)
  - 局部等待票据队列 (Wait Tickets)
  - 沙盒探测事务日志 (Candidate Trials)
  - 对未知 comptime 参数的计算: SelectType(wide)
                 |
                 |  执行 freeze_definition(region)
                 v
  ========================================================
   安检门过滤：
   1. 结算所有定义期必达义务；若有未决必填变量或残留等待票据 -> 报错！
   2. 局部符号重写为显式、稳定的约束绑定器 (Stable Binders: param#wide)
   3. 合法延迟计算封装为显式残余节点: (defer (call SelectType param#wide))
   4. 彻底剥离并销毁所有 SolverCell、WaitTicket、TrialLog！
  ========================================================
                 |
                 v
  [ 冻结后的归一化定义 (Frozen Definition) ]  <--- 纯不可变、可跨线程共享、可持久化缓存！
        /                                 \
       v (导入全新实例区域 1)               v (导入全新实例区域 2)
  keep_selected(false, 3i32)          keep_selected(true, 7i64)
  代入 wide = false                   代入 wide = true
  触发 SelectType(false) => i32       触发 SelectType(true) => i64
```

正如 `AGENTS.md` §5 所强调的：

> **Elaboration 与 comptime interpretation／invocation 交织；冻结稳定残余和契约，不冻结活动推断变量、等待者或候选事务。**

让我们看看 `keep_selected` 经过 `freeze_definition` 之后生成的 S-Expression 转储长什么样：

```scheme
(frozen-fn :name "keep_selected"
  :binders (
    (binder :id b_wide :name "wide" :stage comptime :ty bool)
    (binder :id b_value :name "value" :stage runtime
      :ty (defer-ty (comptime-call "SelectType" (binder-ref b_wide)))))
  :ret-ty (defer-ty (comptime-call "SelectType" (binder-ref b_wide)))
  :contracts ()
  :body (frozen-return (binder-ref b_value)))
```

请看这份转储：
- 里面**没有任何 `?T0` 这样的活动推断变量 ID**，也没有任何指向某个求解器内存池的裸指针；
- 所有对 `wide` 的依赖都被转化成了自包含的稳定绑定器引用 `(binder-ref b_wide)`；
- 依赖 `wide` 具体值的类型计算被干净地封装为 `(defer-ty (comptime-call "SelectType" (binder-ref b_wide)))`。

当后续编译 `const SMALL: i32 = keep_selected(false, 3i32)` 时，编译器只需将这份不可变的 `frozen-fn` 导入当前调用的全新推断区域，将 `b_wide` 绑定为 `false`，唤醒 `(defer-ty ...)` 执行 `SelectType(false)` 得到 `i32`，整个过程干净利落，两个调用点之间绝对没有任何状态串扰！

---

## 三、边界与编译期拒绝：禁止未决推断变量逃逸过冻结边界

如果在 `freeze_definition(region)` 安检时，发现定义内部还有未能收敛的活动推断变量或依然挂在半空中的等待票据，冻结器会立即拦截报错，绝不允许带病产物进入下游或写入增量缓存：

```flurry
fn unresolved_internal_hole(x: i32) -> i32 {
    -- 局部变量 y 的类型依赖一个既无输入约束、也无输出约束的泛型空洞
    let y = default_of_unknown(); -- ❌ 编译期直接拒绝！在 `freeze_definition` 阶段检测到未决的活动推断变量 `?T_default`，禁止逃逸出定义区域
    return x;
}
```

---

## 四、本期小结

| 核心机制 | 解决的痛点 | 语义与工程保障 |
| :--- | :--- | :--- |
| **Normal Form 头保持** | 防止类型构造头被越权抹平 | `Identity<T>` 保留其范式头身份，不因函数体返回 `T` 就与 `T` 等同 |
| **双编译期执行载体** | 兼顾混合上下文灵活性与独立函数的高性能执行 | `comptime interpretation`（树解释）与 `comptime invocation`（字节码调用）严格遵守同一套语义契约 |
| **区域冻结 (`freeze_definition`)** | 防止多个调用点共享可变求解器状态导致串扰 | 仅冻结稳定的绑定器、契约和合法的 `defer` 残余；彻底剥离活动推断变量、等待票据与沙盒事务 |
| **实例化导入 (`import_frozen`)** | 支持泛型与 `comptime` 参数的高效复用 | 每次调用在全新的实例区域中导入冻结定义并填入实参，互不干扰 |

在 `keep_selected(wide: comptime bool, ...)` 中，被依赖的参数 `wide` 是一个 `comptime` 编译期参数，所以在实例化时就能把类型彻底算出来。

但如果一个类型依赖的不是 `comptime` 参数，而是一个**直到程序运行时才会产生、或者指向某个运行时局部变量的引用**呢？比如我们在前面卷目见过的 `Sensor<first.ref>` 和 `Sensor<second.ref>`！如果每个运行时实例都去生成一份全新的编译器类型结构，编译器的内存池岂不是要爆炸？而且 `Sensor<first.ref>` 和 `Sensor<second.ref>` 明明底层布局一模一样，编译器该如何在共享它们的核心结构的同时，又不把这两个不同的依赖类型混为一谈？

在下一期（**第 186 期**）中，我们将揭开 **运行时依赖类型的编译器表示——可驻留的带洞 `TypeCore`、引用端替换（`TypeRef`）与环境取得计划（`EnvironmentPlan`）**！我们下期见！
