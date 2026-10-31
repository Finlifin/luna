# Flurry 演进手记 (028)：在模式匹配中途算个值——and expr is pat 链式顺序匹配

在 [上一篇 (027)](./027-pattern-functions-and-case-parameters.md) 的结尾，我们遇到了一个几乎所有写过复杂 `match` 的程序员都踩过的坑：**“模式匹配中途需要调个函数算个值，然后再继续匹配这个计算结果。”**

来看一个具体的传感器固件场景：
主控台收到一个传感器注册请求 `DeviceNode { gateway_id: ?u32, sensor_code: u32 }`。
我们要判断这个设备能否接入当前告警通道，必须依次跨过三道关卡：
1. 先检查 `node` 里的 `gateway_id` 是否非空的 `gid?`；
2. 拿着刚拆出来的 `gid`，**调用查询函数 `find_gateway(gid)`**，检查它返回的 `?Gateway` 是否非空的 `gw?`；
3. 再拿着 `gw.region_id`，**调用查询函数 `find_region_limit(gw.region_id)`**，检查它返回的 `?u32` 是否非空的 `limit?`；
4. 最后检查 `node.sensor_code < limit`。如果以上任何一步为 `null` 或违背，都统统落入同一个“报错接入（Fallback）”分支。

如果只用传统的模式匹配，这三步“拆包 $\to$ 调函数计算 $\to$ 再拆包”怎么写？下面我们逐步分析。

---

## 一、 层层缩进的“厄运金字塔”（Pyramid of Doom）

在没有链式顺序匹配的语言里，因为 `find_gateway(gid)` 是一个函数调用表达式而非 `node` 内部现成的字段，你根本没法把它写进外层的模式里。你只能写出这样的多层嵌套：

```flurry
use std.prelude.*;

struct Gateway {
    region_id: u32,
}

struct DeviceNode {
    gateway_id: ?u32,
    sensor_code: u32,
}

fn find_gateway(gid: u32) -> ?Gateway {
    return null while gid == 0u32;
    Gateway { region_id: gid * 10u32 }
}

fn find_region_limit(region_id: u32) -> ?u32 {
    return null while region_id > 100u32;
    50u32
}

-- 每次需要调函数算个中间值，模式匹配就被迫中断，缩进加一层。
fn check_admission_initial(node: DeviceNode) -> bool {
    if node is { gateway_id: gid?, sensor_code: code } do {
        if find_gateway(gid) is gw? do {
            if find_region_limit(gw.region_id) is limit? do {
                if code < limit {
                    return true;
                }
            }
        }
    }
    -- 任何一层失败，都得小心翼翼地跨过四层花括号掉到这里
    false
}
```

而在多分支的 `match` 里，情况更糟。假设你在第一个 `match` 分支里先匹配了 `{ gateway_id: gid? }`，走进分支体后调了 `find_gateway(gid)` 发现返回了 `null`——**此时你已经进到第一个分支的 `{ ... }` 肚子里面了，根本没法反悔说“算了，我退回去试 `match` 的第二个分支吧”。**

我们需要一种机制，允许我们在**同一个模式候选（Pattern Candidate）内部**，先匹配左边的结构拿到绑定变量，紧接着用这个变量计算一个新表达式，再对新表达式的结果继续做模式匹配。

---

## 二、 `pat and expr is next_pat` 链式顺序匹配

在 Flurry 中，模式层提供了中缀组合算子：**`and expr is pattern`**（详见 `docs/control-flow/pattern-matching/index.md`）。

下面先来看看一条 `and ... is ...` 链条是怎么像接力赛一样工作的：

```text
  主语: node
   │
   ▼
  [第 1 棒: 模式] { gateway_id: gid?, sensor_code: code }
   ├── (若失败) ───────────────────────────────────────────┐
   ▼ (成功! 导出候选绑定 gid, code)                          │
  [第 2 棒: 表达式 is 模式] and find_gateway(gid) is gw?    │
   ├── (若返回 null 失配 gw?) ──► 撤销 gid, code! ────────┤
   ▼ (成功! 追加候选绑定 gw)                                 │
  [第 3 棒: 表达式 is 模式] and find_region_limit(...) is limit?
   ├── (若返回 null) ────────────► 撤销 gid, code, gw! ────┤
   ▼ (成功! 追加候选绑定 limit)                              │
  [第 4 棒: 模式守卫] if code < limit                       │
   ├── (若为 false) ─────────────► 撤销全部候选绑定! ────────┼──► 安全回退，尝试下一分支。
   ▼ (全部通关  )                                            │
  提交绑定 (gid, code, gw, limit)，执行分支体。              ▼
```

来看看用 `and ... is ...` 重写后的 `check_admission`，四层嵌套被整理为一条清晰的顺序匹配链：

```flurry
-- 用 `and expr is pat` 把“解构 -> 计算 -> 再解构 -> 守卫”串成一条线。
fn check_admission(node: DeviceNode) -> bool {
    if node is { gateway_id: gid?, sensor_code: code }
        and find_gateway(gid) is gw?
        and find_region_limit(gw.region_id) is limit?
        if code < limit
    do {
        true
    } else {
        false
    }
}
```

甚至在 `let` 声明中，`and ... is ...` 也能直接用来串联多个中间计算步骤。比如我们要对数据包里的 `ready` 标志位做连续两次翻转校验：

```flurry
use std.prelude.*;

struct Packet {
    ready: bool,
}

fn flip(value: bool) -> bool {
    not value
}

fn inspect_computed(packet: Packet) -> bool {
    -- 1. `item` 绑定原 packet
    -- 2. 调用 flip(item.ready) 并将结果匹配绑定到 `inverted`
    -- 3. 调用 flip(inverted) 并将结果匹配绑定到 `restored`
    let item
        and flip(item.ready) is inverted
        and flip(inverted) is restored = packet;

    restored and not inverted
}

test {
    println("{}", inspect_computed(Packet { ready: true })); -- true
}
```

> **🔍 语法与优先级小贴士：模式里的 `and ... is ...` vs 守卫里的 `and`**
> 仔细看 `check_admission`：
> 1. 在模式层，写的是 **`pat1 and expr is pat2`**。只有当左侧的 `pat1` 匹配成功后，才会求值中间的 `expr`，并把结果拿去匹配 `pat2`。
> 2. 而在 `if code < limit` 守卫（Guard）之后，紧跟着的表达式按普通布尔表达式解析（比如 `if code < limit and code > 0` 里的 `and` 是普通布尔短路与）。
> 3. **失败即干净回退**：如果链条中途的某个 `is pat2` 或最后的 `if cond` 失败了，当前候选在前面几棒建立的所有绑定都会被撤销，**下一个 `match` 分支拿到的依然是完好无损的原始主语**。

---

## 三、 `and expr is pat` 中的表达式约束与顺序绑定

在 `pat1 and expr is pat2`（以及模式守卫 `pat if expr`）中，中间的 `expr` 就是一个普通的表达式位置。因此，任何合法的表达式都可以直接写在这里——既可以是函数调用、`match` 表达式，也可以是普通的 `do { ... }` 块表达式（当你需要用几步局部语句计算中间值、又不想单独抽出顶层辅助函数时）：

```flurry
use std.prelude.*;

struct Packet {
    ready: bool,
}

fn accept(ready: bool) -> bool {
    ready
}

fn inspect_block(packet: Packet) -> bool {
    -- `do { ... }` 只是一个普通的块表达式，作为 `and expr is pat` 中的 `expr` 求值。
    let item and do {
        let original = item.ready;
        var result = not original;
        result = not result;
        accept(result)
    } is true = packet else {
        return false;
    }

    item.ready
}

test {
    println("{} {}",
        inspect_block(Packet { ready: true }),
        inspect_block(Packet { ready: false }));
    -- 输出: true false
}
```

由于 `and expr is pat` 和 `if expr` 发生在**模式候选尚未最终提交**的阶段（随时可能因为后续匹配失败而回退到下一个候选分支），此处的 `expr` 自然受到模式候选求值的约束：禁止于匹配中途修改外层变量、消费正在被匹配的候选主语所有者，或直接向外层函数 `return`：

```flurry
-- ❌ 编译期直接报错：在 `and expr is pat` 的 `expr` 中禁止修改外部变量。
fn bad_candidate_mutation(packet: Packet) -> bool {
    var external_counter = 0u32;
    let item and do {
        external_counter += 1u32; -- ❌ 错误：禁止写入外层变量。
        item.ready
    } is true = packet else {
        return false;
    }
    true
}
```

### 元组模式内部的从左到右顺序绑定

非但 `and expr is pat` 可以引用左侧绑定，在 Flurry 的**元组模式 `(first, second)`** 中，绑定也是按字段从左到右依次建立的。这意味着**后面字段上的守卫（Guard）可以直接读取前面字段刚刚绑定的变量**：

```flurry
use std.prelude.*;

-- 检查二元组里的两个布尔值是否相等：
-- 第 2 个元素上的守卫 `if first == second` 直接读取了第 1 个元素绑定的 `first`。
fn same_pair(value: Tuple<bool, bool>) -> bool {
    value match {
        (first, (second if first == second)) => true,
        _ => false,
    }
}

test {
    println("{} {}", same_pair((true, true)), same_pair((true, false)));
    -- 输出: true false
}
```

```flurry
-- ❌ 编译期直接报错：前向引用未能成立（禁止于第 1 个字段里引用还没绑定的第 2 个字段），
-- 且同一个完整模式内亦无法重复声明同名绑定。
fn bad_tuple_pattern(value: Tuple<bool, bool>) -> bool {
    value match {
        ((first if first == second), second) => true, -- ❌ 此时 second 尚未建立。
        _ => false,
    }
}
```

---

## 本篇小结

| 特性工具 | 写法形态 | 核心作用与安全边界 |
| :--- | :--- | :--- |
| **链式顺序匹配** | `pat1 and expr is pat2` | 在不打断外层模式的前提下，用 `pat1` 的绑定计算任意表达式 `expr`（包括函数调用或普通 `do { ... }` 块表达式）并继续解构；失败自动撤销回退，且 `expr` 严禁修改外部变量或消费候选主语 |
| **元组顺序绑定** | `(first, (second if first == second))` | 按从左到右顺序建立候选绑定，后序字段守卫可直接读取前序字段绑定 |

---

### 下一篇预告

在这篇的例子里，我们在模式里算来算去的值（如 `bool`、`u32`）全都是可以随意复制的 **`Copy` 标量**。
但是，如果我们的传感器数据包里装着一个**唯一无二的独占硬件票据 `Ticket`（仿射类型 Affine Type / Once 值）**呢？

思考两个务实的问题：
1. 如果我想在 `match` 里**既检查数据包里的某个字段（比如 `{ ready: true }`），又在匹配成功后把整个唯一无二的 `Packet<Ticket>` 完整地搬走（Move）交给下游函数**，我该怎么写？如果我写了 `{ ready, payload } as whole`，是否会因为同时绑定了内部字段和整个对象而把编译器惹毛？
2. 如果我有一个 `or` 模式：`.pair(item, _) or .tagged(_, _, item)`，左边变体有 2 个 `Ticket`，右边变体有 3 个 `Ticket`，我只移走了其中一个绑定到 `item`，**剩下没被移走的那个 `Ticket` 该怎么自动清理（`Drop`）？**

在下一篇 **《Flurry 演进手记 (029)：既要检查字段又要搬走整个包——仿射类型的 as whole 与 or 模式所有权汇合》** 中，我们将深入模式匹配与所有权系统的交汇心脏。
