# Flurry 演进手记 (170)：第七卷总结——统一控制流与架构选型指南

> **阅读指南**：欢迎来到第七卷的收官篇。在过去的 29 篇手记（第 141 篇至第 169 篇）中，我们系统梳理了 Flurry 的控制流体系：从局部结构化控制流、恢复性错误行（`!Err`）、分级代数效应与界限延续（`#Eff`），到结构化异步（`async effect`）与软件事务内存（STM `atomic`）。
>
> 本篇将用一张全卷知识图谱和一段综合工程示例，总结 Flurry 控制流的设计边界与选型方法。

---

## 一、 全卷全景图：Flurry 控制流四大支柱的正交分工

Flurry 按照**“控制流是否跨函数边界”、“是否允许恢复回原调用点”、“是否伴随事务状态回滚”**三个正交维度，将控制流划分为职责清晰的四大支柱：

```
=====================================================================================
                     Flurry 第七卷：统一控制流四大支柱全景图
=====================================================================================

  【支柱一：函数内局部结构化控制流】
   ├── 分支与解构：`if cond { ... }`、`if expr is pattern do { ... }`、`let pat = expr else`
   ├── 后缀模式匹配：`expr match { pat => ... }`（顺应数据流从左到右阅读）
   └── 语句级后置守卫：`return/break/continue expr? while cond;`、
                       `resume expr? while cond;`（减少深层条件嵌套）
          │
          ▼ (需要跨越函数边界向上传递？)
  ┌───────┴─────────────────────────────────────────────────────────────────┐
  │                                                                         │
  ▼ 【单向退栈，不回到原调用点】                   ▼ 【请求外层处理器，支持就地或挂起恢复】
  【支柱二：恢复性错误行 `!Err T`】                【支柱三：代数效应与界限延续 `#Eff T`】
   ├── 行类型：唯一标签行（自动合并去重）           ├── 行类型：保留同名层次行（支持嵌套遮蔽路由）
   ├── 抛出与传播：`error e`、后缀 `expr!`          ├── `direct_passing`：只读上下文直传
   ├── 局部处理：`expr! { .Variant => ... }`        ├── Direct / `inline effect`：原地调用或协同内联
   ├── 处理器复用：`let h handles E = ...; .use(h)` ├── `catch continuation`：定界延续与局部地址稳定
   └── 底层 ABI：独立的值返回通道，与延续正交       └── `async effect` + `.use(runtime)` + `.await`：
                                                               结构化异步并发与取消清理
                                                                │
                                                                ▼ (需要共享内存的原子更新与冲突回滚？)
                                                    【支柱四：纯度受控的软件事务内存 STM】
                                                     ├── 事务块与函数：`atomic(vars) { ... }`、`atomic fn`
                                                     └── 编译期纯度边界：仅允许值纯（P4/P5）与受管事务读写，
                                                         禁止不可回滚的外部 I/O（P0）或未消去外部效应逃逸
=====================================================================================
```

---

## 二、 综合实战：构建带审计追踪、事务扣减与配额熔断的结算管线

为了把第七卷的核心机制串联起来，我们来看一个配额结算管线的设计需求：
1. 从上下文中读取当前租户的审计追踪 ID（只读上下文）；
2. 在内存事务 `atomic(primary, backup)` 中原子扣减共享账户配额，若主账户余额不足则自动尝试备用账户；
3. 将每一笔扣款汇报给外部监控流，由外层处理器决定是否继续处理下一笔（支持条件熔断）；
4. 遇到非法订单金额或配额耗尽时，通过恢复性错误通道 `!SettleErr` 返回结构化错误。

### 反例：在 `atomic` 事务块内混入未处理的外部效应

如果把外部效应请求直接写进 `atomic(...) { ... }` 事务块内，编译器会因违反事务纯度边界而直接拒绝：

```flurry
effect emit_settled(trace_id: u64, amount: u64) -> bool;

enum SettleErr {
    ZeroAmount,
}

-- ❌ 错误示例：在 STM 事务块内发起未消去的外部效应
fn bad_settle_one(
    primary: *u64,
    backup: *u64,
    amount: u64
) -> !SettleErr #[emit_settled] void {
    return error SettleErr.ZeroAmount while amount == 0u64;

    atomic(primary, backup) {
        if primary.* >= amount {
            primary.* -= amount;
        }
        -- ❌ 编译期拒绝：
        -- `atomic` 事务在遇到并发冲突时会回滚并重试，
        -- 未在事务内部闭合消去的外部效应 `emit_settled` 不能在 `atomic` 块中触发
        let keep_going = emit_settled(1001u64, amount)#;
    }
}
```

### 正例：四大控制流支柱协同工作

遵循四大支柱的职责边界：
- 将 `atomic(primary, backup)` 与 `atomic fn` 收敛为纯粹的受管状态更新；
- 用 `^attrs.effects.direct_passing` 传递只读的 `trace_id`；
- 用 `!SettleErr` 传递单向业务错误，并在批量入口用 `expr! { .ZeroAmount => () }` 局部处理零金额项；
- 在外层效应处理器中使用 `resume true while accumulated <= max_total_deducted;` 表达配额熔断。

```flurry
use std.prelude.*;

^attrs.effects.direct_passing
effect current_trace_id() -> u64;

effect on_settled(trace_id: u64, deducted: u64) -> bool;

enum SettleErr {
    ZeroAmount,
    InsufficientBalance,
    QuotaExhausted,
}

-- 1. 原子子操作：尝试从单个账户扣减配额
atomic fn try_deduct(slot: *u64, amount: u64) -> bool {
    return false while slot.* < amount;
    slot.* -= amount;
    true
}

-- 2. 组合事务：优先扣减主账户，不足时扣减备用账户
fn deduct_with_fallback(primary: *u64, backup: *u64, amount: u64) -> bool {
    atomic(primary, backup) {
        return true while try_deduct(primary, amount);
        try_deduct(backup, amount)
    }
}

-- 3. 单笔结算：结合后置守卫、STM 事务、Direct Passing 与效应汇报
fn settle_item(
    primary: *u64,
    backup: *u64,
    amount: u64
) -> #[current_trace_id, on_settled] !SettleErr void {
    -- 后置守卫 + 错误返回
    return error SettleErr.ZeroAmount while amount == 0u64;

    -- 在纯事务内完成扣款
    let ok = deduct_with_fallback(primary, backup, amount);
    return error SettleErr.InsufficientBalance while not ok;

    -- 读取当前追踪 ID（direct_passing）
    let tid = current_trace_id()#;

    -- 向外层处理器汇报进度，并根据返回值决定是否中止
    let should_continue = on_settled(tid, amount)#;
    error SettleErr.QuotaExhausted while not should_continue
}

-- 4. 批量编排：局部过滤 ZeroAmount 错误，保留其余错误向上传播
fn run_two_items(
    primary: *u64,
    backup: *u64,
    first: u64,
    second: u64
) -> #[current_trace_id, on_settled] !SettleErr void {
    settle_item(primary, backup, first)! {
        .ZeroAmount => (),
    }
    settle_item(primary, backup, second)! {
        .ZeroAmount => (),
    }
}

fn execute_batch(
    primary: *u64,
    backup: *u64,
    max_total_deducted: u64
) -> !SettleErr u64 {
    var accumulated = 0u64;

    run_two_items(primary, backup, 0u64, 60u64)# {
        current_trace_id() => 9527u64,
        on_settled(tid, deducted) => {
            accumulated += deducted;
            -- 当累计扣款未超标时提前恢复 true，否则由分支尾表达式返回 false
            resume true while accumulated <= max_total_deducted;
            false
        },
    }!

    accumulated
}

test {
    var primary = 50u64;
    var backup = 100u64;

    let total = execute_batch(primary.ref, backup.ref, 80u64)! {
        .ZeroAmount => 0u64,
        .InsufficientBalance => 0u64,
        .QuotaExhausted => 0u64,
    }
    assumes total == 60u64;
    assumes primary == 50u64;
    assumes backup == 40u64;
}
```

---

## 三、 工程选型速查指南

在实际编写 Flurry 代码时，可以按以下原则选择合适的控制流机制：

1. **出错后只需单向退出、不需要回到原调用点继续执行**：使用 **`!Err T`（恢复性错误行）** 配合 `expr!`、就地 `! { ... }` 或 `HandleError`（`.use(handler)`）。
2. **跨多层调用栈传递只读配置、日志标识或追踪 ID**：使用 **`^attrs.effects.direct_passing` 的 `#Eff`**，兼顾静态效应行检查与直接参数传递开销。
3. **向调用方发出策略询问或事件通知，且处理器总是当场恢复**：使用 **普通 Direct Handler**；若处于高频循环热路径，可声明为 **`inline effect`** 进行协同内联。
4. **需要在中途挂起并稍后恢复（如流式生成器、回溯搜索、自定义协程调度）**：使用 **`catch continuation: builtin.Continuation`** 定界延续。
5. **并发异步 I/O 或多核共享内存原子更新**：并发 I/O 使用 **`async effect` + `.use(runtime)` + `.await`**；多变量原子更新使用 **STM `atomic(vars) { ... }` 与 `atomic fn`**。

---

## 本篇小结（暨第七卷核心机制总表）

| 控制流类别 | 核心语法 | 类型与语义特征 | 执行模型特征 |
| :--- | :--- | :--- | :--- |
| **结构化分支与守卫** | `if`、`expr match`、`let ... else`、`return/break/continue/resume ... while` | 穷尽性检查、模式绑定、扁平化提前退出或恢复 | 局部基本块跳转 |
| **恢复性错误 (`!Err`)** | `error e`、`expr!`、`expr! { ... }`、`.use(handler)` | **唯一标签行（Unique-Label Row）**，自动合并去重 | 独立值返回 ABI，函数签名不加 `handles` |
| **只读上下文效应** | `^attrs.effects.direct_passing effect` | 无参数、不捕获延续，安装时求值一次 | 隐式参数直传，请求点直接读取 |
| **直接/内联处理器** | `expr# { op() => { ... resume v; } }`、`inline effect` | **保留同名层次行（Layered-Occurrence Row）**，尾部就地恢复 | 普通函数调用或协同内联，无栈切换 |
| **定界延续** | `catch continuation: builtin.Continuation` | 一次性仿射资源（One-Shot Affine），支持放弃时自动逆序 `Drop` | 定界切片保持局部变量 `.ref` 地址稳定 |
| **结构化异步效应** | `async effect` + `.use(runtime)` + `.await` | 请求点返回 `Future<T>`，在 `.await` 处按需挂起 | 就绪值走快路径，挂起复用定界延续与取消清理 |
| **软件事务内存 (STM)** | `atomic(vars) { ... }`、`atomic fn` | 强制纯度边界，禁止不可回滚的外部 I/O 与未消去效应逃逸 | 乐观读写日志、冲突验证与自动重试 |

---

### 下一卷预告（第八卷：异构硬件投影与系统架构实战）

到这里，我们已经掌握了 Flurry 在通用 CPU 上的核心语言机制：从资源绑定、所有权与内存视图、编译期元编程与依赖类型（PTC），一直到本卷的统一控制流。

在最终卷 **第八卷（第 171 篇至第 200 篇）** 中，我们将把视野扩展到异构硬件与完整工程架构：
1. 探索如何通过**流水线投影操作符（Pipeline Projection `~>`）**、硬件内建指令（`builtin.intrinsics`）与 GPU Kernel 映射，将计算安全地投射到 SIMD 与加速器硬件上；
2. 掌握多包工作区（`anar.toml` / `package.fl`）、构建脚本与可见性封装；
3. 最后通过一个跨 10 篇的 **Orion 异构实时流式信号处理引擎** 综合实战，将全书八卷的知识融会贯通。
