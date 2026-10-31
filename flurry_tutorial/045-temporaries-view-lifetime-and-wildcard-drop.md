# Flurry 演进手记 (045)：那些一闪而过的匿名值——临时值生命周期、视图保活与 _ 立即丢弃规则

在 [上一篇 (044)](./044-drop-elaboration-and-failure-paths.md) 中，我们摸清了具名局部变量和结构体字段的逆序 `Drop` 法则。只要一个值被绑定到了变量名（比如 `let bundle = ...;`），它的销毁时刻就稳稳地钉在词法作用域的右花括号 `}` 上。

但在日常编写 `RingBuffer` 的操作代码时，我们并并非每次都会给中间结果起名字。
看看下面这几行常见的链式调用与控制流代码：
1. `let len = acquire_lease(1u32).inspect();` —— 其中的 `acquire_lease(1u32)` 是一个**没有名字的匿名临时值（Temporary）**；
2. `let ptr = acquire_lease(2u32).ref;` —— 试图把匿名临时值的指针保存到变量 `ptr` 里跨语句使用；
3. `acquire_lease(6u32) match { { id: 6u32 } => println("matched"), _ => () }` —— 把匿名临时值直接放进 `match` 的主语位置；
4. `if acquire_lease(7u32).is_valid() { println("inside branch"); }` —— 把匿名临时值放在普通的 `if` 布尔条件里。

思考一个关键问题：
**上面这四行代码里的四个匿名 `acquire_lease(...)` 临时对象，分别会在哪一瞬间执行 `Drop`？哪些合法，哪些又会在编译期被当场击毙？**

如果你写过 C++ 或 Rust，你一定领教过它们在“临时对象生命周期延长（Temporary Lifetime Extension）”上的玄学——在 Rust 里，`let r = &make_temp();` 能把临时对象偷偷延长到和 `r` 一样长，但稍微改写成 `let r = make_temp().get_ref();`，临时对象就突然在分号处暴毙报错。

Flurry 摒弃了这种让人猜谜的隐式延长潜规则，用四条清晰、刚性的**临时值边界定律**统治一切（详见 `docs/resource-management/temporaries.md`）。让我们从 传统的踩坑现场开始。

---

## 一、 指望“临时对象自动续命”引发的悬垂指针

假设我们为 `RingBuffer` 定义了一个带自动释放日志的临时访问租约 `SlotLease`：

```flurry
use std.prelude.*;

struct SlotLease {
    id: u32,

    fn inspect(*self) -> u32 {
        self.id.*
    }

    fn is_valid(*self) -> bool {
        self.id.* > 0u32
    }
}

impl Drop for SlotLease {
    fn drop(*self) {
        println("drop {}", self.id.*);
    }
}

fn make_lease(id: u32) -> SlotLease {
    SlotLease { id: id }
}
```

现在，一位带着 C++/Rust“引用自动给临时对象续命”习惯的程序员，随手写下了初始写法：

```flurry
-- ❌ 编译期直接报错。
fn dangling_lease() {
    -- 他以为 `.ref` 取了指针赋给 `pointer`，编译器就会自动帮 `make_lease(2u32)` 续命到函数结束。
    let pointer = make_lease(2u32).ref;

    -- 错误：在上一行的分号 `;` 处，临时的 `SlotLease(2)` 已经被 Drop 销毁了。
    -- 此处解引用 `pointer.inspect()` 被编译器静态拦截。
    println("{}", pointer.inspect());
}
```

为什么 Flurry 坚断难为 `let pointer = make_lease(2u32).ref;` 自动延长临时对象的寿命？
下面拆解 Flurry 的第一和第二条临时值定律。

---

## 二、 完整表达式边界与“保存视图不续命”核心规则

### 定律 1：未转交的临时责任，在“所属完整表达式”结束时立即清理

什么叫**完整表达式（Full Expression）**？
- 一整条 `let` / `var` 初始化语句的右侧（直到末尾的 `;`）；
- 一整条赋值语句的动作（直到末尾的 `;`）；
- 一整条独立的表达式语句（直到末尾的 `;`）。

在一条链式调用 `let number = make_lease(1u32).inspect();` 中：
1. `make_lease(1u32)` 产生了一个匿名临时 `SlotLease(1)`；
2. 紧接着的 `.inspect()` 需要借用这个临时对象读取 `u32`，因此临时对象**保活支撑整条调用链完成访问**；
3. `.inspect()` 返回了一个独立的标量 `u32` 并交给 `number`，而那个匿名的 `SlotLease(1)` 并没有交给任何具名 Owner。
4. 因此，在这条初始化语句的分号 `;` 处（完整表达式结束时），`SlotLease(1)` **立刻执行 `Drop`**。

看看它的精确输出顺序：

```flurry
test {
    let number = make_lease(1u32).inspect();
    println("number {}", number);
}
-- 精确输出顺序：
-- drop 1      ◄── 在 `let number = ...;` 分号处就已经 Drop 了。
-- number 1    ◄── 随后才执行下一行 println。
```

### 定律 2：保存指针或视图，非自动延长临时 Owner 的寿命

理解了定律 1，你就明白为什么 `let pointer = make_lease(2u32).ref;` 会报错了：

```text
  ❌ 错误写法：`let pointer = make_lease(2u32).ref;`
  [调用 make_lease(2)] ──► 产生临时 Owner ──► 取地址 `.ref` 交给 `pointer`
                                │
                                ▼ (到达本行分号 `;`，没人接手临时 Owner 的按值责任。)
                       [临时 Owner 当场 Drop 销毁。💀]
                                │
                                ▼
  下一行 `pointer.inspect()` ──► 指向死尸的悬垂指针。编译期直接报错。

  ✅ 正确写法：先建立具名 Owner 绑定，再取视图。
  let owner   = make_lease(3u32);  ──► 责任正式转交给具名变量 `owner` (活到作用域 `}` 结束。)
  let pointer = owner.ref;         ──► 在 `owner` 存活期间，`pointer` 全程安全有效。
```

为什么 Flurry 不搞 C++/Rust 那套“特定语法下自动给临时对象续命”的规则？
因为隐式续命规则是一座充满陷阱的迷宫：一旦你把 `make_lease(2u32).ref` 包进一个函数调用 `identity(make_lease(2u32).ref)` 或者放进结构体字段里，C++ 和 Rust 的隐式续命就会悄无声息地失效。
而在 Flurry 中，规则十分简明：**想要一个对象跨语句存活，就堂堂正正地用 `let owner = ...;` 给它一个名字，直接表达它需要活到哪里。**

---

## 三、 普通布尔条件 vs `match` 主体的临时值保活边界

接下来看我们在开头提出的第 3 和第 4 个问题：
当匿名临时值出现在 **`if` 布尔条件**里，和出现在 **`match`（或 `if ... is ... do`）的匹配主语**里时，它们的销毁时机一样吗？

**完全有所差异。** 这正是 Flurry 控制流设计最细腻的地方（详见 `docs/resource-management/temporaries.md`）：

```text
  场景 A：普通布尔条件 `if make_lease(7u32).is_valid() { println("in then"); }`
  ┌──────────────────────────────────────────────────────────────────────┐
  │ [1] 求值条件：创建临时 `SlotLease(7)`，调用 `.is_valid()` 得到 `bool`│
  │ [2] 条件求值完成  在进入 `{ println("in then"); }` 分支体【之前】，  │
  │     临时 `SlotLease(7)` 已经没用了，立即清理 (输出 "drop 7")         │
  │ [3] 执行分支体 (输出 "in then")                                      │
  └──────────────────────────────────────────────────────────────────────┘

  场景 B：模式匹配主语 `make_lease(6u32) match { { id: 6u32 } => println("number 6"), _ => () }`
  ┌──────────────────────────────────────────────────────────────────────┐
  │ [1] 求值主语：创建临时 `SlotLease(6)` 作为被匹配的主体               │
  │ [2] 为什么不能在进入分支前销毁？因为选中的分支臂可能正在借用或解构   │
  │     这个主语内部的字段                                               │
  │ [3] 执行选中的分支臂 (输出 "number 6")                               │
  │ [4] 整个 `match` 表达式结束时，才清理临时主语 (输出 "drop 6")        │
  └──────────────────────────────────────────────────────────────────────┘
```

来写一段测试代码，亲眼见证这两者截然相反的输出次序：

```flurry
test {
    -- 1. 普通 bool 条件：临时值在进入分支正文【前】清理。
    if make_lease(7u32).is_valid() {
        println("inside if branch");
    }
    -- 输出顺序：
    -- drop 7
    -- inside if branch

    -- 2. match 主体（及 if-is 主体）：临时值保活支撑所选分支，在整个 match 结束【后】清理。
    make_lease(6u32) match {
        { id: 6u32 } => println("number 6"),
        _ => (),
    }
    -- 输出顺序：
    -- number 6
    -- drop 6
}
```

当然，编译器对 `match` 临时主体的保活也有着严格的防逃逸底线：临时主体只保活到整个 `match` 表达式结束，你禁止于分支臂里用 `.ref` 取这个临时主体的指针并作为 `match` 的返回值带到外面去继续访问。

---

## 四、 重温并深挖 `_` 的两面性与编译器 IR 边界

我们在第一卷第 006 篇初次认识 `_` 时就知道，`_` 是一个**免于建立按值绑定的通配符**。
现在站在第三卷资源管理与编译器实现的高度（详见 `docs/resource-management/temporaries.md`），让我们把 `_` 的两条核心规则与编译器的优化边界打通：

1. **对已有变量位置使用 `let _ = existing;`**：
   - 因为 `_` 无需建立新绑定，编译器区分了“对已有位置的观察”与“按值移动”，**免因写了 `let _ = existing;` 就把 `existing` 里的资源提前移出或销毁**。`existing` 后面依然可以正常调用 `.inspect()`，并在作用域末尾正常清理。
2. **对新产生的拥有值使用 `let _ = make_lease(5u32);`**：
   - 右侧的函数调用、副作用和资源创建**必须照常执行**——编译器禁止将 `let _ = make_lease(5u32);` 错误地当成“既然左边是 `_` 切勿结果，那干脆把右边的函数调用直接当死代码删掉”。
   - 因为右边造出了一个新的拥有值，而左边的 `_` 报错接手 Owner 责任，所以这颗新值就作为当前语句的未转交临时值，在 `;` 处**立即执行 `Drop`**。

> **💡 编译器实现视角：IR 拆解不会切碎临时值的生命期。**
> 在编译器内部（Normalized IR），一条复杂的初始化语句 `let x = a().b().c();` 往往会被拆成好几条平铺的中间指令（`t1 = a(); t2 = b(t1); x = c(t2);`）。
> Flurry 规范明确要求：**将一个初始化右侧拆成多条 IR 指令，免令每一条中间指令都变成提前清理的边界。** 编译器会为临时值严格保留源码级的“完整表达式范围”，确保链式调用中途的临时对象稳稳活到整条源码表达式结束。

让我们把这一章所有的临时值与 `_` 规则汇总成一段完整的验收代码：

```flurry
use std.prelude.*;

struct Trace {
    id: u32,

    fn inspect(*self) -> u32 {
        self.id.*
    }
}

impl Drop for Trace {
    fn drop(*self) {
        println("drop {}", self.id.*);
    }
}

fn make_trace(id: u32) -> Trace {
    Trace { id: id }
}

fn verify_temporaries() {
    -- 1. 同一调用链读取：先 drop 1，再打印 number 1
    let number = make_trace(1u32).inspect();
    println("number {}", number);

    -- 2. 具名 Owner 保活指针：先打印 number 3，函数末尾才 drop 3
    let owner = make_trace(3u32);
    let pointer = owner.ref;
    println("number {}", pointer.inspect());

    -- 3. 通配符作用于已有位置：不移出 value，先打印 number 4，函数末尾才 drop 4
    let value = make_trace(4u32);
    let _ = value;
    println("number {}", value.inspect());

    -- 4. 通配符作用于新产生的临时值：在当前分号处立即 drop 5，再打印 after
    let _ = make_trace(5u32);
    println("after");
}

test {
    verify_temporaries();
}
```

---

## 本篇小结

通过追踪每一颗匿名临时值的轨迹，我们完整拼出了 Flurry 的临时对象生命周期版图：

| 场景 | 示例代码 | 临时对象的精确清理时机 | 核心设计理由 |
| :--- | :--- | :--- | :--- |
| **链式调用提取值** | `let n = make_trace(1).inspect();` | 在整条初始化语句的 `;` 处立即 `Drop` | 取得的 `u32` 独立于临时对象，未转交责任在完整表达式末尾结清 |
| **保存临时对象的指针** | `let p = make_trace(2).ref;` | **编译期静态报错。** | 避免脆弱的隐式续命规则；跨语句使用视图必须先建立具名 Owner |
| **普通 `bool` 条件** | `if make_trace(7).ok() { ... }` | 在进入选定分支正文**之前**立即 `Drop` | 布尔条件求值完毕后，临时对象使命已结束 |
| **`match` / `if-is` 主体** | `make_trace(6) match { ... }` | 保活支撑所选分支，在整个 `match` 结束时 `Drop` | 分支内部模式可能正在借用或解构该临时主体的字段 |
| **`let _ = ...` 两种情况** | `let _ = existing;` vs `let _ = make_trace(5);` | 前者原地保留不提前销毁；后者在 `;` 处立即 `Drop` | `_` 免于建立按值绑定，也不删除右侧真实发生的调用与副作用 |

### 下一篇预告

在前面的篇章中，我们已经多次使用过指针 `*T`（用 `.ref` 取地址，用 `.*` 解引用）。
如果你写过 Rust，你一定有一个憋了很久的疑问：
> “在 Rust 里，指针/引用被严格拆成共享不可变引用 `&T` 和独占可变引用 `&mut T`——只要存在一个 `&mut T`，任何其他引用都不准同时存在（Aliasing XOR Mutability）。为什么在 Flurry 里，指针只有一种 **`*T`**？如果我把同一个变量的指针 `num.ref` 同时传给函数的两个参数 `first: *i32` 和 `second: *i32`，并在函数里先写 `first.* = 11` 再读 `second.*`，Flurry 为什么非但零报错，还能精准推导它的行为？”
> “更进一步：如果一块内存被释放后，操作系统的分配器碰巧在**完全相同的物理内存地址**上又分配了一个新对象，以前留下来的旧指针 `*T` 能否‘借尸还魂’通过检查？”

在下一篇 **《Flurry 演进手记 (046)：为什么 Flurry 无需 `&mut` 独占锁——指针目标身份、世代（Generation）与顺序别名》** 中，我们将正面揭开 Flurry 指针模型与世代追踪系统的核心机密。
