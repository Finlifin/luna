# Flurry 演进手记 (165)：资源池租借（Arena / Pool Lease）与 Handler 作用域生命周期绑定

> **阅读指南**：在上一回（第 164 回）末尾，我们提出了系统编程中关于 **Arena（区域内存池）与对象池租借（Pool Lease）** 的终极安全拷问：当业务代码通过代数效应从当前 Handler 安装的局部 `Arena` 或 `StackPool` 中借出一块内存视图/指针时，怎样保证在计算过程中可以畅行无阻地使用它，但**绝不允许任何指向该池的指针或未归还的 Continuation 逃逸出该池的存活作用域**？本篇我们将结合 `docs/algebraic-effect-and-delimited-continuation/handling-capability.md`、`effect-abi.md` 与第三卷的核心武器 `docs/resource-management/reachability.md`，见证 **可达性限定（`&R T`）与效应 Handler 作用域** 的天作之合！

---

## 一、 痛点：Arena 分配器的“1 纳秒天堂”与“悬垂指针地狱”

在写编译器前端、游戏引擎帧循环或高频网络网关时，**Arena（竞技场/区域内存池）** 是公认的性能神器：
- 分配内存时，只需要把游标指针往前推一下（Bump Pointer），耗时不到 1 纳秒；
- 释放内存时，不需要逐个对象调用 `free`，等整个阶段（即 Handler 作用域）结束时，**一把将游标归零或释放整块缓冲区**！

### 审视 `v0`：当通过效应借出的指针偷偷溜出了 Handler 的大门

如果我们用一个简单的代数效应 `effect alloc_slot() -> *u32` 来从局部池里分配槽位，却没有可达性（Reachability）系统的保护（`v0`），灾难瞬间就会发生：

```flurry
-- 假想 v0：如果没有可达性依赖追踪 (&R T)
-- 1. 我们在函数内部创建了一个局部内存池 var arena = LocalArena { ... };
-- 2. 在业务代码块里通过 alloc_slot()# 拿到了一个指向 arena 内部缓冲区的指针 ptr: *u32
-- 3. 业务代码块居然直接把 ptr 当作返回值 return 了出去！
-- 4. 离开 Handler 作用域后，local arena 被整块销毁重置，
--    外层拿到的 ptr 瞬间变成指向已销毁内存的悬垂指针（Dangling Pointer / Use-After-Free）！
```

同样的道理也发生在我们在第 150 回和 `effect-abi.md` 学过的 **显式栈池实例（`.use(pool)`）** 上：如果一个捕获到的 `builtin.Continuation` 持有着从局部 `pool` 分配出来的执行栈，而有人企图先把 `pool` 销毁、却让那个 `continuation` 逃逸到外面继续存活，一旦恢复执行就会直接踩进死掉的栈内存！

---

## 二、 破局（v1）：有状态 Handler + `&R T` 可达性限定——读出数值可带走，借出指针锁门内！

Flurry 是如何在不引入繁琐生命周期体操的前提下，彻底封死这类池逃逸漏洞的？
答案就藏在第三卷 `docs/resource-management/reachability.md` 与第七卷 `docs/algebraic-effect-and-delimited-continuation/handling-capability.md` 的联动法则里：

> **💡 核心心智模型：“租借的视图带着池的烙印，独立的数值自由离开！”**
> 1. **取址与视图传播来源要求（Reachability Propagation）**：
>    当 Handler 分支从局部对象 `arena`（或 `pool`）中取出一个指针或切片视图（例如 `arena.slot.ref`）交付给效应请求时，该指针携带着对 `arena` 实例的可达性依赖 **`&arena *u32`**。无论你在业务代码里把这个指针复制多少次、塞进元组还是传给辅助函数，这份“我依赖 `arena` 存活”的烙印都如影随形！
> 2. **跨作用域交付审查（Cross-Scope Delivery Check）**：
>    当 `# { ... }` 处理块结束、且局部 `arena` 即将随作用域退出而终结（Kill / Drop）时，编译器会审查从该块交付出去的**返回值**和**写入外部对象的状态**：
>    - 如果返回值只是一个**读取出来的独立标量数值**（如 `u32`、`bool`，或者拥有独立堆内存的 `Vec<i32>`），它不再依赖局部 `arena` 的存储，**100% 放行带走**！
>    - 如果返回值里还夹带着任何依赖局部 `arena` 的指针 `*u32`、切片视图或未归还栈槽的 `Continuation`，编译器会立刻发现“结果依赖即将结束的局部资源”，**在编译期当场拦截**！

让我们用一张 Code Cartoon，看清“池内自由租借使用”与“池外严格禁止逃逸”的边界：

```text
  ┌─ 局部池与 Handler 作用域 (Arena / Pool Scope) ──────────────────────────┐
  │ var pool = ScratchArena { slot: 42u32 };                                │
  │                                                                         │
  │ let final_number: u32 = (do {                                           │
  │     -- 1. 通过效应租借指向 pool 内部的指针视图：                        │
  │     let lease: &pool *u32 = borrow_slot()#;                             │
  │     -- 2. 在作用域内部自由读写：                                        │
  │     lease.* = lease.* + 8u32;                                           │
  │     -- 3. 解引用读取出独立的标量值 u32 (50u32) 作为块结果交付！         │
  │     lease.*                                                             │
  │ }# {                                                                    │
  │     borrow_slot() => pool.slot.ref,  -- 交付带 &pool 依赖的指针         │
  │ });                                                                     │
  │                                                                         │
  │ ◀── 出口审查：                                                          │
  │     ✅ 交付的 50u32 是独立标量，不依赖 pool 存储，安全带出！            │
  │     ❌ 若企图直接把 `lease` (*u32) 或装有 `lease` 的 Vec<*u32> 带出，   │
  │        编译器检测到 &pool 在 pool 终结后失效，当场报错拦截！            │
  └─────────────────────────────────────────────────────────────────────────┘
```

### 演进到 `v1`：有状态 Trait 接入效应 + 池内计算交付独立数值

让我们结合 `docs/algebraic-effect-and-delimited-continuation/handling-capability.md` 中的有状态接口模式，写一段在局部池中分配/累加状态并安全交付独立标量结果的代码：

```flurry
use std.prelude.*;

-- 定义有状态资源池的分配与计费接口
trait ScratchLease {
    fn allocate_and_add(*self, delta: u32) -> u32;
}

struct ArenaPool {
    current_Acc: u32,
    alloc_count: u32,
}

impl ScratchLease for ArenaPool {
    fn allocate_and_add(*self, delta: u32) -> u32 {
        self.alloc_count.* += 1u32;
        self.current_Acc.* += delta;
        self.current_Acc.*
    }
}

effect lease_add(delta: u32) -> u32;

fn run_batch_in_arena() -> #[lease_add] u32 {
    let step1 = lease_add(15u32)#;
    let step2 = lease_add(27u32)#;
    step1 + step2
}

test {
    -- 创建局部 ArenaPool 实例，在 Handler 中将效应请求接到同一个 pool 对象
    var pool = ArenaPool {
        current_Acc: 0u32,
        alloc_count: 0u32,
    };

    let result = run_batch_in_arena()# {
        lease_add(delta) => pool.allocate_and_add(delta),
    };

    -- 输出：result = 57 (15 + 42), alloc_count = 2
    println("result: {}, allocations: {}", result, pool.alloc_count);
}
```

---

## 三、 严格拦截：企图让局部池指针或栈池 Continuation 逃逸出作用域（v2）

现在，让我们看看当有人企图把指向局部池存储的指针 `*i32`（无论是直接返回，还是偷偷塞进一个堆分配的 `Vec<*i32>` 容器里）带出局部池的作用域时，Flurry 的可达性检查器是如何像激光网一样精准拦截的（见 `docs/resource-management/reachability.md`）：

```flurry
use std.prelude.*;

-- 负例 1：企图直接将局部存储的地址逃逸出作用域
fn broken_escape_address() -> *i32 {
    let local_slot = 23i32;

    -- ❌ 编译期直接拒绝！(Reachability Escape Error)
    -- 返回的指针 *i32 依赖即将随函数退出而终结的局部存储 local_slot！
    local_slot.ref
}

-- 负例 2：企图把局部地址藏进堆容器 Vec<*i32> 里蒙混过关
fn broken_escape_in_vec() -> Vec<*i32> {
    let values = Vec.new();
    let local_slot = 23i32;
    values.push(local_slot.ref);

    -- ❌ 编译期直接拒绝！
    -- 虽然 Vec 自己的堆缓冲区可以转交所有权，但它的元素类型 *i32
    -- 依然携带着对局部存储 local_slot 的外部可达性要求！
    -- 把局部地址装进堆容器绝不会延长局部对象的生命！
    values
}
```

### 演进到 `v2`：用 `&R1 &R2` 显式声明跨函数边界的保守可达性上界

如果我们的辅助函数确实需要接收两个来自不同池或缓冲区的视图 `&R1 str` 和 `&R2 str`，并从中挑一个返回（只要这两个池在调用者手里还活着，返回的视图就完全合法！），我们只需要在签名上用 `&R1 &R2 str` 写出它们的**保守可达性并集上界**即可：

```flurry
use std.prelude.*;

-- 演进到 v2：显式声明返回值依赖 R1 与 R2 的可达性并集上界
fn select_longer_lease(
    first: &R1 str,
    second: &R2 str,
) -> &R1 &R2 str
where
    R1: Type.vars.ReachSet,
    R2: Type.vars.ReachSet,
    outcomes read(first) and read(second)
{
    if first.len() >= second.len() {
        first
    } else {
        second
    }
}

test {
    let pool_a = "short".to_string();
    let pool_b = "longer_lease".to_string();

    -- ✅ 在 pool_a 和 pool_b 均存活的作用域内，返回的视图 100% 安全可用！
    let chosen = select_longer_lease(pool_a.as_str(), pool_b.as_str());
    println("chosen lease: {}", chosen);
}
```

> **🔍 深度补充：栈池实例（`.use(pool)`）的可达性铁律**
> 正如 `docs/algebraic-effect-and-delimited-continuation/effect-abi.md` 所述：
> 当你使用 `(work()# { ... }).use(pool)` 接入显式栈池时，捕获到的任何 `builtin.Continuation` 都会在可达性图中记录对 `pool` 存储实例的依赖。**`pool` 的有效期必须完整覆盖所有尚未归还的执行栈及其关联依赖**——编译器绝不允许你先把 `pool` 销毁，却让依赖它的 `continuation` 逃逸出去！

---

## 本篇小结

| 场景 / 操作 | 可达性与效应规则（`reachability.md` & `effect-abi.md`） | 编译期安全裁决 |
| :--- | :--- | :--- |
| **池内借用并交付独立数值** | 指针仅在 Handler 作用域内读写，最终读取出标量 `u32` 或独立拥有型 `Vec<i32>` | ✅ 标量不附着原存储槽位依赖，允许自由离开局部池作用域 |
| **直接返回局部池指针** | 返回值 `*i32` 携带对即将销毁的局部存储的可达性要求 | ❌ 编译期直接拒绝（防止 Use-After-Free） |
| **将局部指针装入 `Vec<*i32>` 返回** | 容器自有缓冲区可转交，但元素仍暴露对局部对象的外部可达性要求 | ❌ 编译期直接拒绝（堆容器不延长所指局部对象的生命周期） |
| **显式栈池 `.use(pool)`** | `Continuation` 持有的每段栈记录其来源 `pool` 依赖 | ✅ 强制保证在所有挂起栈完成清理归还前 `pool` 始终有效 |

---

### 留给下一篇的悬念

在前面的章节里，当我们在效应 Handler 分支里想要恢复被挂起的计算时，我们通常写的是无条件的 `resume val;`，或者套一层 `if`：
```flurry
if quota_remaining > 0u32 {
    resume val;
}
```
但是，你还记得我们在第一卷（第 014 回）学过的 Flurry 标志性语法——**后缀 `while` 语句守卫（Statement Guard：`return expr while cond;`、`break while cond;`）** 吗？

翻开 `docs/grammar/syntax` 的第 459 行，你会发现一个令人惊喜的语法彩蛋：
```text
resume_statement -> resume expr? (while expr)?
```
没错！**`resume` 后面也可以直接挂接 `while` 守卫：`resume val while cond;`！**

当我们在编写状态机驱动器、API 速率限制器（Rate Limiter）或硬件看门狗熔断器时，`resume val while cond;` 会怎样让代码变得像诗一样紧凑？当 `while` 条件为 `false` 跳过 `resume` 时，后续的兜底语句与挂起栈的清理又是如何无缝衔接的？

下一篇 **《Flurry 演进手记 (166)：当 `resume` 附带 `while` 守卫——条件恢复与状态机驱动》**，我们将解锁这个精妙的语法组合技！
