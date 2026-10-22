# Flurry 演进手记 (033)：比宏更干净、比普通函数更透明——inline fn 与编译期确定性展开

在 [上一篇 (032)](./032-higher-order-functions-and-callback-contracts.md) 的结尾，我们提到了在高频传感器采样循环或模式匹配守卫（Guard）里调用小辅助函数时的一个核心诉求：**我们希望把被调用的函数体展开到调用位置，消除调用跳转开销，同时让调用处的控制流和状态分析直接看穿函数体。**

一提到“把代码展开到调用处”，系统程序员脑海里往往会浮现出两种典型取舍：
- **方案 A（C 语言宏 `#define`）**：虽然保证展开，但它是毫无类型与作用域观念的文本替换——实参 `produce()` 在宏体里出现两次就会被求值两次，宏体里的局部变量名会和调用者的变量名撞车，宏体里写个 `return` 会直接把调用者整个函数给退掉。
- **方案 B（C/C++/Rust 的 `inline`）**：它有正常的函数语义，但它通常只被当成扔给后端 LLVM 的一个“黑盒优化提示”。如果在前端/中端分析阶段（比如模式匹配守卫检查、或代数效应的局部消去）维持未展开态它，许多依赖看穿函数体的高级静态分析就无法进行。

在 Flurry 中，代码展开被清晰地划分为两套相辅相成的语言级机制：**`inline fn`** 与 **`quote fn`**（详见 `docs/functions/inline-and-quote.md`）。今天我们先聚焦 **`inline fn`**，看看它是如何在编译器中端（NIR）完成卫生、确定、且 100% 保持资源与返回边界的展开的。

---

## 一、 先立规矩——`inline fn`、`comptime fn` 与 `quote fn` 的三分天下

在动手写代码之前，很多初学者容易把 `inline fn`、`comptime fn` 和 `quote fn` 搞混。
下面通过对比图和表格，梳理这三者的职责边界（详见 `docs/functions/inline-and-quote.md` §三种不同工作）：

```text
  调用点: f(x)  （其中 x 是运行时的传感器读数变量。）
   │
   ├──► [1. comptime fn] ──► ❌ 报错。comptime fn 必须在编译时算出最终结果值，
   │                         不能接收普通的 runtime 变量并在运行时补算。
   │
   ├──► [2. inline fn]   ──► ✅ 按普通函数接口调用，但携带语言级内联意向 (inline true)。
   │                         编译器展开 pass 会将主体卫生接入调用处（若遇递归/超预算可安全保留普通调用）。
   │
   └──► [3. quote fn]    ──► ✅ 强制代码接入。必须在编译期把代码结构拼进调用处，
                             但拼进去的代码在运行时照常对 runtime 变量 x 执行计算。
```

| 函数修饰词 | 必须完成的工作 | 运行时主体与退路 |
| :--- | :--- | :--- |
| **`inline fn`** | 按函数接口调用，向编译器提供内联展开意向 | 展开后完全保持原函数语义；遇到递归或预算耗尽时可安全保留普通调用 |
| **`comptime fn`** | **必须在编译期计算出最终值** | 无法留下需要在 runtime 补算的调用 |
| **`quote fn`** | **参数化代码接入必须在编译期 100% 完成** | 接入后的代码结构在 runtime 执行；若无法完成接入则直接编译报错 |

今天我们先来看最贴近日常系统编程的 **`inline fn`**。

---

## 二、 `inline fn` 的卫生展开——提前 `return` 不误伤调用者

假设我们在写一个传感器数据包筛选函数 `select(input: bool) -> bool`。
在模式匹配的守卫 `if accepts(value)` 里，我们调用了一个辅助函数 `accepts`。注意：`accepts` 的函数体里写了一条**提前返回语句 `if value { return true; }`**。

如果是 C 语言的文本宏，在 `match` 的分支守卫里展开一条 `return true;`，会直接把外层的 `select` 函数给提前返回掉，根本不会去走 `match` 分支箭头 `=>` 后面的代码。

而在 Flurry 中，当我们用 `inline fn` 声明 `accepts` 并将其展开到模式守卫中时，会发生什么？来看看这段来自规范的标准代码：

```flurry
use std.prelude.*;

-- 使用 inline fn 声明带提前返回的判断函数
inline fn accepts(value: bool) -> bool {
    if value {
        return true;
    }
    false
}

-- 也可以在 inline fn 内部使用带守卫的 let ... else 提前返回。
inline fn accepts(value: bool) -> bool {
    let ready if ready = value else {
        false
    }
    ready
}

fn select(input: bool) -> bool {
    input match {
        -- 在模式守卫 (guard) 中调用 inline fn。
        value if accepts(value) => true,
        _ => false,
    }
}

test {
    println("{} {}", select(true), select(false));
    -- 输出: true false
}
```

下面来看看编译器在展开 `accepts(value)` 时，是怎么处理那个 `return true;` 和变量名的：

```text
  调用点 (在 select 的 match guard 内):
      value if accepts(value) => true

  编译器 NIR 卫生展开后 (概念示意):
      value if do {
          -- 1. 卫生重命名：生成具有全新身份 (Fresh Identity) 的形参绑定與私有返回槽。
          --    不会覆盖或污染调用者外层同名的变量。
          let param_value#1 = value;
          let return_slot#1: bool = ...;

          -- 2. 边界锁定：accepts 内部的 `return true;` 只结束这次展开的 accepts 调用，
          --    将结果交付给 return_slot#1，免将外层的 select() 函数直接 return 掉。
          ...
          return_slot#1
      } => true
```

看到精髓了吗？
1. **词法 `return` 的边界隔离**：当 `inline fn` 主体没有提前 `return` 时，尾表达式直接交付结果；当主体含有跨语句的提前 `return`（如 `if value { return true; }` 或 `let ... else { return false; }`）时，编译器会自动生成私有的返回值存储与退出边界——**`accepts` 里的 `return` 只结束本次 `accepts` 调用。** 如果它返回了 `false`，外层的 `match` 守卫就知道本次匹配失败，规规矩矩地继续去尝试下一个 `_ => false` 分支。
2. **卫生重命名（Hygienic Renaming）**：展开产生的形参绑定和内部局部变量全部拥有全新的编译器内部身份（Identity），哪怕你的 `inline fn` 里有个变量叫 `input`，亦免于和外层 `select(input: bool)` 的 `input` 发生名字冲突。

---

## 三、 当 `inline fn` 含有局部 `Drop` 资源——作用域清理一分不差

接下来考虑一个更硬核的问题：
如果被内联展开的函数 `accepts` 内部创建了一个带有 `Drop` 析构函数的局部资源 `let scratch = Scratch {};`，当 `accepts` 被展开到调用处（甚至展开到模式匹配的 `if` 守卫里。）时：
**这个 `scratch` 是在 `accepts` 展开块结束的瞬间立刻被 `Drop`，还是会被稀里糊涂地拖延到外层调用者函数结束时才 `Drop`？**
**如果模式守卫最终返回了 `false`（匹配失败，准备尝试下一个 `match` 分支），`scratch` 的 `Drop` 是否会漏掉？**

让我们写一段带打印追踪的 进阶代码来验证：

```flurry
use std.prelude.*;

struct Scratch {
    tag: u32,
}

impl Drop for Scratch {
    fn drop(*self) {
        println("drop scratch {}", self.tag.*);
    }
}

inline fn check_with_scratch(value: bool) -> bool {
    let scratch = Scratch { tag: 1u32 }
    if value {
        -- 即使走提前 return 分支，scratch 也必须在离开本次 inline 调用边界时立刻清理。
        return true;
    }
    false
}

fn process_signal(sig: bool) {
    println("before match");
    sig match {
        -- 当 sig 为 false 时，check_with_scratch(false) 依然创建并立刻清理了 scratch，
        -- 然后守卫失败，进入下面的 `_` 分支。
        v if check_with_scratch(v) => println("matched true branch"),
        _ => println("matched fallback branch"),
    }
    println("after match");
}

test {
    println("=== Case 1: sig = true ===");
    process_signal(true);

    println("=== Case 2: sig = false ===");
    process_signal(false);
}
```

来看看这段程序的输出顺序：

```text
=== Case 1: sig = true ===
before match
drop scratch 1
matched true branch
after match
=== Case 2: sig = false ===
before match
drop scratch 1
matched fallback branch
after match
```

仔细观察 `drop scratch 1` 出现的位置。
无论 `check_with_scratch` 是走 `return true;` 提前返回，还是走末尾的 `false` 返回（导致守卫失败转入 `_` 分支），**`drop scratch 1` 永远精准地发生在 `check_with_scratch` 逻辑退出的那一刻**——既没有因为内联展开而把 `scratch` 的析构推迟到 `after match` 之后，也没有因为模式守卫失败回退而漏掉 `scratch` 的清理。

下面通过图示总结 Flurry 内联展开的三项核心规则：

```text
  ┌─────────────────────────────────────────────────────────────────────────┐
  │                 Flurry 内联展开 (Inlining) 的三项核心规则                   │
  ├─────────────────────────────────────────────────────────────────────────┤
  │ 核心规则 1：【实参单次求值】                                                 │
  │         普通实参严格按调用处从左到右顺序各求值一次，绑定到新鲜形参槽；       │
  │         哪怕函数体引用了形参 10 次（或 0 次），实参表达式也只执行 1 次      │
  │                                                                         │
  │ 核心规则 2：【指针别名真实保持】                                             │
  │         新鲜的是形参绑定的名字，不是指针指向的物理对象                     │
  │         若传入 (x.ref, x.ref)，展开后编译器依然知道两个指针指向同一个 x    │
  │                                                                         │
  │ 核心规则 3：【语义作用域与 Drop 边界不合并】                                  │
  │         消除的是机器级的 call/ret 指令，保留的是完整的逻辑退出边界          │
  │         callee 的局部对象在本次调用结束处立即 Drop，不会拖延到 caller 末尾  │
  └─────────────────────────────────────────────────────────────────────────┘
```

### 顺便看一眼：在模式守卫中，什么操作会被编译器报错？

虽然 `inline fn` 允许你在模式匹配的守卫或 `and expr is pat` 表达式里创建像 `Scratch` 这样在函数内部自产自销、且 `Drop` 确定顺利通过的无指针局部对象，但如果你试图通过一个 `inline fn` 在模式守卫里**消费调用者外层的仿射资源**，或者**修改调用者外层的变量**，编译器依然会按模式候选表达式的规则予以报错：

```flurry
struct Ticket {}
fn consume_ticket(t: Ticket) {}

inline fn sneaky_guard(t: Ticket) -> bool {
    consume_ticket(t);
    true
}

-- ❌ 编译期直接报错：模式匹配的守卫随时可能匹配失败并回退，
--    就算通过 inline fn 展开，也禁止在守卫表达式里消耗外部原有的仿射所有者。
fn bad_guard_consume(flag: bool, external_ticket: Ticket) -> bool {
    flag match {
        v if sneaky_guard(external_ticket) => v,
        _ => false,
    }
}
```

---

## 本篇小结

| 对比维度 | C 语言函数式宏 `#define` | C/Rust 的普通 `inline` | Flurry 的 `inline fn` |
| :--- | :--- | :--- | :--- |
| **实参求值次数** | ❌ 按文本出现次数重复求值 | ✅ 求值一次 | ✅ **严格按从左到右各求值一次（含未使用实参的副作用）** |
| **内部 `return` 行为** | ❌ 直接从外层调用者函数返回 | ✅ 只结束被调函数 | ✅ **生成私有返回边界，只结束本次展开调用** |
| **局部 `Drop` 时机** | ❌ 无作用域隔离，容易污染外层 | ✅ 按函数边界清理 | ✅ **在展开后再跑资源验证，精准在逻辑调用出口处清理** |
| **与模式守卫/效应协同** | ❌ 无类型与资源安全检查 | ⚠️ 前端/中端往往视为黑盒调用 | ✅ **在 NIR 确定性展开，支持守卫观察验证与局部效应消去** |

---

### 下一篇预告

在这一篇里，我们看到了 `inline fn` 作为“带内联意向的函数”是如何工作的。如果遇到递归或者展开深度超限，编译器可以安全地保留 `inline fn` 的普通函数调用。

但是，在很多元编程和零开销抽象场景下（比如我们要写一个必须把代码织入调用者当前上下文的代码生成函数，甚至这个函数需要直接读写调用者作用域里的某个特定上下文变量 `implicit quote context: *Context`），**“尽量展开、实在不行就留着普通调用”是欠缺的——我们要求编译器必须在编译期 100% 完成代码接入。**

这就是我们在本篇开头提到的第三位主角：**`quote fn`**。
- 为什么 语言规范反复强调：**“`quote fn` 并非属 `comptime fn`，普通实参仍求值一次”**？
- 当我们在 `quote fn twice(value: u32) -> u32 { value + value }` 里把 `value` 写了两次，传入 `twice(produce())` 时，它是怎么做到既把 `value + value` 的代码结构直接贴进调用处，又保证 `produce()` 只执行一次的？
- 什么是 `implicit quote` 上下文输入？

在下一篇 **《Flurry 演进手记 (034)：实参只求值一次的代码拼装——quote fn 与普通宏的本质区别》** 中，我们将揭开 `quote fn` 的神秘面纱。
