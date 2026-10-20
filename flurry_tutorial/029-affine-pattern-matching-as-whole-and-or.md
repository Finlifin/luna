# Flurry 演进手记 (029)：既要检查字段又要搬走整个包——仿射类型的 as whole 与 or 模式所有权汇合

在 [上一篇 (028)](./028-chained-matching-with-and-is-and-observation-blocks.md) 的结尾，我们抛出了一个让许多系统程序员头皮发麻的场景：**当模式匹配遇上不可随意复制的仿射类型（Affine Type / Once 资源）时，怎么做到既能检查或提取字段，又不把整个对象拆碎？**

来看我们固件里的真实需求：
我们有一个泛型数据包 `Packet<T>`，里面包含一个布尔标志 `ready: bool` 和一个载荷 `payload: T`：
```flurry
struct Packet where T {
    ready: bool,
    payload: T,
}
```
注意：这里的载荷 `T` 可能是一个**独占的硬件 DMA 授权票据 `Ticket`（没有实现 `Copy`，只能使用一次）**。

现在，我们需要实现三个操作：
1. **看一眼标志就整体移交**：用模式匹配检查 `packet` 的 `ready` 是不是 `true`，如果是，就把**完整的 `Packet<T>` 原封不动地移交（Move）返回**；
2. **抽出 `Copy` 字段的同时整体移交**：把 `ready` 布尔值提取出来绑定到变量 `ready`，同时还要把**完整的 `Packet<T>`** 绑定到变量 `whole` 一并移交出去。
3. **在 `or` 模式的不同位置汇合仿射资源**：当匹配一个包含多个 `Ticket` 的元组或枚举时，比如 `(true, item, _) or (false, _, item)`，第一个候选移走第 1 张票据，第二个候选移走第 2 张票据，**那个被 `_` 忽略的剩余票据由谁来负责销毁？**

让我们从传统写法遇到的墙开始，一步步看 Flurry 的模式匹配是如何精准调度每一份所有权的（详见 `docs/control-flow/pattern-matching/index.md`）。

---

## 一、 拆开了就装不回去的痛苦

如果我们不知道整体绑定模式 `as`，或者带着“一旦在模式里写了 `{ ready: true }` 就把结构体拆散了”的旧观念去写代码，写出来的 传统要么得手动把拆散的字段重新装回一个新的 `Packet`，要么在提取字段后发现原对象已经残缺不全（Partially Moved）了：

```flurry
use std.prelude.*;

struct Packet where T {
    ready: bool,
    payload: T,
}

-- 为了检查 ready == true 并返回整个 Packet，笨拙地拆开再重新拼装
fn retain(packet: Packet<T>) -> Packet<T> where T {
    packet match {
        { ready: true, payload: p } => {
            -- 问题背景：如果 Packet 有 10 个字段，或者包含私有字段、甚至实现了 Drop（禁止拆字段移出），
            -- 这种“全拆开再重装”的做法当场就会崩溃。
            Packet<T> { ready: true, payload: p }
        },
        other => other,
    }
}
```

我们需要一种告诉编译器的语法：**“我写的 `{ ready: true }` 只是走过去看一眼（Observe）它的标志位，并没有把它的肚子剖开拿走字段；一旦匹配成功，请把完整的原对象绑给 `whole`。”**

---

## 二、 观察模式与 `pat as whole` 整体交付

在 Flurry 中，模式后缀 **`as name`** 允许你在匹配左侧模式的同时，将**整个被匹配的值**绑定到 `name`。

这里有两条极其精妙的所有权核心规则（详见 `docs/control-flow/pattern-matching/index.md` §观察模式与整体绑定的所有权）：

1. **纯观察不消耗所有权**：当左侧模式只检查字面量、变体标签或用 `_` 忽略字段（没有导出内部字段绑定）时，它根本不会索取内部字段的所有权。因此 `{ ready: true } as whole` 匹配成功时，直接将完整的仿射对象 `Packet<T>` 移动给 `whole`。
2. **额外绑定 `Copy` 字段可与整体移动共存**：那如果我们在左侧写了 `{ ready } as whole`，既要把 `ready` 绑定出来，又要把整个 `Packet<T>` 移交给 `whole` 呢？
   只要被额外绑定的字段 `ready` 具备 **`Copy` 能力**（如 `bool`、`u32`），编译器就会**先复制一份 `ready` 字段的值，同时把完好无损的整个仿射对象 `Packet<T>` 移动给 `whole`**。

下面对比合法的“`Copy` 字段复制 + 整体移动”与非法的“仿射字段重叠索取”：

```text
  Packet<T> (仿射对象，其中 ready: bool 是 Copy，payload: T 是非 Copy 仿射资源)
  ┌──────────────────┬─────────────────────────────┐
  │ ready: bool      │ payload: T (Once / Affine)  │
  └────────┬─────────┴──────────────┬──────────────┘
           │                        │
  [场景 A] let { ready } as whole = packet;  (✅ 完全合法。)
           ├──► 复制一份 bool 交给 `ready`
           └──► 整個 Packet<T> 完好无损，整体 Move 交给 `whole`。

  [场景 B] let { payload: item } as whole = packet;  (❌ 编译期直接拒绝。)
                                    ├──► 试图把非 Copy 的 payload 移给 `item`
                                    └──► 又试图把包含 payload 的完整 Packet 移给 `whole`
                                         💥 一份独占资源不能掰成两半交出两次。
```

来看看用 `as whole` 写出的 改进代码有多么清爽：

```flurry
use std.prelude.*;

struct Packet where T {
    ready: bool,
    payload: T,
}

-- 1. 只观察 ready: true，成功时直接把完整的 Packet<T> 移交给 whole
fn retain(packet: Packet<T>) -> Packet<T> where T {
    packet match {
        { ready: true } as whole => whole,
        whole => whole,
    }
}

-- 2. 复制 Copy 字段 `ready`，同时把完整的仿射对象 `Packet<T>` 移交给 `whole`。
--    注意：这里只要求 ready 的 bool 类型可 Copy，完全不要求 T 实现 Copy。
fn inspect_ready(packet: Packet<T>) -> Tuple<bool, Packet<T>> where T {
    let { ready } as whole = packet;
    (ready, whole)
}
```

```flurry
-- ❌ 编译期直接拒绝：不能在把非 Copy 载荷绑给 item 的同时，又用 as whole 拿走整个 Packet。
fn bad_overlap_move(packet: Packet<T>) -> Tuple<T, Packet<T>> where T {
    let { payload: item } as whole = packet;
    (item, whole)
}
```

---

## 三、 `or` 模式中的仿射所有权汇合与动态残余清理

现在，我们进入本篇关键部分：**当 `p1 or p2` 选择模式遇上仿射资源时，编译器是怎么管账的？**

先看最简单的情况：两个候选分支从**同一个物理字段**移出仿射值，只是书写顺序或条件不同：

```flurry
fn unpack(packet: Packet<T>) -> T where T {
    packet match {
        -- 无论走左边还是右边，item 都来自同一个 payload 字段，只会在成功时移出一次。
        { ready: true, payload: item }
            or { payload: item, ready: false } => item,
    }
}
```

这很容易理解。但如果两个 `or` 分支**从不同的字段位置、甚至是不同的枚举变体中**移出那张不可复制的 `Ticket` 呢？

来看看这段代码：

```flurry
use std.prelude.*;

-- Ticket 是没有实现 Copy 的独占仿射资源
struct Ticket {
    id: u32,
}

impl Drop for Ticket {
    fn drop(*self) {
        println("Dropping unused ticket #{}", self.id.*);
    }
}

fn consume_ticket(t: Ticket) {
    println("Consuming selected ticket #{}", t.id);
    -- t 在函数结束时被销毁
}

-- 场景 1：在同一元组的不同位置选择移出一个 Ticket
fn choose_from_tuple(flag: bool) {
    let pair = (flag, Ticket { id: 1u32 }, Ticket { id: 2u32 });
    pair match {
        -- 如果 flag 为 true，移走第 1 张票据，留下的第 2 张票据需自动清理；
        -- 如果 flag 为 false，移走第 2 张票据，留下的第 1 张票据需自动清理。
        (true, item, _) or (false, _, item) => consume_ticket(item),
    }
}

-- 场景 2：在不同枚举变体的不同载荷布局中移出一个 Ticket。
enum TicketBundle {
    pair(Ticket, Ticket),
    tagged(bool, Ticket, Ticket),
}

fn choose_from_enum(bundle: TicketBundle) {
    bundle match {
        -- .pair 移走第 1 个 Ticket（剩第 2 个需清理）；
        -- .tagged 移走最后 1 个 Ticket（剩前 1 个需清理）。
        .pair(item, _) or .tagged(_, _, item) => consume_ticket(item),
    }
}

test {
    println("--- Tuple choice (true) ---");
    choose_from_tuple(true);

    println("--- Enum choice (.tagged) ---");
    choose_from_enum(TicketBundle.tagged(
        true,
        Ticket { id: 10u32 },
        Ticket { id: 20u32 },
    ));
}
```

停下来想一想：在 `choose_from_tuple` 和 `choose_from_enum` 的 `=> consume_ticket(item)` 分支体里，代码只有一份。
可是：
- 如果是通过左边候选 `(true, item, _)` 进来的，`pair` 里已经移空了的是第 1 张票，**还留在原地等着被 `Drop` 的是第 2 张票（`#2`）**；
- 如果是通过右边候选 `(false, _, item)` 进来的，`pair` 里已经移空了的是第 2 张票，**还留在原地等着被 `Drop` 的是第 1 张票（`#1`）**。

编译器是怎么保证既**不会漏掉那个被 `_` 忽略的 `Ticket`（造成内存/资源泄漏）**，又**不会把已经移交给 `item` 的那个 `Ticket` 重复销毁两次（Double Free）**的？

如下所示，揭秘 Flurry 所有权检查器的“动态残余清理计划（Residual Cleanup Plan）”：

```text
  主语: pair = (flag, Ticket #1, Ticket #2)
                         │
        ┌────────────────┴────────────────┐
        ▼ (若命中第 1 候选)                ▼ (若命中第 2 候选)
  (true, item, _)                   (false, _, item)
   • item ◄── Move Ticket #1         • item ◄── Move Ticket #2
   • 记录残余状态:                    • 记录残余状态:
     [Slot 1: 已移出, Slot 2: 存活]    [Slot 1: 存活, Slot 2: 已移出]
        │                                 │
        └────────────────┬────────────────┘
                         ▼
         执行共同分支体: consume_ticket(item)
         (在函数内用完并 Drop 掉被选中的 item)
                         │
                         ▼
         退出 match 时执行【残余主语清理计划】：
         • 若当初走的是第 1 候选 ──► 只 Drop 仍存活的 Slot 2 (Ticket #2)。
         • 若当初走的是第 2 候选 ──► 只 Drop 仍存活的 Slot 1 (Ticket #1)。
```

### 连绑定声明的逆序清理顺序都按实际命中的候选走

更夸张的细节还在后面：
如果在 `or` 模式里，我们把两个 `Ticket` **全都绑定了出来，但在左右两个候选里写的名字顺序刚好相反**，比如：

```flurry
fn inspect_both(value: Tuple<bool, Ticket, Ticket>) {
    value match {
        -- 第 1 候选先绑 first 再绑 second；
        -- 第 2 候选先绑 second 再绑 first。
        (true, first, second) or (false, second, first) => {
            println("first={}, second={}", first.id, second.id);
        },
    }
}
```

退出这个分支时，`first` 和 `second` 谁先被 `Drop`？
按照 Flurry 的“后声明先清理（逆序清理）”核心规则（详见 `docs/control-flow/pattern-matching/index.md`）：
- 如果命中第一候选 `(true, first, second)`：绑定建立次序是 `first` $\to$ `second`，因此退出时**先清理 `second`，再清理 `first`**。
- 如果命中第二候选 `(false, second, first)`：绑定建立次序是 `second` $\to$ `first`，因此退出时**先清理 `first`，再清理 `second`**。

编译器绝不会因为把变量名合并到了同一个分支体，就篡改你每个候选真实的绑定声明次序。而且如果两个绑定之间存在析构依赖（比如其中一个的 `Drop` 需要访问另一个），编译器会强制检查**所有可能命中的候选清理次序**都必须满足内存安全依赖。

---

## 本篇小结

| 模式组合场景 | 写法示例 | 所有权与清理语义 |
| :--- | :--- | :--- |
| **纯观察 + 整体移动** | `{ ready: true } as whole` | 内部只检查条件不拿走所有权，成功后将完整仿射对象移交给 `whole` |
| **`Copy` 字段 + 整体移动** | `{ ready } as whole` | 复制 `Copy` 字段给 `ready`，同时将完整仿射对象移交给 `whole`（不要求载荷 `T` 可 `Copy`） |
| **`or` 跨位置提取仿射值** | `(true, item, _) or (false, _, item)` | 守卫成功后提交选中字段的移动；剩余未移出的字段按实际命中的候选精准清理 |
| **`or` 逆序绑定汇合** | `(true, a, b) or (false, b, a)` | 保留实际成功候选的绑定取得次序，退出分支时严格按该候选的逆序执行 `Drop` |

---

### 下一篇预告

到现在为止，我们在处理传感器数据时，所有的过滤条件（如 `c >= 90`、`code < limit`）都是写死在 `match` 分支里的。
但是，如果调用方希望动态传入一个**自定义的过滤规则或变换动作**呢？比如：
```flurry
let offset: u32 = 10;
let calibration_fn = |reading: u32| reading + offset;
```
当我们写下 `|reading: u32| reading + offset` 这行 **Lambda 表达式** 的瞬间：
1. 外部的变量 `offset` 是什么时候进入闭包的？如果下一行我把外部的 `offset` 改成了 `20`，调用 `calibration_fn(1)` 算出来的是 `11` 还是 `21`？
2. 为什么 Rust 的闭包会根据函数体里怎么用变量而自动猜测“按共享引用捕获 vs 按可变引用捕获 vs 按值移动”，而 Flurry 却**坚持默认按值捕获，要求观察外部修改时必须显式捕获指针 `offset.ref`**？
3. 如果闭包里只用到了结构体里的一个字段 `pair.left`，编译器会把整个 `pair` 都抓进闭包里锁死，还是只精准搬走 `pair.left` 这一个最小独立位置？

在下一篇 **《Flurry 演进手记 (030)：当 Lambda 捕获外部世界——|x| expr、按值捕获与最小独立位置指针观察》** 中，我们将正式揭开 Flurry 闭包设计的第一层奥秘。
