# Flurry 演进手记 (161)：用效应实现可回滚的解析器回溯（Parser Backtracking）

> **阅读指南**：在前面的章节里，我们已经掌握了代数效应（Algebraic Effects）的直接处理、单次限定 Continuation（`catch continuation: builtin.Continuation`）、异步效应与事务内存。从本篇开始，我们将进入第七卷的压轴进阶实战专题。今天先来看第一个经典工程难题：**解析器回溯（Parser Backtracking）**。当我们解析复杂的变长传感器协议或命令语法时，经常需要先试探一条分支，如果消费了几个字节后发现此路难通，就必须把游标（Cursor）精准退回到试探前的检查点（Checkpoint），再去尝试另一条分支。结合 `docs/algebraic-effect-and-delimited-continuation/algebraic-effect.md` 与 `multi-prompt-delimited-continuation.md`，让我们看看如何用代数效应写出十分干净、且 100% 遵守单次 Continuation 与资源安全红线的可回滚解析器。

---

## 一、 问题背景：手写游标回滚的“漏退坑”与多次（Multi-Shot）克隆的资源安全问题

假设我们在解析一种智能工业传感器的双格式命令包：
- **格式 A（快速校准包）**：以字节 `0xA0` 开头，紧跟字节 `0x01`，最后是一个数据字节；
- **格式 B（标准读取包）**：同样以字节 `0xA0` 开头，但紧跟的是字节 `0x02`，最后是一个数据字节。

注意看。这两种格式拥有**共同的前缀 `0xA0`**。如果收到的数据包是 `[0xA0, 0x02, 42]`，当我们先尝试按“格式 A”解析时，第 1 个字节 `0xA0` 匹配成功（游标从 `0` 前进到了 `1`），但第 2 个字节读到 `0x02 != 0x01`，格式 A 匹配失败。这时候，在尝试“格式 B”之前，游标必须从 `1` 退回到 `0`。

### 到处传 `*usize` 游标指针，忘了回滚就全盘错位

在传统命令式代码里，程序员只能把游标指针 `cursor: *usize` 塞进每一个解析子函数里，并在每一处失败分支手动恢复旧游标：

```flurry
-- 手动保存和恢复游标，容易在复杂的早期返回中漏掉回滚。
fn parse_format_a(buf: Array<u8, 3>, cursor: *usize) -> ?u8 {
    let saved = cursor.*;
    if buf[cursor.*] != 0xA0u8 { return null; }
    cursor.* += 1usize;

    if buf[cursor.*] != 0x01u8 {
        -- ⚠️ 一旦在这里忘了写 `cursor.* = saved;`，
        -- 游标就会停在半路 1usize，导致后续的 parse_format_b 读错字节全盘崩溃。
        cursor.* = saved;
        return null;
    }
    cursor.* += 1usize;
    let payload = buf[cursor.*];
    cursor.* += 1usize;
    payload
}
```

而在一些学院派函数式语言里，有人会说：“我在 `choose()` 分支点直接把 Continuation 克隆（Clone）两份，先跑第一份，失败了再跑第二份不就行了吗？”
**这在系统级语言中实属违规。** 正如 `multi-prompt-delimited-continuation.md` 明确规定的：
- **`builtin.Continuation` 不可 `Copy`，也未提供 `Clone`。**
- 为什么？因为如果允许把挂起栈克隆成两份并恢复两次，栈上持有的仿射资源（如文件句柄、唯一内存所有权、`Drop` 责任）就会被凭空复制并双重释放（Double Free）。

**核心诉求**：
我们既想让业务解析函数告别到处手写 `let saved = cursor.*;` 的样板与漏退风险，又要严格遵守 **单次恢复（One-Shot Continuation）与显式游标事务回滚** 的系统级安全契约。

---

## 二、 核心机制：用 `effect enum Stream` 封装游标，让 Handler 接管检查点回滚

在 Flurry 中，解析函数全然无需知道底层游标 `cursor` 存在哪儿。我们可以定义一个代数效应组 **`effect enum Stream`**，把“读下一个字节”、“打检查点”、“回滚到检查点”声明为效应操作，或者提供一个带自动回滚能力的组合器 `try_branch`。

下面来看清业务解析器与底层游标 Handler 是如何分工协作的：

```text
  ┌─ 业务解析层 (只关心语法逻辑，完全看不到游标指针!) ──────────────────────┐
  │ fn parse_format_a() -> #Stream ?u8 {                                    │
  │     if Stream.next_byte()# != 0xA0u8 { return null; }                   │
  │     if Stream.next_byte()# != 0x01u8 { return null; }                   │
  │     Stream.next_byte()#                                                 │
  │ }                                                                       │
  └───────────────────────────────────┬─────────────────────────────────────┘
                                      │ 发出效应请求 Stream.next_byte()#
                                      │ 组合器自动执行 mark() 与 backtrack(saved)
                                      ▼
  ┌─ 效应 Handler 状态层 (集中维护唯一游标 var cursor = 0usize) ────────────┐
  │   buffer: [ 0xA0,  0x02,  42 ]                                          │
  │             ▲      ▲                                                    │
  │   1. 尝试格式 A 前：Stream.mark()# 记录检查点 saved = 0                 │
  │   2. 消费 0xA0 (cursor=1)，接着读到 0x02 != 0x01，格式 A 返回 null      │
  │   3. 组合器检测到 null，自动发出 Stream.rewind(0)# 将 cursor 退回 0      │
  │   4. 尝试格式 B：从 cursor=0 重新读取 0xA0 -> 0x02 -> 42，成功交付 42   │
  └─────────────────────────────────────────────────────────────────────────┘
```

### 基于 `#Stream` 效应组与自动回滚组合器的干净解析器

让我们把这套设计写成真实的 Flurry 代码。你看，在 `parse_format_a` 和 `parse_format_b` 里面，连一行操作 `cursor` 索引的代码都没有。

```flurry
use std.prelude.*;

-- 定义字节流与回溯控制的代数效应组
effect enum Stream {
    next_byte() -> ?u8,
    mark() -> usize,
    rewind(pos: usize),
}

-- 通用回溯试探组合器：
-- 自动在执行 parser 前打下检查点，若解析返回 null 则自动将流回滚到原检查点。
fn try_parse(parser: fn() -> #Stream ?T) -> #Stream ?T
where
    T: Type,
{
    let checkpoint = Stream.mark()#;
    let outcome = parser()#;
    if outcome is null do {
        Stream.rewind(checkpoint)#;
    }
    outcome
}

-- 格式 A：0xA0 -> 0x01 -> payload（只管往前读，失败直接 return null，无需操心回滚。）
fn parse_format_a() -> #Stream ?u8 {
    let b0? = Stream.next_byte()# else { return null; }
    if b0 != 0xA0u8 { return null; }

    let b1? = Stream.next_byte()# else { return null; }
    if b1 != 0x01u8 { return null; }

    Stream.next_byte()#
}

-- 格式 B：0xA0 -> 0x02 -> payload
fn parse_format_b() -> #Stream ?u8 {
    let b0? = Stream.next_byte()# else { return null; }
    if b0 != 0xA0u8 { return null; }

    let b1? = Stream.next_byte()# else { return null; }
    if b1 != 0x02u8 { return null; }

    Stream.next_byte()#
}

-- 顶层协议解析：先试探格式 A，若失配则自动回滚游标并尝试格式 B。
fn parse_packet() -> #Stream ?u8 {
    if try_parse(parse_format_a)# is val? do {
        return val;
    }
    try_parse(parse_format_b)#
}

test {
    let input: Array<u8, 3> = [0xA0u8, 0x02u8, 42u8];
    var cursor: usize = 0usize;

    let parsed = parse_packet()# {
        Stream.next_byte() => {
            if cursor < 3usize {
                let b = input[cursor];
                cursor += 1usize;
                b
            } else {
                null
            }
        },
        Stream.mark() => cursor,
        Stream.rewind(pos) => {
            cursor = pos;
        },
    }

    println("parsed payload: {}, final cursor: {}", parsed is u8, cursor);
}
```

---

## 三、 结合限定 Continuation 时的“快速中止”与单次消费红线

除了让子解析器返回 `?u8` 之外，有时我们希望在深层解析树里一旦遇到失配的字节，连层层返回 `null` 都省去书写，直接调用一个捕获 Continuation 的效应 `fail_branch(catch k: builtin.Continuation)` **瞬间跳出整个当前候选分支**。

在使用这种捕获型效应（Capturing Effect）做快速失败回溯时，我们必须牢记 `docs/algebraic-effect-and-delimited-continuation/multi-prompt-delimited-continuation.md` 中的两条核心规则：

1. **放弃未恢复的 Continuation 会自动触发隐藏中止清理（Abandonment Cleanup）**：
   当 `fail_branch(continuation)` 分支决定放弃当前试探路径（即跳过执行 `resume`）时，离开 Handler 作用域会自动沿着隐藏中止通道重新进入被挂起的计算栈，把该分支中途创建的局部 `Drop` 对象（比如临时解析缓冲区 `ScratchBuffer`）全部安全析构，一直清理到当前限定边界为止。
2. **严禁对同一个 `builtin.Continuation` 恢复两次（No Multi-Shot）**：
   每次恢复都会消费掉唯一的恢复权；企图把同一个 `continuation` 保存或恢复两次，会在编译期的仿射移动检查（Move Check）中被静态拦截。

### 用捕获型效应实现深层零样板回溯，并拦截二次恢复

```flurry
use std.prelude.*;

-- 声明一个允许捕获剩余计算以直接中止当前试探分支的效应
effect bail_out(catch continuation: builtin.Continuation) -> never;

struct BranchGuard { id: u32 }
impl Drop for BranchGuard {
    fn drop(*self) {
        println("cleaned up branch {}", self.id.*);
    }
}

fn try_candidate_one(first_byte: u8) -> #[bail_out] u8 {
    let _guard = BranchGuard { id: 1u32 }
    if first_byte != 0x01u8 {
        -- 一键跳出当前候选分支。
        -- 当 Handler 放弃 continuation 时，_guard 的 Drop 会被自动执行。
        bail_out()#
    }
    42u8
}

test {
    let result = (do {
        let v = try_candidate_one(0x02u8);
        v
    }# {
        bail_out(continuation) => {
            -- 跳过执行 resume，直接返回兜底值 99u8。
            -- 编译器自动对挂起的 continuation 执行放弃清理协议，打印 "cleaned up branch 1"
            99u8
        },
    }).use(pool);

    println("fallback result: {}", result);
}
```

而如果有人企图在 Handler 里把捕获到的 `continuation` 当作多次（Multi-Shot）协程连续恢复两次，编译器会立刻亮起红灯：

```flurry
unsafe fn resume_once(k: builtin.Continuation);

effect checkpoint(catch continuation: builtin.Continuation);

fn broken_multishot_backtrack() {
    (do {
        checkpoint()#;
    }# {
        checkpoint(continuation) => unsafe {
            -- 第一次消费 continuation（合法）
            resume_once(continuation);

            -- ❌ 编译期直接报错：(Use of Moved Value: `continuation`)
            -- builtin.Continuation 是严格的仿射（Affine）单次恢复句柄，不可 Copy。
            -- 第一次 resume_once 已经消费了恢复权，禁止第二次恢复同一挂起栈。
            resume_once(continuation);
        },
    }).use(pool);
}
```

---

## 本篇小结

| 回溯模式 / 机制 | 实现方式 | 核心优势与安全保障 |
| :--- | :--- | :--- |
| **`#Stream` 效应游标封装** | `next_byte()`、`mark()`、`rewind(pos)` | 业务解析函数零游标指针样板，由 `try_parse` 统一保证失败必回滚 |
| **`bail_out(catch k)` 快速跳出** | 遇到失配直接挂起至候选边界并放弃 `k` | 无需层层透传 `null`，同时由隐藏中止通道自动析构分支内的局部 `Drop` 资源 |
| **单次恢复规则（One-Shot）** | `builtin.Continuation` 不实现 `Copy` / `Clone` | 避免多次（Multi-Shot）栈克隆导致的仿射资源双重释放（Double Free） |

---

### 下一篇预告

在这一篇里，我们的 `#Stream` 效应每调用一次 `Stream.next_byte()#`，都会进入一次 Handler 分支去推进游标 `cursor += 1`。

但是，在大型分布式系统或后端服务里，还有另一类迥然各异的需求：**结构化日志与分布式追踪（Structured Logging & Tracing）**。
想象一下：当一个 HTTP 请求或 RPC 调用进入系统时，我们在入口处分配了一个 `request_id: 9527` 和当前的调用链 `span_name: "db_query"`。在接下来的十层函数调用里，可能会打印上百条日志，每一条日志都需要读取这个 `request_id` 和 `span_name`。
- 如果用普通 `effect request_id() -> u32`，每次读 `request_id` 都要调一次 Handler 分支函数，太浪费了。
- 如果用全局静态变量，多线程并发时就会互相踩踏；如果把 `ctx: *TraceContext` 手动加进 100 个函数的参数列表里，代码又会被上下文参数淹没。

怎么才能做到**“在安装 Handler 的那一刻只求值一次，随后整条调用链像隐式寄存器传参一样零分支开销直接取值，而且还能在内层子任务里随时局部覆盖（Override）新的 `span_name`”**？

下一篇 **《Flurry 演进手记 (162)：用 Direct Passing 实现零全局变量的结构化日志与分布式追踪（Tracing）》**，我们将深入挖掘 `^attrs.effects.direct_passing` 的工程威力。
