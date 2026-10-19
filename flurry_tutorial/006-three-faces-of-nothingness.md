# Flurry 演进手记 (006)：空无一物的三种面孔——()、null、_ 与危险的 undefined

在 [上一篇 (005)](./005-literals-symbols-and-custom-suffixes.md) 中，我们给数字贴上了类型安全的单位标签 `100ms` 和 `250us`。今天，我们要为传感器监控固件编写**采样会话管理与生命周期追踪模块**。

在写这个模块时，我们会频繁遇到人类语言里都叫“什么都没有 / 空 / 忽略”的四个场景：
1. 一个函数只负责把寄存器清零，**不需要返回任何有意义的数据**；
2. 传感器处于休眠状态，本次查询**没有采集到读数**；
3. 调用了一个返回句柄或状态的函数，但我们**现在不想用这个返回值，想直接丢弃或忽略它**；
4. 在底层驱动里分配了一块内存槽位，在硬件写入前**暂时还没有初始化**。

在许多语言里，这四种概念常常被揉成一团（比如用 `void*`、`NULL`、`None`、`undefined` 混着使），或者在生命周期上藏着惊天暗雷（比如在 Rust 里写 `let _ = lock.lock();` 会导致锁在当前分号处瞬间释放，而写 `let _guard = lock.lock();` 却会持有到作用域结束！）。

在 Flurry 中，这四个场景分别对应四个截然不同的语法构造：**`()`**、**`null`**、**`_`** 与 **`undefined`**（详见 `docs/basics/unit-null-undefined.md`）。注意：在 Flurry 的语法分类中，它们**不是普通字面量**，而是各自拥有独立语义的构造表达式或通配符！

让我们从一段把“空”用出诡异 Bug 的 `v0` 代码开始。

---

## 第一版：混淆“无结果”与“空值”，以及神秘消失的追踪器（v0）

假设我们定义了一个带 `Drop` 析构日志的传感器会话追踪器 `SessionTrace`，用来在进入采集区域时打印启动日志、离开时自动打印 `"drop <id>"` 关闭硬件采样门控：

```flurry
use std.prelude.*;

struct SessionTrace {
    id: u32,

    fn inspect(*self) -> u32 {
        self.id.*
    }
}

impl Drop for SessionTrace {
    fn drop(*self) {
        println("drop {}", self.id.*);
    }
}

fn start_session(id: u32) -> SessionTrace {
    println("start {}", id);
    SessionTrace { id: id }
}
```

现在，一位刚接手项目的程序员写了下面这段测试代码 `v0`，他以为 `let _ = ...` 只是“给一个我不想起名字的变量占位，让它活到函数结束”：

```flurry
fn run_measurement_v0() {
    -- 他以为会话 1 会一直保持开启，直到 run_measurement_v0 结束才 drop！
    let _ = start_session(1u32);
    println("=== sampling sensor data ===");
}
```

当他运行这段代码时，控制台打印出来的顺序让他惊呆了：

```text
start 1
drop 1
=== sampling sensor data ===
```

**为什么在打印 `"=== sampling sensor data ==="` 之前，`SessionTrace(1)` 就已经被提前 `drop` 掉了？！**
更奇怪的是，当他又写了一段把已有变量赋给 `_` 的代码时：

```flurry
fn check_existing_v0() {
    let s = start_session(2u32);
    let _ = s; -- 他以为这行会把 s 移动并立刻销毁掉！
    println("still alive: {}", s.inspect());
}
```

这段代码居然**没有报错**，而且先输出了 `still alive: 2`，直到函数最后才输出 `drop 2`！

为什么同样是 `let _ = ...`，右边放 `start_session(1u32)` 时当场销毁，右边放已有的变量 `s` 时却连移动都没发生？
让我们把 `()`、`null`、`_` 和 `undefined` 排成一排，用 Code Cartoons 彻底照亮它们的底层机制！

---

## 第二版：理清 `void`/`()` 与 `?T`/`null`——“确定只有一个空盒” vs “盒子可能不存在”（v1）

在深入 `_` 的生命周期奥秘之前，我们先用最清晰的物理模型把前两个面孔——**`()`** 和 **`null`**——彻底分清（详见 `docs/basics/primitives.md` 与 `docs/basics/unit-null-undefined.md`）。

```text
  面孔一：单元类型 `void` 与唯一值 `()`
  ┌──────────────────────────────────────────────────────┐
  │ 类型：void                                           │
  │ 唯一合法值：()                                       │
  │ 物理意义：“操作已正常完成，但没有额外信息需要携带”     │
  │ 内存大小：0 字节（编译期已知唯一值，无需运行时比特） │
  └──────────────────────────────────────────────────────┘

  面孔二：可选类型 `?T` 的无值构造 `null`
  ┌──────────────────────────────────────────────────────┐
  │ 类型：?T (如 ?u8, ?SessionTrace)                     │
  │ 两种状态：有值 T  │  无值 null                       │
  │ 物理意义：“这里本来可以装一个 T，但这次是空的”         │
  │ 绝不混用：null 不能赋给普通指针 *T，也不是 void！     │
  └──────────────────────────────────────────────────────┘
```

在 C 或 Java 里，`void` 不是一个正经的值类型（你不能声明一个 `void x;` 变量，也不能在泛型里传 `void`），而 `NULL` 更是可以塞进任何指针里的万恶之源。

而在 Flurry 中：
1. **`void` 是堂堂正正的单元类型（Unit Type），`()` 是它唯一的实例值**。
   - 当一个函数不写返回类型（如 `fn reset_hardware() { ... }`）时，它实际上就是 `fn reset_hardware() -> void`。
   - 当你在 `match` 的某个分支里什么都不想做时，写 `_ => ()` 就是交出一个类型为 `void` 的 `()` 值！
2. **`null` 只属于可选类型 `?T`**。
   - 普通指针 `*T` 是**非空的**；如果你想表达一个“可能为空的指针”，必须显式写成 `?*T`，这时才能用 `null` 构造它的无值状态。
   - 你永远不可能把 `()` 传给期望 `?u8` 的地方，也不可能把 `null` 当成 `void` 返回！

```flurry
-- 省略 `-> void`，等价于返回 `void`
fn pulse_reset_pin() {
    -- 也可以显式写出单元值 `()`
    ();
}

fn poll_optional_reading(ready: bool) -> ?u8 {
    if ready {
        42u8 -- 自动注入 ?u8 的有值分支
    } else {
        null -- 构造 ?u8 的无值分支
    }
}
```

---

## 第三版：通配符 `_` 与临时值生命期——为什么 `let _ = expr` 如此独特？（v2）

现在，让我们回到 `v0` 里那个让人困惑的谜题：**通配符 `_`**。

在 Flurry 中，`_` 是一个**不建立任何按值绑定的通配符模式（Wildcard Pattern）**（详见 `docs/resource-management/temporaries.md`）。
要理解为什么 `let _ = start_session(1u32);` 和 `let _ = s;` 表现完全不同，只需要记住 Flurry 资源管理的两条黄金定则：

### 定则 1：临时值的责任边界是“所属完整表达式（Full Expression）”

当你调用 `start_session(1u32)` 时，它产生了一个需要被 `Drop` 清理的临时对象。
- 如果你写了 **`let guard = start_session(1u32);`**：临时对象的拥有权（Owner 责任）被**正式转交给了具名绑定 `guard`**。因此，它会一直活到 `guard` 所在的作用域（大括号）结束时才被清理。
- 如果你写了 **`let _ = start_session(1u32);`**：因为 `_` **根本不建立绑定**，没有任何具名 Owner 来接手这个新造出来的 `SessionTrace`！于是，这个值依然属于当前这条初始化语句的**未转交临时值**。一旦这条完整表达式（在 `;` 处）执行完毕，编译器就会立即兑现清理责任，当场调用 `drop`！

### 定则 2：`_` 作用于已有位置时，绝不额外移出（Move）已有值！

那么，为什么 `let s = start_session(2u32); let _ = s;` 又不会销毁 `s` 呢？
因为 `_` 的语义是**“此处不需要绑定这个值”**，而不是“把右边的值吸进黑洞销毁”！
- 当右边是 `start_session(1u32)` 时，函数调用本来就已经发生、新值本来就已经造出来了，因为没人接手，所以在语句末尾按临时值清理；
- 当右边是一个**已经存在的变量位置 `s`** 时，既然左边的 `_` 根本不需要建立新绑定，编译器就**不会从 `s` 的位置把值移出（Move）**！`s` 依然安然无恙地呆在原地，继续活到作用域结束！

让我们画一张 Code Cartoon，把这三种写法的生命周期轨迹刻在脑海里：

```text
  场景 A：具名绑定 `let guard = start_session(1);`
  [调用 start_session(1)] ──(责任转交给 guard)──► 跨越后续语句... ──► [作用域 `}` 结束时 Drop!]

  场景 B：通配丢弃新临时值 `let _ = start_session(1);`
  [调用 start_session(1)] ──( `_` 不接手责任！)──► [在当前 `;` 完整表达式结束时立即 Drop!]

  场景 C：通配观察已有位置 `let s = start_session(2); let _ = s;`
  [s 持有责任] ──► `let _ = s;` (`_` 不建立绑定，不从 s 移出值！) ──► [s 继续有效，作用域结束时 Drop!]
```

### 附带铁律：保存临时值的指针（`.ref`）绝不会自动延长临时值的寿命！

如果你写惯了 Rust 或 C++，你可能听说过“临时对象生命周期延长（Temporary Lifetime Extension）”——在 Rust 里写 `let r = &temp();` 有时候编译器会偷偷帮你在栈上把临时对象延长到和引用一样长，但在复杂表达式里又会突然失效。

Flurry 拒绝这种让人猜谜的隐式延长规则（详见 `docs/resource-management/temporaries.md`）：
**保存引用或视图，绝不会自动延长来源临时 Owner 的寿命！**

```flurry
-- ❌ 编译期直接拒绝！
fn dangling_temporary() {
    -- `start_session(99u32)` 是临时值，在这条语句的分号 `;` 处就已经被 Drop 清理了！
    -- 编译器静态检查发现 `ptr` 指向了已销毁的临时对象，当场拒绝编译！
    let ptr = start_session(99u32).ref;
    println("{}", ptr.inspect());
}

-- ✅ 正确做法：先给 Owner 建立明确的具名绑定，直接表达它需要活到哪里！
fn valid_owner_lifetime() {
    let owner = start_session(99u32);
    let ptr = owner.ref;
    println("{}", ptr.inspect());
}
```

唯一一个会自动保活临时主体的特殊控制流是 **`match` 与 `if ... is ... do`**：当你写 `start_session(6u32) match { { id: 6u32 } => println("ok"), _ => () }` 时，作为匹配主体的临时值会一直保活到选中的分支臂执行完毕，然后在整个 `match` 表达式结束时立即清理。

---

## 第四版：危险的第四张面孔——`undefined` 的安全边界（v3）

最后，我们来看看那位系统程序员心心念念的 **`undefined`**。

在 Flurry 的语法中，`undefined`（`undefined_expr -> undefined`）代表**未初始化或未定义的占位**（详见 `docs/basics/unit-null-undefined.md`）。

但是，请把下面这条警告标成鲜红色：
> **在默认的 `safe` 代码中，你绝不能用 `undefined` 来逃避初始化检查、读取未定义比特、或者绕过仿射资源的清理契约！**

为什么？还记得我们在 `AGENTS.md` 和 `docs/resource-management/moves-and-initialization.md` 中强调的原则吗：
- 在 Flurry 中，每个绑定（`let` / `var`）和每个结构体字段都有着严格的初始化状态追踪（完整初始化、部分初始化、已移出）；
- 一个未初始化的槽位里面装的是随机的垃圾比特，如果允许在安全代码里随便写 `let x: SessionTrace = undefined;` 然后让作用域末尾去调用 `x.drop()`，或者把 `undefined` 当成合法的 `Packet` 去匹配，整个类型安全和资源安全大厦就会瞬间崩塌！
- 因此，`undefined` 仅在受控的底层契约、形式化规范或特定受检上下文中作为未定义占位使用。在日常系统编程中，表示“暂时还没有值”请永远使用零成本、受编译器静态保护的 **`?T` 与 `null`**，或者在准备好初始值时再声明绑定！

让我们把正确区分了 `()`、`null`、具名保活与 `_` 的传感器会话代码整合成最终的 `v3` 版本：

```flurry
use std.prelude.*;

struct SessionTrace {
    id: u32,

    fn inspect(*self) -> u32 {
        self.id.*
    }
}

impl Drop for SessionTrace {
    fn drop(*self) {
        println("drop {}", self.id.*);
    }
}

fn start_session(id: u32) -> SessionTrace {
    println("start {}", id);
    SessionTrace { id: id }
}

-- 1. 无返回值函数（返回 void），在内部只做副作用并立即丢弃临时凭证
fn ping_session_immediately() {
    -- 使用 `_`：start_session(10) 产生的临时值在本行 `;` 结束时立即执行 drop！
    let _ = start_session(10u32);
    println("after ping");
}

-- 2. 需要跨语句保持会话存活：使用具名绑定 `_session` 或 `session`
fn run_guarded_sampling(enable: bool) -> ?u32 {
    if not enable {
        -- 使用 `null` 构造 `?u32` 的无值状态
        return null;
    }

    -- 具名绑定持有 Owner 责任，直到函数结束才会 drop！
    let session = start_session(20u32);
    let reading = session.inspect() + 5u32;
    println("sampling under session {}", session.inspect());
    reading
}

test {
    ping_session_immediately();
    -- 输出顺序：
    -- start 10
    -- drop 10
    -- after ping

    run_guarded_sampling(true) match {
        val? => println("got reading: {}", val),
        -- 使用 `()` 作为 match 分支的 void 结果值
        null => (),
    }
    -- run_guarded_sampling 内部输出顺序：
    -- start 20
    -- sampling under session 20
    -- drop 20
    -- 随后 match 输出：
    -- got reading: 25
}
```

---

## 本篇小结

现在，当你再次面对 Flurry 里代表“空”或“忽略”的记号时，脑海里应该有一张泾渭分明的地图：

| 记号 | 语法身份 | 所属类型 / 位置 | 核心语义与生命周期铁律 |
| :--- | :--- | :--- | :--- |
| **`()`** | `unit_expr` | `void` 的唯一实例 | 表示“正常完成但无额外信息”；零运行时内存开销 |
| **`null`** | `null_construct` | `?T`（可选类型） | 构造 `?T` 的“无值”分支；绝不能赋给普通非空指针 `*T` 或 `void` |
| **`_`** | `wildcard` | 模式 / 丢弃位置 | **不建立按值绑定**：对已有变量不触发移动；对新产生的临时值在当前完整表达式末尾立即清理 |
| **`undefined`** | `undefined_expr` | 受控未定义占位 | 非普通字面量；不能在安全代码中用来读取垃圾内存或跳过初始化与析构契约 |

同时，我们还掌握了**临时值（Temporaries）**的三条保命法则：
1. 无人接收的临时值在所属**完整表达式（`;`）**结束时立即清理；
2. 用 `.ref` 取临时值的指针**不会自动延长**临时值的寿命（跨语句使用必须先建立具名 Owner 绑定）；
3. `match` 和 `if ... is ... do` 的临时匹配主体会安全保活到所选分支执行结束。

### 留给下一篇的悬念

在上面的 `run_guarded_sampling` 函数里，我们写了 `if not enable { return null; }`。
随着传感器固件功能越来越丰富，告警分类器开始需要根据温度、电压、噪声计数做复杂的分级判断：
- 如果我们想把 `if ... else ...` 算出来的结果直接赋给一个 `let level = ...` 变量，而不需要先声明一个 `var level` 再在每个分支里赋值，该怎么写？
- 当遇到 5 个以上并列的复杂布尔条件（既不是匹配同一个枚举，也不是简单的相等，而是 `temp > 80 and volt < 3`、`noise >= 10` 等）时，写一长串 `else if ... else if ...` 丑得让人窒息，Flurry 有没有专门的并列多条件表达式？
- 还有我们在第 002 篇惊鸿一瞥过的 **`return null while not enable;`**——为什么条件守卫非得用 `while`，要是允许写成 `return x if cond` 到底会在语法解析器里引爆怎样的连环车祸？

在下一篇 **《Flurry 演进手记 (007)：状态机分支大扫除——表达式化的 `if`/`when` 与带 `while` 守卫的跳转》** 中，我们将给监控器的分支控制流来一场彻底的大扫除！
