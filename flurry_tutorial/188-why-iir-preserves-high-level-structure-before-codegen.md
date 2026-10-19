# 第 188 期：为什么 IIR 在降码前要保留高级结构而非过早退化为 SSA？

> **系列导航**：[Flurry 语言漫游（200 期规划）](./README.md)  
> **当前卷目**：第八卷 · 编译器原理与全栈实战（181–200）  
> **上一期**：[第 187 期：细粒度增量查询（Incremental Queries）、环路检测与错误恢复](./187-fine-grained-incremental-queries-and-cycle-recovery.md)  
> **下一期**：[第 189 期：实现带安全回退的 `BitVec` 形式化综合器](./189-implementing-verified-bitvec-synthesis-with-fallback.md)

---

## 一、开篇痛点：如果一出前端就退化成底层 SSA + `goto` 跳转，我们会失去什么？

当一段 Flurry 代码完成了前端的 Elaboration、编译期函数求值与具体化实例化之后，就进入了中端的核心载体——**IIR（Instantiated Intermediate Representation，实例化中间表示）**。

在许多传统编译器的设计（`v0`）中，一旦类型检查结束，编译器会迫不及待地把所有高级语法结构一键拍平（Flatten），直接降级成类似 LLVM IR 的底层 **CFG + SSA（控制流图 + 静态单赋值 $\phi$ 节点 + 原始指针运算）**。

### 初代尝试（`v0`）：过早拍平为低级 SSA 的三大惨重代价

如果我们把 Flurry 程序一出前端就拍平为低级 SSA，会立刻遭遇三个不可挽回的损失：

```
  [ v0 过早退化为底层 SSA 的灾难 ]
  1. 结构化循环与模式匹配被拆碎成满屏的 br / phi 节点：
     --> 线性资源清理检查、GPU 结构化控制流后端（SPIR-V）被迫做痛苦的“控制流重建”！
  2. 错误返回值 (`!Err`) 与代数效应 (`#[eff]`) 被过早揉成低级 ABI 分支：
     --> 尾恢复效应（Tail-resumptive Effect）内联优化与流水线融合彻底认不出原来的语义！
  3. 连续的 BitVec 切片与拼接被拆成几十条零碎的移位/掩码指令：
     --> 第 189 期的 BitVec 形式化综合器失去全局代数视图，无法合成单条硬件位操作指令！
```

正因如此，Luna 编译器在 `AGENTS.md` §5 中确立了关于 IIR 的核心铁律：

> **IIR 保留高级特征，不预设为 SSA；其中类型的 comptime 部分必须完成，合法 runtime 依赖仍可存在。**

---

## 二、机制迭代（`v0 -> v1 -> v2`）：IIR 的黄金边界——“什么必须完成”与“什么必须保留”

在 Luna 编译器的架构（`v2`）中，IIR 处于一个精妙的黄金分割点上：**它向下游承诺“所有编译期不确定性已 100% 消除”，同时又向上游保留了完整的高级程序结构。**

让我们用一张全景对比图，看清进入 IIR 时的准入契约：

```
  ================================================================================
                          [ IIR (实例化中间表示) 黄金边界 ]
  ================================================================================
   ✅ 已经彻底完成 (Completed at IIR Entry)    💎 刻意保留的高级特征 (Preserved High-Level)
  -----------------------------------------   --------------------------------------------
  1. 所有类型的 comptime 部分已求值完毕       1. 结构化控制流 (loop / break / match 分支)
     (不存在未展开的 SelectType(true))           而非碎片化的 goto / SSA phi 节点！
  2. 所有推断元变量 (?T0) 已消解              2. 显式闭包捕获清单与环境所有权契约
  3. 所有候选求解与隐式证据已唯一锁定         3. 错误结果 (!Err) 与代数效应 (#[eff]) 独立表示
  4. 所有模板空洞 (H0) 已填入合法替换         4. 合法的运行时依赖类型引用 (TypeRef + EnvPlan)
  5. 保持合法的 Normal Form 类型构造头        5. 高级 BitVec 视图/拼接/切片节点与流水线原语
  ================================================================================
```

下面我们通过四个真实的代码场景，看看 IIR 保留这些高级特征究竟带来了多大的威力！

---

## 三、深度透视：IIR 保留的四大高级特征及其降码契约

### 1. 结构化控制流与模式匹配（Structured Loops & Pattern Matching）

请看这段通过循环交替翻转布尔状态来判断奇偶性的代码：

```flurry
fn even_by_loop(limit: u32) -> bool {
    let mut index: u32 = 0;
    let mut even = true;
    return loop {
        if index == limit {
            break even;
        }
        index += 1;
        even = not even;
    };
}
```

在 IIR 中，这个 `loop { ... break even; }` **绝不会被拆成底层标签（Basic Block Labels）和 `goto` 跳转**，也不会为了迎合 SSA 而把 `index` 和 `even` 拆成 `index_0 = phi(0, index_1)`。相反，IIR 直接保留了带返回值的结构化 `iir-loop` 和 `iir-break` 节点：

```scheme
(iir-fn :instance "even_by_loop"
  (params (iir-param :local l_limit :name "limit" :ty u32))
  :ret-ty bool
  (iir-block
    (iir-let-mut :local l_index :name "index" :ty u32 :init (iir-lit 0 u32))
    (iir-let-mut :local l_even  :name "even"  :ty bool :init (iir-lit true bool))
    (iir-return
      (iir-loop :ty bool
        (iir-if :cond (iir-binop eq (iir-local l_index) (iir-local l_limit))
          :then (iir-break :value (iir-local l_even))
          :else (iir-unit))
        (iir-assign :dst l_index :val (iir-binop add (iir-local l_index) (iir-lit 1 u32)))
        (iir-assign :dst l_even  :val (iir-unop not (iir-local l_even)))))))
```

保留结构化循环的好处显而易见：
- **精确的作用域清理（Drop / Defer Placement）**：当 `break even` 跳出循环时，该跳出点跨越了哪些词法块、需要按什么逆序清理哪些线性资源，在结构化树上一目了然；
- **多后端友好**：如果目标后端是要求结构化控制流的 GPU 着色器（SPIR-V / WGSL），可以直接一对一映射；如果目标后端是 LLVM，则在最后的 CFG Lowering 阶段再按需生成基本块与 SSA，各取所需！

### 2. 显式闭包捕获与环境关闭（Closures & Explicit Captures）

再看一段带有闭包捕获的代码：

```flurry
fn apply(step: fn(i32) -> i32, value: i32) -> i32 {
    return step(value);
}

fn show_shifted(base: i32) -> i32 {
    let shift = |value: i32| value + base;
    return apply(shift, 5);
}
```

在 IIR 阶段，`shift` 闭包既不是一个含糊不清的 AST 匿名函数，也没有被过早拆散成一堆无法辨认的裸函数指针加 `void*` 强转。它在 IIR 中是一个带有**显式捕获列表（Capture List）和捕获方式（按值移动 / 借用引用）**的高级闭包构造节点：

```scheme
(iir-let :local l_shift :name "shift"
  :ty (fn-ty (params i32) :ret i32)
  :init (iir-closure
          :captures ((capture :outer l_base :inner c_base :mode by-copy :ty i32))
          :params ((iir-param :local l_val :name "value" :ty i32))
          :ret-ty i32
          :body (iir-binop add (iir-local l_val) (iir-capture-ref c_base))))
```

正因为在 IIR 中保留了高层闭包语义：
- 如果后续优化器决定将 `apply(shift, 5)` 就地内联（Inlining），它可以瞬间把 `shift` 还原为 `5 + base`，**连闭包环境结构体都不用分配**！
- 只有当闭包真正逃逸或跨 ABI 传递时，随后的 **Closure Conversion（闭包转换）** 通道才会将其降级为具名的环境记录（Environment Record）与函数入口。

### 3. 错误返回（`!Err`）与代数效应（`#[eff]`）严格分立

还记得 `AGENTS.md` §5 中的另一条重要红线吗？

> **Capability 由 trait 协议和 elaboration 实现；错误结果与代数效应具有不同 ABI，函数签名不添加 `handles`。**

让我们看下面这段同时使用了代数效应 `#[read_number]` 和错误返回类型 `i32 !InputErr` 的代码：

```flurry
effect fn read_number() -> i32;

errdef InputErr {
    Negative(i32),
}

fn read_non_negative() -> i32 !InputErr {
    let value = #[read_number]();
    if value < 0 {
        return !InputErr.Negative(value);
    }
    return value;
}

fn demo() -> i32 !InputErr {
    return handle read_non_negative() with {
        read_number() => resume -3,
    };
}
```

在 IIR 中，这两种控制流机制拥有截然不同的高级节点与降码通道：

```
  [ IIR 中的双轨控制流表示 ]
  
  轨道 A: 错误结果 (!InputErr)
  - 节点: (iir-return-err :err-ty InputErr :variant "Negative" :payload (iir-local l_val))
  - 降码 ABI: 走确定性的函数返回值可区分联合通道（或寄存器状态位 + 错误负载），与调用栈帧生命周期同步！
  
  轨道 B: 代数效应请求 (#[read_number]() 与 handle ... with)
  - 节点: (iir-effect-perform :effect "read_number" :args ())
          (iir-effect-handle :body ... :clauses ((clause "read_number" (iir-resume (iir-lit -3 i32)))))
  - 降码 ABI: 传递 Handler 证据向量；在 demo() 中，因为 `resume -3` 是尾恢复（Tail-resumptive），
              Effect Lowering 通道可以直接将其编译为普通跳转/函数调用，零栈拷贝开销！
```

如果在进入 IIR 前就把它们混为一谈，不仅会破坏两者的 ABI 契约，还会让尾恢复效应（`resume -3`）失去就地化简为普通控制流的黄金机会！

### 4. 运行时依赖类型的同步重写（Co-transformation of `TypeRef`）

在上一期（第 186 期）我们学过，合法的运行时依赖类型（如 `Sensor<cal>`）在进入 IIR 时依然存在（其编译期部分已全部完成，运行时依赖通过 `TypeRef` 的替换表记录）。

因此，IIR 的各个降码通道（如函数内联、闭包环境打包、投影消解）在改写或重命名任何运行时变量绑定时，都遵守一条严格的**协同变换契约（Co-transformation Contract）**：
- 每当一个被依赖的运行时绑定 `R_cal` 被重命名、内联替换为实参表达式、或者打包进闭包环境结构体时，所有引用了 `R_cal` 的 `TypeRef` 替换表与 `EnvironmentPlan` 都会**同步更新**，保证直到最终擦除为底层机器表示之前，类型与值环境始终严丝合缝！

---

## 四、IIR 到后端的降码通道全景表

为了让大家对从 IIR 到最终机器码的旅程一目了然，我们将各降码通道的职责总结如下：

| 高级 IIR 特性 | 专属降码通道 (Lowering Pass) | 降码后的目标低级表示 |
| :--- | :--- | :--- |
| **运行时依赖类型 (`TypeRef`)** | Environment Materialization & Erasure | 按 `EnvironmentPlan` 生成显式字段/参数读取，随后擦除类型级空洞为物理内存布局 |
| **高级 `BitVec` 视图与算子** | Verified BitVec Synthesis（见第 189 期） | 经 SMT/BV 验证的最优目标字宽位指令序列（或安全回退至基准实现） |
| **流水线与数据布局投影 (`|>`)** | Pipeline Fusion & Projection Lowering | 融合循环与 SoA/AoS 索引偏移计算 |
| **代数效应 (`#[eff]` / `handle`)** | Effect Evidence & Handler Lowering | 尾恢复效应直接内联化简；非局部恢复降级为分段栈/协程延续传递 |
| **错误返回 (`T !E` / `return !E`)** | Error Result ABI Lowering | 目标调用约定下的双寄存器/Tagged Result 返回分支 |
| **高级闭包 (`iir-closure`)** | Closure Conversion | 具名环境结构体 + 静态代码入口函数指针 |
| **结构化循环与模式匹配** | Decision Tree & CFG / SSA Construction | 高效跳转表与最终面向 LLVM / 机器码的 CFG + SSA 基本块 |

---

## 五、边界与编译期拒绝：严禁未决编译期类型混入 IIR

虽然 IIR 允许保留合法的运行时依赖类型（如 `Sensor<cal>`），但**任何未完成的编译期类型计算或未决推断变量都绝不允许进入 IIR**。如果某个内部转换试图绕过检查将未决类型送入 IIR 构造器，IIR 验证器（IIR Verifier）会立即拒绝：

```flurry
pure comptime fn UnfinishedType(flag: bool) -> Type {
    return if flag { i32 } else { i64 };
}

-- 假设在某个具體实例中，编译期参数 flag 未能收敛为常量布尔值
fn bad_instance_entry(x: i32) -> i32 {
    let runtime_flag: bool = (x > 0);
    -- ❌ 编译期直接拒绝！`UnfinishedType` 的参数要求在编译期完成以确定具体值布局，不能将运行时布尔变量 `runtime_flag` 塞入非 lift 的纯编译期类型位置，禁止生成含未完成 comptime 类型的 IIR！
    let val: UnfinishedType(runtime_flag) = x;
    return val;
}
```

---

## 六、本期小结

1. **IIR 的核心定位**：类型的 `comptime` 部分 100% 完成，绝无活动推断变量，但保留所有高级语言结构（不预设为底层 SSA）。
2. **四大保留特征的价值**：结构化控制流保障资源清理与 GPU 后端映射；显式闭包支撑无分配内联；错误（`!Err`）与效应（`#[eff]`）分立保障各自最优 ABI；高级 `BitVec` 节点为位域代数综合保留完整数据流图。
3. **协同变换契约**：任何重写运行时绑定的 IIR 通道同步更新 `TypeRef` 替换表与 `EnvironmentPlan`。

现在，让我们把目光聚焦到上面降码表格中最具传奇色彩的一环——**`BitVec` 形式化程序综合器**！

当我们在代码里写出优雅的位切片、拼接、掩码以及就地写回（`word'bitvec[4..12] = word'bitvec[0..8]`）时，编译器是如何在保证**副作用不丢失、重叠写回不踩踏、`unsafe` 转换前置契约必检验**的前提下，自动搜索并形式化证明出一条最优 CPU 位操作指令序列的？如果遇到极其复杂的位运算导致求解器超时（返回 `Unknown`），编译器又如何保证 100% 稳健地回退（Fallback）？

在下一期（**第 189 期**，也是编译器原理篇的压轴之作）中，我们将亲手拆解 **带安全回退的 `BitVec` 形式化综合器**！我们下期见！
