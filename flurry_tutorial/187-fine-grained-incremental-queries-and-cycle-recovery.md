# 第 187 期：细粒度增量查询（Incremental Queries）、环路检测与错误恢复

> **系列导航**：[Flurry 语言漫游（200 期规划）](./README.md)  
> **当前卷目**：第八卷 · 编译器原理与全栈实战（181–200）  
> **上一期**：[第 186 期：运行时依赖类型的编译器表示——`TypeCore`、空洞替换与环境计划](./186-representing-runtime-dependent-types-in-the-compiler.md)  
> **下一期**：[第 188 期：为什么 IIR 在降码前要保留高级结构而非过早退化为 SSA？](./188-why-iir-preserves-high-level-structure-before-codegen.md)

---

## 一、开篇痛点：为什么我只改了函数体里的一个数字，整个工程却要重新类型检查？

想象你正在开发一个包含 500 个模块的大型系统。你打开其中一个底层工具函数 `adjust`，把函数体里的 `+ 1` 改成了 `+ 2`：

```flurry
fn adjust(value: i32) -> i32 {
    return value + 1; -- 修改为: return value + 2;
}

fn show_adjusted(value: i32) -> i32 {
    return adjust(value);
}
```

请注意：你只改了 `adjust` 的**运行时函数体（Body）**，它的**签名契约（Interface：`fn(i32) -> i32`）**连一根汗毛都没变！

### 初代尝试（`v0`）：按文件时间戳或粗粒度函数节点级联失效

在传统的批处理编译器或粗粒度增量系统中（`v0`），每个函数只是一个不可分割的黑盒节点 `CheckFn(adjust)`：
- 当你修改了 `adjust` 里的 `1 -> 2`，`CheckFn(adjust)` 失效重算；
- 因为 `show_adjusted` 调用了 `adjust`，`v0` 认为 `CheckFn(show_adjusted)` 依赖 `CheckFn(adjust)`，于是把 `show_adjusted` 的类型检查也全部推翻重算；
- 接着，调用了 `show_adjusted` 的上层百余个模块像多米诺骨牌一样全部失效重跑！

更隐蔽的痛点还在后面：
1. **构建配置（Build Config）依赖漏记**：如果某段代码写了 `if comptime (build.mode matches .debug) { ... }`，当用户把编译模式从 `debug` 切换到 `release` 时，源码文件一个字节都没改！如果增量缓存只看源码 Hash，就会直接复用旧的 `debug` 缓存，酿成严重的构建事故！
2. **“负面结果（Negative Results）”缓存不失效**：某个泛型调用原本因为找不到特化实现而回退到了默认实现；后来用户在另一个模块新增了一个特化 `impl`，如果增量系统当初没有记录“当时查过特化候选集且为空”的版本号，旧缓存就不会失效！
3. **死循环与错误恢复的冲突**：当用户在编辑器里写出 `const FIRST: u32 = SECOND + 1; const SECOND: u32 = FIRST + 1;` 时，编译器既要确定性地抓出环路报错，又要在 IDE 模式下允许其他无关函数继续获得代码补全，但**绝不能让错误的假占位符（比如偷偷塞个 `0`）混过检查流进后端生成错误机器码**！

---

## 二、机制迭代（`v0 -> v1 -> v2`）：三层依赖架构与早期截断（Early Cutoff）

为了实现真正工业级的按需增量编译，Luna 编译器首先在内部明确区分了**三类不同尺度的依赖结构**：

```
  +-----------------------------------------------------------------------+
  | 1. AST / HIR 包含树 (Containment Hierarchy)                           |
  |    职责：词法作用域嵌套、源码 Span 映射与快速局部语法重解析            |
  +-----------------------------------------------------------------------+
  | 2. 局部目标依赖图 (Local Goal Dependencies —— 见第 183 期)             |
  |    职责：单个推断区域内的 ?T 变量唤醒、候选探测与局部死锁预防          |
  +-----------------------------------------------------------------------+
  | 3. 持久化增量查询图 (Persistent Query Dependency Graph —— 本期主角!)   |
  |    职责：跨函数、跨模块、跨编辑会话的细粒度缓存复用与精确失效传播      |
  +-----------------------------------------------------------------------+
```

在持久化增量查询图中，Luna 采用了 **接口查询与实现查询分离 + 早期截断（Early Cutoff / Red-Green Algorithm）** 的设计（`v2`）：

### 1. 签名契约（Signature）与函数体（Body）彻底解耦

对于每一个函数（如 `adjust`），编译器将其拆分为粒度完全独立的查询键（Query Keys）：
- `FnSignature(adjust)`：只产出冻结后的函数签名、参数类型、`where` 契约与效应签名；
- `FnTypecheckBody(adjust)`：检查 `adjust` 的内部函数体是否符合其签名；
- `FnNormalizedBody(adjust)`：产出 `adjust` 的冻结函数体 IR（供内联或代码生成使用）。

现在，当我们把 `adjust` 函数体里的 `value + 1` 改成 `value + 2` 时，看看增量查询图发生了什么：

```
  [ 源码修改: adjust 函数体内的 `1` 改为 `2` ]
        |
        +---> 触发 FnSignature(adjust) 重验
        |     发现产出仍是: fn(i32) -> i32 (与旧指纹 100% 相同！)
        |     ====> 触发早期截断 (Early Cutoff: GREEN)！
        |           |
        |           v
        |     FnTypecheckBody(show_adjusted) 仅依赖 FnSignature(adjust)
        |     ====> 直接命中缓存跳过！上游 500 个模块的类型检查零开销复用！
        |
        +---> 触发 FnNormalizedBody(adjust) 重算
              产出新的函数体 IR: `value + 2` (指纹变化: RED)
                    |
                    v
              CodegenInstance(show_adjusted) (若内联了 adjust 的函数体)
              ====> 精准触发下游机器码重生成，绝不漏更！
```

这就是细粒度增量查询的威力：**`show_adjusted` 的类型检查因为只依赖 `FnSignature(adjust)` 而直接复用；而 `show_adjusted` 的内联代码生成因为依赖了 `FnNormalizedBody(adjust)` 而被精准重算**——既不多算一步冤枉路，也绝不漏掉任何受影响的二进制产物！

### 2. 显式记录构建配置与候选集版本依赖

在 Luna 中，任何影响语义的外部输入都必须是查询图中的显式节点：

```flurry
const ACTIVE_LIMIT: usize = if comptime (build.mode matches .debug) {
    64
} else {
    1024
};
```

当编译期求值器在计算 `ACTIVE_LIMIT` 并读取 `build.mode` 时，查询引擎会自动在 `EvalConst(ACTIVE_LIMIT)` 与 `BuildConfigQuery(mode)` 之间连上一条依赖边。一旦用户将构建命令从 `debug` 切为 `release`，`BuildConfigQuery(mode)` 指纹改变，所有依赖它的常量、条件编译类型和内联产物都会精准失效重算。

同理，当候选求解器（见第 184 期）查询某个 trait 的实现列表时，哪怕只查到了 0 个特化候选，它也会记录对 **`CandidateSetVersion(TraitHead)`** 的依赖。一旦任何地方新增或修改了一个 `impl`，该版本号递增，所有曾经依赖“当时没找到更特化候选”的查询都会自动重新核验！

---

## 三、深度法则：合法递归 vs 非法求值环路的确定性裁决

在增量查询引擎运行过程中，如果查询 `Q1` 在计算途中又层层调用回到了正在计算中的 `Q1`（即激活栈上出现了环路 $Q_1 \to Q_2 \to \dots \to Q_1$），编译器该怎么处理？

首先，我们要区分**合法的函数相互递归**与**非法的编译期值/类型计算死循环**。

### 场景 A：合法的函数相互递归（依赖签名，不构成求值环）

```flurry
fn is_even(n: u32) -> bool {
    return if n == 0 { true } else { is_odd(n - 1) };
}

fn is_odd(n: u32) -> bool {
    return if n == 0 { false } else { is_even(n - 1) };
}
```

为什么 `is_even` 和 `is_odd` 互相调用不会触发查询死循环？
因为在刚刚讲过的细粒度查询拆分下：
- 检查 `is_even` 的函数体（`FnTypecheckBody(is_even)`）只需要查询 `is_odd` 的**声明签名**（`FnSignature(is_odd)`）；
- 而 `FnSignature(is_odd)` 直接从源码头解析得出 `fn(u32) -> bool`，**根本不需要反过来查询 `FnTypecheckBody(is_even)`**！因此查询图是完全无环的（DAG）！

### 场景 B：非法的常量/类型求值死循环（确定性拦截，绝不伪造合法值！）

相反，如果两个常量在值计算上直接互相依赖：

```flurry
const FIRST: u32 = SECOND + 1;
const SECOND: u32 = FIRST + 1; -- ❌ 编译期直接拒绝！
```

此时查询栈会出现真实的闭环：

$$\text{EvalConst}(\text{FIRST}) \longrightarrow \text{EvalConst}(\text{SECOND}) \longrightarrow \text{EvalConst}(\text{FIRST})$$

当 Luna 的查询引擎检测到这一闭环时，它严格执行三条铁律：
1. **确定性环路规范化（Canonical Cycle Normalization）**：无论这次增量编译是从 `FIRST` 先开始算，还是从 `SECOND` 先开始算，编译器都会按照环路节点的稳定标识符对闭环路径进行规范化旋转，生成 100% 确定性的环路诊断报告，绝不因编译入口顺序不同而飘忽不定；
2. **绝不伪造合法默认值打破环路**：编译器**严禁**为了“把环打破”而偷偷给 `SECOND` 塞一个假的默认值 `0u32` 让 `FIRST` 算出 `1u32`！
3. **带污染标记的错误恢复占位符（Poisoned Recovery Placeholder）**：为了在 IDE 编写代码时，不因为这一处死循环就让整个文件的语法高亮和无关函数的类型提示瘫痪，查询引擎会产出一个显式的错误恢复节点 `ErrorValue(Cycle#1)`。任何读取了 `ErrorValue` 的下游查询都会自动继承 **`has_error = true`（已污染）** 标记——它允许无关的兄弟分支继续做静态分析，但在进入 IIR 降码和后端代码生成前会被硬性拦截，绝不可能产出伪造的可执行文件！

我们来看看这个环路错误在编译器中的 S-Expression 诊断转储：

```scheme
(diag-error
  :stage query-engine
  :code E0391
  :message "cyclic dependency detected during constant evaluation"
  :cycle-path (
    (query eval-const "FIRST")
    (query eval-const "SECOND")
    (query eval-const "FIRST")))
```

---

## 四、本期小结

| 核心设计 | 传统粗粒度缓存 (`v0`) | Luna 细粒度增量查询引擎 (`v2`) |
| :--- | :--- | :--- |
| **函数级依赖隔离** | 函数签名与函数体绑死，改一行函数体重跑全工程类型检查 | 拆分 `FnSignature` 与 `FnNormalizedBody`，函数体修改触发 Early Cutoff，上游类型检查直接复用 |
| **环境与配置追踪** | 漏记构建参数，切换 `debug/release` 易误用旧缓存 | `build.mode` 等编译期配置作为显式查询节点纳入依赖指纹 |
| **负面结果失效** | 新增特化 `impl` 后，旧的回退选择未能感知变化 | 显式记录对 `CandidateSetVersion` 的依赖，新增候选自动触发重验 |
| **相互递归与死循环** | 容易将合法函数互递归误判为环，或用假 `0` 掩盖常量死循环 | 互递归仅依赖 `FnSignature` 天然无环；真实求值环路规范化报错，并以带污染标记的恢复节点阻断后端降码 |

当所有的 Elaboration、编译期求值与增量查询全部通过检验后，定义被实例化为我们多次提到的 **IIR（Instantiated Intermediate Representation）**。

这时候，很多熟悉 LLVM 或传统编译器后端的同学会问：**“既然在 IIR 里，所有的编译期类型都已经算完了，为什么不干脆把 IIR 直接设计成类似 LLVM IR 的底层 SSA（静态单赋值）基本块形式，还要费劲在 IIR 里保留结构化循环、模式匹配、闭包捕获、错误与代数效应分离、以及高级 `BitVec` 节点？”**

在下一期（**第 188 期**）中，我们将深入探讨 **为什么 IIR 在降码前要保留高级结构而非过早退化为 SSA**！我们下期见！
