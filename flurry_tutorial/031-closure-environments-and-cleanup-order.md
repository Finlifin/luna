# Flurry 演进手记 (031)：闭包环境的生死簿——%c fn 闭包限定类型、捕获状态更新与逆序清理

在 [上一篇 (030)](./030-lambdas-value-capture-and-pointer-observation.md) 中，我们学会了用 `|x| expr` 创建闭包，并明白了 Flurry“默认按值捕获，观察外部修改显式捕获 `.ref` 指针”的核心规则。

但当我们准备把一个捕获了外部变量的闭包写进函数签名、或者在多次调用它之后观察它的销毁行为时，三个关键问题浮出了水面：
1. **闭包的类型到底长什么样？** 在 C++ 和 Rust 中，每一个 Lambda 表达式都会生成一个编译器内部匿名的、独一无二的结构体类型——哪怕两个 Lambda 的签名和捕获的变量类型一模一样，它们也是两个完全不相等的类型。你只能用 `impl Fn(u32) -> u32` 或 `Box<dyn Fn>` 去擦除它。难道在 Flurry 里，闭包的类型也是个不可名状的黑盒吗？
2. **为什么 Flurry 不需要把闭包切碎成 `Fn`、`FnMut`、`FnOnce` 三个割裂的 Trait？** 一个闭包调用一次之后，还能不能调用第二次？
3. **如果闭包捕获了多个带 `Drop` 的硬件资源，调用或未调用离开作用域时，它们按什么顺序清理？**

这一篇，我们就来揭开 Flurry 受上下文模态类型论（Contextual Modal Type Theory, CMTT）启发而设计的明珠：**`%c` 闭包环境限定类型（Closure-Qualified Type）** 与闭包环境的完整生命周期法则（详见 `docs/type/qualified-types/closure-type.md` 与 `docs/functions/closure-environments.md`）。

---

## 一、 告别“匿名黑盒类型”——初识 `%c fn(...)` 闭包环境限定

假设我们在写一个传感器读数校准工厂，需要在函数里创建两个带偏移量的闭包 `add_offset` 和 `keep_larger`：

```flurry
use std.prelude.*;

fn compare_operations(offset: u32) {
    let add_offset = |value: u32| -> u32 value + offset;
    let keep_larger = |value: u32| -> u32 {
        if value < offset {
            offset
        } else {
            value
        }
    }

    println("sum: {}", add_offset(5));
    println("larger: {}", keep_larger(5));
}

test {
    compare_operations(10);
}
```

在普通日常编码时，你直接写 `let add_offset = ...`，编译器会自动推导出它的完整类型，你连一个多余的字符都不用写。
但是，如果你打开编译器的类型透视镜，或者想写一个返回这个闭包的工厂函数 `make_offset(offset: u32) -> ???`，它的类型到底是什么？

在 Flurry 中，它是极其透明的**前缀限定类型**：

```flurry
%{ offset: u32 = offset } fn(u32) -> u32
```

如下所示，拆解这个 `%c T` 闭包环境限定类型的每一个零件：

```text
  闭包限定类型语法:   % { offset : u32 = offset }   fn(u32) -> u32
                      ▲   ──────   ───   ──────     ──────────────
                      │     │       │      │              │
            环境限定前缀 `%` │    绑定类型  来源值/对象     主体函数类型 (T)
                        环境绑定名
```

看看这套设计相比传统语言“每个 Lambda 一个匿名唯一名义类型”的巨大优势（详见 `docs/type/qualified-types/closure-type.md`）：

1. **空环境自动消去（Empty Environment Elimination）**：
   如果一个 Lambda 什么外部变量都没捕获（比如 `|x: u32| x + 1`），它的环境是空的 `%{}`。在 Flurry 中，**空环境限定直接等于主体类型本身**：`%{} fn(u32) -> u32` 就是普普通通的 `fn(u32) -> u32`。
2. **不按构造位置制造虚假的名义隔离**：
   在上面的 `compare_operations(offset: u32)` 中，虽然 `add_offset` 和 `keep_larger` 是两个不同的 Lambda 表达式、函数体代码也不同，但因为它们的主体签名都是 `fn(u32) -> u32`、且所需的环境关系完全一致，**它们在类型层面的结构关系是相容的**。（当然，运行时调用时各自执行各自的函数体和实际环境值，绝不会混淆）。
3. **局部环境绑定一致改名不改变类型（$\alpha$-Equivalence）**：
   环境关心的是绑定的类型结构与真实外部对象的依赖身份（Identity），局部名字的拼写差异不会阻碍类型等价。

---

## 二、 为什么不需要 `Fn` / `FnMut` / `FnOnce` 三分天下？——调用后的状态追踪

在 Rust 中，为了知道一个闭包能不能多次调用、会不会修改环境，语言发明了 `Fn`、`FnMut`、`FnOnce` 三个 Trait。
而在 Flurry 中，**一个闭包能不能再次调用，根本不需要靠三个固定的接口标签去猜，而是直接看“一次调用之后，它的环境状态还满不满足下一次调用的前提”**（详见 `docs/functions/closure-environments.md` §调用后的状态）。

让我们用一张对比图，看清四种最典型的闭包调用状态转移：

```text
  闭包环境状态机（每个捕获字段分别追踪：持有有效值 / 已更新版本 / 已移出）：

  [情况 A：只读取 Copy 捕获] (如 |v| v + offset)
     调用前: [ offset: 持有 u32 ] ──(调用)──► 调用后: [ offset: 仍持有 u32 ]
     ──► ✅ 环境完好无损，可无限次重复调用。

  [情况 B：通过指针修改外部对象] (如 || { remaining.* -= 1; ... })
     调用前: [ remaining: *u32 (指向有效对象) ] ──(调用)──► 调用后: [ remaining: 仍在，目标内容版本更新 ]
     ──► ✅ 只要目标对象依然存活且满足入口契约，可继续调用。

  [情况 C：把非 Copy 捕获按值交出 (Move)] (如 || finish(ticket))
     调用前: [ ticket: 持有 Ticket ] ──(第 1 次调用)──► 调用后: [ ticket: ❌ 已移出 (Moved Out)! ]
     ──► 🛑 第一次调用合法；若试图进行第 2 次调用，因缺少 ticket 责任而在编译期直接拒绝。
```

来看看这三种情况在代码里的直观体现：

```flurry
use std.prelude.*;

struct Ticket {
    number: u32,
}

fn finish(ticket: Ticket) -> u32 {
    ticket.number
}

test {
    -- 1. 通过指针修改外部配额状态，并连续调用三次。
    var quota: u32 = 2;
    let remaining = quota.ref;
    let take_one = || {
        if remaining.* == 0u32 {
            false
        } else {
            remaining.* -= 1u32;
            true
        }
    }
    println("{} {} {} (left={})", take_one(), take_one(), take_one(), quota);
    -- 输出: true true false (left=0)

    -- 2. 按值消费非 Copy 捕获 Ticket：只能调用一次。
    let ticket = Ticket { number: 7u32 }
    let submit = || finish(ticket);
    println("submitted: {}", submit()); -- 输出: submitted: 7
}
```

如果有人对已经把 `ticket` 移交出去的 `submit` 闭包再调第二次，编译器根本不会被“表面上看 `submit()` 两次都没传参数”给骗过去，而是根据环境字段 `ticket` 已经处于“已移出”状态静态拦截：

```flurry
-- ❌ 编译期直接拒绝：第一次调用 submit() 已经将环境中的非 Copy 字段 ticket 移出。
fn repeated_consumption() {
    let ticket = Ticket { number: 8u32 }
    let submit = || finish(ticket);
    println("{}", submit());
    println("{}", submit()); -- ❌ 错误：环境中的 ticket 责任已在前一次调用中消耗。
}
```

---

## 三、 闭包环境的清理规则——首次捕获词法序与局部消费免清理

现在，我们来做一个极具挑战性的思维实验：
假设我们在作用域里先后声明了两个带 `Drop` 析构函数的追踪器：`first`（编号 1）和 `second`（编号 2）。
然后我们创建了一个闭包 `action`，在闭包的主体里，**我们先写了对 `second` 的引用，后写了对 `first` 的引用**：

```flurry
let action = || {
    println("{}", second.inspect()); -- 先出现 second。
    println("{}", first.inspect());  -- 后出现 first。
}
```

注意：**外部变量声明的顺序是 `first` $\to$ `second`，但闭包主体里首次捕获它们的词法顺序是 `second` $\to$ `first`。**
而且，假设我们创建完 `action` 之后，**根本没有调用它**就直接退出了函数作用域。

猜猜看：
1. `first` 和 `second` 会不会被清理？会清理几次？
2. 到底是按外部声明顺序（先 2 后 1）清理，还是按**闭包捕获的语义字段逆序（先 1 后 2）**清理？
3. 如果我们调用了一次 `action()`，而 `action` 内部只消费掉了 `first`，留下的 `second` 又该在什么时候清理？

让我们直接看权威规范（`docs/functions/closure-environments.md`）给出的标准代码与运行结果：

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

fn consume(resource: Trace) -> u32 {
    resource.id
}

-- 实验 A：闭包创建后未调用即离开作用域
fn leave_uncalled() {
    let first = Trace { id: 1u32 }
    let second = Trace { id: 2u32 }
    let action = || {
        -- 首次捕获的词法次序为：1. second, 2. first
        println("{}", second.inspect());
        println("{}", first.inspect());
    }
    -- action 未调用直接离开作用域。
}

-- 实验 B：调用时消费掉其中一个捕获 (first)，保留另一个 (second) 到环境退出
fn consume_one_keep_one() {
    let first = Trace { id: 1u32 }
    let second = Trace { id: 2u32 }
    let action = || {
        -- 首次捕获次序：1. first, 2. second
        let val = consume(first); -- first 在 consume 退出时立即被 Drop。
        println("still holding {}", second.inspect());
        val
    }
    println("result {}", action());
    -- 离开作用域时，环境中只剩下尚未移出的 second，只清理 second。
}

test {
    println("--- Experiment A ---");
    leave_uncalled();

    println("--- Experiment B ---");
    consume_one_keep_one();
}
```

看看精确的输出结果，并总结这两条核心规则：

```text
  --- Experiment A 输出 ---
  drop 1
  drop 2

  💡 为什么先 drop 1 再 drop 2？
  因为：
  1. 创建 action 时，first 和 second 的所有权都已经移入了 action 的环境（原局部变量不再重复清理）。
  2. 闭包环境的语义字段次序，严格由【去重后首次在闭包体内出现的词法顺序】决定。
     在 Experiment A 的闭包体中，先写了 `second`，后写了 `first`：
     环境字段顺序 = [ 字段 #0: second (2), 字段 #1: first (1) ]
  3. 环境销毁时，严格按语义字段【逆序清理】：先清理排在后面的字段 #1 (drop 1)，再清理字段 #0 (drop 2)。

  --- Experiment B 输出 ---
  drop 1
  still holding 2
  result 1
  drop 2

  💡 为什么 drop 1 发生在 still holding 2 之前，且最后只 drop 2？
  因为：
  1. 在调用 action() 期间，`consume(first)` 移走并销毁了 `first`（输出 `drop 1`），
     此时闭包环境的字段状态更新为：[ first: 已移出, second: 仍持有 ]。
  2. 当 `consume_one_keep_one` 结束、清理 `action` 环境时，编译器精确跳过“已移出”的 `first`，
     只清理仍然留在环境中的 `second`（输出 `drop 2`）。
```

### 最后一道防线：阻止捕获了局部指针的闭包逃逸

既然 `%c` 类型把闭包对外部环境的依赖完完整整地写在了脸上，当有人试图把一个捕获了短命局部变量指针 `local.ref` 的闭包返回到块外时，会发生什么？

```flurry
-- ❌ 编译期直接拒绝：闭包 action 捕获了指向块内局部变量 local 的指针 source，
--    当 do 块结束时 local 已经被销毁，绝不允许携带该指针依赖的闭包逃逸出块外。
test {
    let action = do {
        let local: u32 = 7u32;
        let source = local.ref;
        || source.*
    }
    println("{}", action());
}
```

相反，如果闭包在 `do` 块里直接按值捕获 `local`（复制了 `u32` 的值，没有任何指向局部栈槽的指针依赖），它就可以安全地跨出 `do` 块返回：

```flurry
-- ✅ 完全合法。环境自己持有 u32 副本，不依赖 do 块的局部栈槽继续存活。
test {
    let action = do {
        let local: u32 = 7u32;
        || local
    }
    println("{}", action()); -- 输出 7
}
```

---

## 本篇小结

| 核心概念 | Flurry 的机制与规则 | 工程收益 |
| :--- | :--- | :--- |
| **`%c T` 闭包环境限定** | `%{ name: Type = source } fn(...) -> R`；空环境 `%{} T` 自动消去为 `T` | 环境依赖在类型中显式可追踪；无捕获 Lambda 零成本化为普通 `fn` |
| **调用后状态追踪** | 逐字段追踪 `持有有效值` / `内容版本更新` / `已移出` | 无需割裂的 `Fn`/`FnMut`/`FnOnce` Trait，按真实状态判断能否再次调用 |
| **语义捕获次序与清理** | 按去重后**首次出现的词法顺序**建立环境字段，退出时**逆序清理仍存活的字段** | 已在调用中移出的字段绝不重复 `Drop`；未调用的闭包也会安全释放捕获资源 |
| **逃逸边界检查** | 检查返回或长寿命存储的闭包环境中所有可达指针的目标有效期 | 编译期 100% 拦截悬垂指针逃逸，同时放行按值拥有的独立环境 |

---

### 下一篇预告

现在，我们已经彻底搞懂了一个闭包自己是怎么创建、怎么更新状态、怎么清理环境的了。
但是，当我们作为库作者，要写一个接收回调函数的**高阶函数（Higher-Order Function）**——比如传感器数据流过滤器 `filter_stream(stream, predicate)`，或者把两个校准函数拼成一个新函数的组合器 `compose(first, second)` 时：
1. 如果我们把参数类型写成普通的 `action: fn(u32) -> u32`，为什么调用者一旦传入一个带捕获的闭包 `|v| v + offset` 就会被编译器编译期拒绝？
2. 我们该如何利用 **`where c: Type.vars.Closure`** 写出一个既能接收无捕获普通函数、又能接收任意带捕获闭包的零开销高阶接口 `action: %c fn(u32) -> u32`？
3. 当 `compose(first, second)` 把两个带不同环境 `%a` 和 `%b` 的闭包组合成一个新闭包返回时，返回类型的环境签名是怎么把两份环境精妙地装在一起的？

在下一篇 **《Flurry 演进手记 (032)：写一个零开销的过滤器——高阶函数接口、环境传递与回调纯度契约》** 中，我们将亲手打造支持环境多态的高阶函数与函数组合器。
