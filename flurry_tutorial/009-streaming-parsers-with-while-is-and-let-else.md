# Flurry 演进手记 (009)：把流式解析写得像呼吸一样自然——while ... is ... do 与 let ... else

在 [上一篇 (008)](./008-escaping-nested-loops-with-labels-and-never.md) 中，我们学会了用 `while:label` 和 `for:label` 在嵌套循环中自由穿梭。今天，我们要把前面几篇打造的零件组装起来，编写传感器固件中最繁忙的心脏模块：**事件流消费与实时聚合器（Stream Consumer）**。

来看看这个模块的数据流长什么样：
1. 底层中断服务程序把采集到的原始事件源源不断地塞进一个环形队列 `EventQueue`；
2. 每次调用 `queue.pop()`，都会返回一个 **`?RawEvent`**：如果队列里还有事件，就返回 `RawEvent`；如果队列已经排空，就返回 `null`，此时消费循环应当结束。
3. 每个 `RawEvent` 有两种可能：
   - `.tick`：代表定时器滴答，直接忽略并继续拉取下一个事件；
   - `.frame { channel, raw_byte }`：代表收到了一帧传感器字节。
4. 收到 `.frame` 后，我们还要依次过两道关：
   - 第一关：要求 `channel` 必须等于当前监控的目标通道，否则丢弃；
   - 第二关：调用 `decode_data(raw_byte)` 提取有效读数（返回 `?u8`），如果是空心跳包（`null`）就丢弃，只有拿到真正的读数 `val` 才累加到总和里。

如果我们只用最基础的 `while true` 和多层嵌套 `match` 来写这段流式解析，代码会长成什么样？先看基础写法。

---

## 一、 令人窒息的“右移缩进金字塔”

来看看每个系统程序员都曾饱多层嵌套的匹配写法：

```flurry
use std.prelude.*;

enum RawEvent {
    tick,
    frame { channel: u8, raw_byte: u8 },
}

struct EventQueue {
    -- 模拟从队列弹出下一个可选事件
    fn pop(*self) -> ?RawEvent {
        null
    }
}

fn decode_data(raw: u8) -> ?u8 {
    raw'bitvec match {
        0xA (payload: BitVec<4>) => payload.as(u8),
        _ => null,
    }
}

-- 四层缩进的“右移金字塔”。
fn drain_and_sum_initial(queue: *EventQueue, target_ch: u8) -> u32 {
    var total = 0u32;

    while true {
        queue.pop() match {
            null => break,
            ev? => {
                ev match {
                    .tick => continue,
                    .frame { channel: ch, raw_byte: raw } => {
                        if ch == target_ch {
                            decode_data(raw) match {
                                null => continue,
                                val? => {
                                    -- 真正干活的代码被挤到了第 9 层缩进（36 个空格之外）。
                                    total += val.as(u32);
                                },
                            }
                        }
                    },
                }
            },
        }
    }

    total
}
```

如下所示，直观感受一下 传统的结构有多么扭曲：

```text
  while true {
      queue.pop() match {
          ev? => {
              ev match {
                  .frame { ... } => {
                      if ch == target_ch {
                          decode_data(raw) match {
                              val? => {
                                  total += val.as(u32);  ◄── 核心逻辑被埋在深谷底。
                              }
                          }
                      }
                  }
              }
          }
      }
  }
```

为什么这段代码读起来这么累？
因为在这条流水线上，**主干路径（Happy Path）只有一条**，但每一步都会分叉出一个“没数据就退出”或“不匹配就跳过”的旁支。使用嵌套 `match` 强迫我们每过一道关卡就把主干代码往右推一层缩进。

怎么把这个金字塔彻底拍平？
我们需要两把手术刀：
1. 第一把刀专门对付循环入口的 `queue.pop()`：**“只要还能匹配出值，就继续循环；一旦匹配失败，自动结束循环”**——这就是 **`while ... is ... do`**。
2. 第二把刀专门对付循环体内部的层层校验：**“尝试把值拆开绑定到当前作用域；如果拆不开，就在旁边的 `else` 块里直接 `continue` 或 `return` 走人”**——这就是 **`let ... else`**。

---

## 二、 用 `while ... is ... do` 优雅驱动流式循环

先拿起第一把手术刀。
在第一个示例中，我们写了 `while true { queue.pop() match { null => break, ev? => ... } }`。
在 Flurry 中，`while` 原生支持直接结合模式匹配，并提供**单臂**和**多臂**两种形态（详见 `docs/control-flow/basic.md` 与 `docs/control-flow/pattern-matching/index.md`）：

### 形态 1：单模式 `while expr is pattern do { ... }`

```text
  while   queue.pop()   is   ev? do { ... }
          ───────────        ───   ──
               │              │    └── 关键字 `do` 必不可少。（防止模式与 `{` 块粘连歧义）
               │              └─────── 对偶模式 `ev?`：匹配 `?RawEvent` 的有值分支并绑定 `ev`
               └────────────────────── 每轮循环开头求值一次
```

它的执行契约极其清爽：
- 每一轮循环开始时，先求值 `queue.pop()`，并尝试用模式 `ev?` 去匹配它；
- **如果匹配成功**：将拆出来的载荷绑定到变量 `ev`（作用域覆盖整个 `do { ... }` 循环体），然后执行循环体；
- **如果匹配失败**（即 `queue.pop()` 返回了 `null`）：**直接正常结束整个 `while` 循环。**

> **🔍 语法小贴士：千万别漏掉模式后面的 `do`。**
> 无论是单模式的 `if expr is pat do { ... }`，还是 `while expr is pat do { ... }`，模式 `pat` 和左花括号 `{` 之间**必须写关键字 `do`**。
> 为什么？还记得我们在第 004 篇讲过的后缀扩展应用吗？如果允许不写 `do` 直接写 `while x is Foo do { a: 1 } { ... }`，编译器和读者都会晕头转向：后面紧跟的 `{ ... }` 到底是模式的一部分，还是循环体的开始？一个短短的 `do`，就像一道坚固的防火墙，把“模式匹配区”和“执行代码块”彻底隔开。

```flurry
-- ❌ 编译期直接拒绝。
fn missing_do_keyword(queue: *EventQueue) {
    -- 错误：`is ev?` 后面漏写了 `do` 关键字。
    while queue.pop() is ev? do {
        println("got event");
    }
}
```

### 形态 2：多臂 `while expr is do { case_arm* }`

不仅如此。如果在某些状态机循环里，你想在每一轮直接根据不同的模式走不同的分支（只要匹配上列出的任意一个分支就继续循环，或者在某个分支里 `break`），你甚至可以把 `pattern` 留空，写成**多臂匹配循环 `while expr is do { ... }`**（同样支持挂标签 `while:loop_name expr is ... do`。）：

```flurry
-- 多臂形式：直接在 while 头部完成分发。
fn count_frames(queue: *EventQueue) -> u32 {
    var frame_count = 0u32;

    -- 注意：当 `queue.pop()` 为 `ev?` 时执行对应分支并继续下一轮；
    -- 当为 `null` 时（未匹配或显式 break），离开循环。
    while queue.pop() is ev? do {
        ev match {
            .tick => (),
            .frame { channel: _, raw_byte: _ } => {
                frame_count += 1u32;
            },
        }
    }

    frame_count
}
```

---

## 三、 用 `let ... else` 把主干路径彻底拍平

`while queue.pop() is ev? do` 帮我们消掉了最外层的 `while true` 和第一次 `match`。
但在循环体内部，我们还要解构 `.frame { channel: ch, raw_byte: raw }`，还要解构 `decode_data(raw)` 的 `val?`。

这时候，第二把手术刀——**`let pattern = expr else { ... }`**（以及可变绑定版 **`var pattern = expr else { ... }`**）闪亮登场。（详见 `docs/control-flow/pattern-matching/index.md`）。

先记住一条核心公理：
> **在 Flurry 中，`let` 和 `var` 的左边从来都不只是一个简单的变量名，它永远是一个完整的模式（Pattern）。**

这意味着，当模式是**不可反驳的（Irrefutable，即 100% 必定匹配成功）**时，比如拆开一个二元组或结构体，你根本不需要写 `else`，直接一行拆开：

```flurry
let (row, col) = (2i32, 3i32);
```

而当模式是**可反驳的（Refutable，即有可能匹配失败）**时——比如匹配枚举的某一个变体 `.frame { ... }`，或者匹配可选类型的有值分支 `val?`，或者匹配错误限定类型的成功分支 `ok!`——你只需要在语句末尾加上 **`else { ... }`**：

下面来看清 `let ... else` 是如何改变控制流拓扑结构的：

```text
  let .frame { channel: ch, raw_byte: raw } = ev else {
      continue;  ◄── [失败逃生舱]：如果 ev 是 .tick，进入 else 块并立即跳走（发散）。
  }
  │
  ▼ [成功主干道]：一旦跨过上面那行，编译器 100% 确信匹配已经成功。
    变量 `ch` 和 `raw` 直接暴露在当前外层作用域中，零额外缩进。
```

### 为什么编译器强制要求 `else` 块必须“发散（Diverge）”？

试想一下：如果 `ev` 实际上是 `.tick`（匹配 `.frame` 失败），程序走进了 `else { ... }` 块；要是这个 `else` 块里只打印了一句日志，然后**正常走出了 `else` 块**，来到下一行代码 `if ch != target_ch`——请问这时候变量 `ch` 和 `raw` 里装的是什么？它们根本就没有被绑定成功。

因此，Flurry 编译器对 `let ... else` 和 `var ... else` 实施了一条铁面无私的静态检查：
**`else` 块的类型必须是 `never`（即必须通过 `return`、`break`、`continue` 等控制流转移离开当前作用域，绝对不允许正常走完并落入后方代码）。**

```flurry
-- ❌ 编译期直接拒绝。
fn non_diverging_else(maybe_val: ?u8) -> u8 {
    let val? = maybe_val else {
        println("oops, it was null!");
        -- 错误：else 块正常结束了，没有 return / break / continue。
        -- 如果允许继续往下走，下一行的 `val` 根本不存在。
    }
    val
}
```

现在，让我们把 **`while ... is ... do`**、**`let ... else`** 以及我们在第 007 篇学到的 **`continue while cond;`** 组合在一起，看看原本缩进了 9 层的 `drain_and_sum_initial` 被改造成 后变成了什么样：

```flurry
use std.prelude.*;

enum RawEvent {
    tick,
    frame { channel: u8, raw_byte: u8 },
}

struct EventQueue {
    fn pop(*self) -> ?RawEvent {
        null
    }
}

fn decode_data(raw: u8) -> ?u8 {
    raw'bitvec match {
        0xA (payload: BitVec<4>) => payload.as(u8),
        _ => null,
    }
}

-- 零嵌套金字塔。所有步骤像呼吸一样在第一层缩进平铺展开。
fn drain_and_sum(queue: *EventQueue, target_ch: u8) -> u32 {
    var total = 0u32;

    -- 第 1 步：只要队列还能弹出事件 ev，就继续循环；为 null 则自动结束循环
    while queue.pop() is ev? do {
        -- 第 2 步：解构 .frame 变体；如果是 .tick，在 else 中直接 continue 下一轮
        let .frame { channel: ch, raw_byte: raw } = ev else {
            continue;
        }

        -- 第 3 步：用后置 while 守卫过滤非目标通道
        continue while ch != target_ch;

        -- 第 4 步：解码并提取有效读数 val；如果损坏或为空包，直接 continue
        let val? = decode_data(raw) else {
            continue;
        }

        -- 第 5 步：到达这里的 100% 是目标通道的有效读数，依然保持在第一层循环缩进。
        total += val.as(u32);
    }

    total
}

test {
    let q = EventQueue {}
    assumes drain_and_sum(q.ref, 1u8) == 0u32;
}
```

请把 `drain_and_sum` 和开头的 `drain_and_sum_initial` 放在一起对比一下：
- 在第一个示例中，代码缩进了整整 5 层大括号，末尾连着 5 个 `}`，读到里面早忘了外面的条件是什么；
- 在第二部分中，整个函数体从头到尾**只有 `while` 循环这一层缩进**。每一步检验就像在流水线上安检：合格的数据拆出 `ch`、`raw`、`val` 大步流星地往下走，不合格的数据在旁边的 `else { continue; }` 或 `continue while ...;` 处瞬间分流。

---

## 本篇小结

从“右移缩进金字塔”到平铺直叙的流式管线，我们在这一篇解锁了模式匹配与控制流结合的两项关键特性：

| 构造 | 语法形态 | 适用场景 | 关键语法与语义规则 |
| :--- | :--- | :--- | :--- |
| **条件匹配循环** | `while expr is pat do { ... }` | 持续拉取流/迭代器直到不匹配 | **`do` 关键字不可省略**；支持挂循环标签 `while:lbl`；也支持多臂 `while expr is do { ... }` |
| **单模式分支** | `if expr is pat do { ... }` | 只关心单一模式分支（可接 `else`） | **`do` 关键字不可省略**；不强制穷尽；同样支持多臂 `if expr is do { ... }` |
| **不可反驳解构** | `let (x, y) = pair;` | 拆开元组或结构体字段 | 模式 100% 匹配，无需写 `else` |
| **可反驳解构绑定** | `let pat = expr else { ... }` | 校验枚举变体、`?T`、`!Err T` 并提取变量 | 匹配成功则绑定进入外层后续作用域；**`else` 块必须发散（类型为 `never`）**；支持 `var pat = expr else` |

### 下一篇预告

到现在为止，我们已经用过了好几种基础模式：`ev?`、`val!`、`.frame { ... }`、`(r, c)`。
但是，当传感器的协议变得更加复杂时，我们很快就会想把多种模式**组合**在一起：
- 如果我们想在一条分支里同时匹配“可选读数存在，且读数是 `1` **或者** `2`”，该写成 `1 or 2?` 还是 **`(1 or 2)?`**？它们俩的意思一样吗？
- 如果我们想在匹配范围 `10..20` 或者 `30` 的同时，用 `as num` 把匹配到的整体数值绑定下来，为什么不能裸写 `10..20 as num`，而必须加上括号写成 **`(10..20) as num`**？
- 还有最容易让人踩坑的括号：在模式和表达式里，**`(p)`（分组括号）** 和 **`(p,)`（单元素元组）** 到底有什么区别？

在下一篇 **《Flurry 演进手记 (010)：给数据包做外科手术——模式优先级、括号分组 `(p)` vs `(p,)` 与范围模式》** 中，我们将拿起放大镜，彻底摸清 Flurry 模式树的组合优先级与区间法则。
