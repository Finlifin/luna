# Flurry 演进手记 (044)：析构函数背后的精密钟表——局部与聚合 Drop 次序、失败路径与环境约束

在 [上一篇 (043)](./043-self-referential-structs-and-safe-anchoring.md) 中，我们见识了编译器是如何在函数退出时检查返回值与剩余清理之间的依赖关系的。

现在，我们要为 `RingBuffer` 编写**多通道资源事务管理器（Channel Bundle）**。在底层系统编程中，资源的释放顺序并非“随便关掉就行”的小事，它是一台咬合严谨的钟表：
- 想象你先初始化了底层的 **DMA 内存池（`first`）**，再初始化了依赖该内存池的 **硬件传输队列（`second`）**，最后把它们组装进了一个顶层控制器 **`ChannelBundle`**；
- 当系统关闭、销毁 `ChannelBundle` 时：
  1. 必须**先执行顶层 `ChannelBundle` 自己的停机逻辑**（比如拉低硬件使能引脚）；
  2. 接着必须**先销毁后创建的 `second`（传输队列）**，最后才能销毁底层的 `first`（内存池）。如果顺序反过来——先把 `first` 内存池释放了，再去关正在往内存池刷尾数据的 `second` 队列，硬件当场就会踩进已释放内存。

如果你同时写过 C++ 和 Rust，这儿有一个容易被忽视、却足以在底层驱动里引发血案的差异：
- 在 C++ 中，局部对象和类成员变量**全部按声明的逆序析构**（后构造的先析构）；
- 而在 Rust 中，局部变量虽然按逆序析构，但**结构体字段和数组元素实际上是按声明的正序（先 `first` 后 `second`）析构的**。

那么，Flurry 选择了哪种顺序？当构造中途发生 `!` 错误提前退出，或者析构函数 `drop(*self)` 自己需要读取外部数据时，编译器又是如何运转的？
让我们从一段对清理顺序和析构依赖产生误判的 初始代码开始。

---

## 一、 当析构函数偷偷读取了已被改坏的缓冲区

假设我们写了一个退出日志守卫 `PrintOnDrop`，它内部持有一个字符串切片 `text: str`，职责是在当前作用域结束、`PrintOnDrop` 被销毁的那一刻，把 `text` 打印出来。

一位工程师写下了下面这段看似平淡无奇的代码初始写法：

```flurry
-- ❌ 编译期直接报错。
use std.prelude.*;

struct PrintOnDrop {
    text: str,
}

impl Drop for PrintOnDrop {
    fn drop(*self) {
        println("audit on exit: {}", self.text.*);
    }
}

fn bad_audit() {
    let text = "ring_init".to_string();
    -- observer 借用了 text 当前的字符缓冲区切片
    let observer = PrintOnDrop { text: text.as_str() }

    -- 随后工程师往 text 里追加了字符（可能触发堆缓冲区重新分配，释放旧缓冲区。）
    text.push_str("_done");

    -- 工程师心想：“我在 push_str 之后再也没碰过 observer 这个变量啊。为什么会报错？”
}
```

当编译器拦截掉 `bad_audit` 时，这位工程师深感困惑：
> “在 `text.push_str("_done")` 之后，我明明没有写任何一行访问 `observer` 的代码。为什么编译器说我在作用域末尾非法读取了已失效的字符？”

因为他忘了一件事：**`Drop` 并非机制，它是编译器在作用域出口为你实打实生成的一次未来函数调用。**
让我们先把 Flurry 自动清理（Drop Elaboration）的精密钟表拆开，看清它的每一颗齿轮（详见 `docs/resource-management/drop-elaboration.md`）。

---

## 二、 局部与聚合的统一逆序核心规则——从外到内，从后到前

在 Flurry 中，无论是在局部作用域还是在聚合类型（`struct`、元组、数组）内部，清理顺序都遵循一套高度统一、不拧巴的准则：

```text
  准则一：局部作用域绑定 ──► 严格按【声明逆序】清理。
  {
      let a = Trace { id: 1 }   ──► 第 1 个声明，最后清理 (第 2 步 Drop)
      let b = Trace { id: 2 }   ──► 第 2 个声明，最先清理 (第 1 步 Drop)
  }
  (注：后声明的 `b` 往往依赖先声明的 `a`，因此必须先销毁 `b` 再销毁 `a`。)

  准则二：聚合对象 (`Bundle`) ──► 先跑【父对象 Drop】，再按【字段逆序】清理剩余字段。
  struct Bundle {
      first:  Trace,            ──► 第 1 个字段，最后清理 (第 3 步 Drop)
      second: Trace,            ──► 第 2 个字段，次先清理 (第 2 步 Drop)
  }
  impl Drop for Bundle {
      fn drop(*self) { ... }    ──► 父对象自身 Drop，最先执行。(第 1 步 Drop)
  }
```

让我们画一张完整的时序图，看看当局部作用域与聚合结构体组合在一起时，精确的析构顺序是怎样的：

```flurry
use std.prelude.*;

struct Trace {
    id: u32,
}

impl Drop for Trace {
    fn drop(*self) {
        println("drop {}", self.id.*);
    }
}

struct Bundle {
    first: Trace,
    second: Trace,
}

impl Drop for Bundle {
    fn drop(*self) {
        println("bundle");
    }
}

test {
    let outside = Trace { id: 0u32 }
    let bundle = Bundle {
        first: Trace { id: 1u32 },
        second: Trace { id: 2u32 },
    }
}
```

```text
  `test` 作用域结束时的精确时钟滴答：
  ┌──────────────────────────────────────────────────────────────────┐
  │ 滴答 1 (清理后声明的 `bundle` - 第 1 步): 执行 `Bundle.drop`     │ ──► 输出: "bundle"
  │ 滴答 2 (清理 `bundle` 内部字段 - 逆序!): 先清理字段 `second`     │ ──► 输出: "drop 2"
  │ 滴答 3 (清理 `bundle` 内部字段 - 逆序!): 再清理字段 `first`      │ ──► 输出: "drop 1"
  │ 滴答 4 (清理先声明的 `outside`):         执行 `outside.drop`     │ ──► 输出: "drop 0"
  └──────────────────────────────────────────────────────────────────┘
```

仔细品味这个设计的精妙之处：
1. **为什么父对象的 `Bundle.drop` 最先执行？**
   因为在销毁 `Bundle` 的那一刻，`Bundle.drop(*self)` 往往需要读取或操作它肚子里的字段 `self.first` 和 `self.second`（比如把未刷盘的数据刷出去）。如果先把字段销毁了才调用 `Bundle.drop`，父对象的析构函数看到的就全是一片废墟了。
   而且，如果在 `Bundle.drop` 执行期间，你合法地把某个字段移走了，随后的自动字段清理就会聪明地**跳过那个已被移走的字段**，互异无重析构。
2. **为什么结构体字段、元组元素、数组元素全部采用“逆序清理”？**
   回忆我们在第 003 篇提到过的：在 Flurry 中，构造 `Bundle { first: ..., second: ... }` 时，字段是按声明顺序从前到后依次求值构造的（`first` 先诞生，`second` 后诞生，甚至后续字段的默认值可以依赖前面的字段）。既然**构造是从前到后**，那么**析构自然应该从后到前（逆序）**。这样无论是局部变量还是结构体字段，全都符合“后进先出（LIFO）”的栈式依赖法则。
3. **最后一次读取不决定析构时机**：
   不像某些带 GC 或激进死变量分析的语言，Flurry 的析构时机严格锚定在**词法作用域出口**（或显式移动/覆盖点），免因你在第 10 行之后没再显式读过变量，就在第 11 行偷偷把它提前析构掉。

---

## 三、 当构造半路翻车——失败路径上的精准回滚

现在考虑一个真实的故障场景：
在组装 `Bundle` 时，我们需要依次调用 `make_trace(1, false)!` 和 `make_trace(2, fail)!`。如果第 1 个字段成功造出来了，但在造第 2 个字段时触发了 `!BuildErr` 错误并通过 `!` 提前退出了——此时会发生什么？

来看看编译器的失败路径清理逻辑（详见 `docs/resource-management/drop-elaboration.md`）：

```text
  执行 `make_bundle(fail: true)` 的失败回滚轨迹：

  [第 1 步] `let first = make_trace(1, false)!;`
            ──► 成功。`first` (id=1) 已建立清理责任 ✅

  [第 2 步] `let second = make_trace(2, true)!;`
            ──► 失败。返回 `error BuildErr.Refused` ⚡
            ──► 控制流在此处通过 `!` 转向错误出口。

  [第 3 步] 错误出口清理：
            ├── `Bundle` 构造完成了吗？ ❌ 没有。跳过调用 `Bundle.drop`。
            ├── `second` 构造完成了吗？ ❌ 没有。跳过调用 `second` 的 `drop`。
            └── `first`  构造完成了吗？ ✅ 是的。立即调用 `first.drop()` (输出 "drop 1")。
```

写成代码验证一下：

```flurry
use std.prelude.*;

enum BuildErr {
    Refused,
}

fn make_trace(id: u32, fail: bool) -> !BuildErr Trace {
    return error BuildErr.Refused while fail;
    Trace { id: id }
}

fn make_bundle(fail: bool) -> !BuildErr Bundle {
    let first = make_trace(1u32, false)!;
    let second = make_trace(2u32, fail)!;
    Bundle { first: first, second: second }
}

test {
    make_bundle(true)! {
        .Refused => println("failed cleanly"),
    }
    -- 精确输出：
    -- drop 1
    -- failed cleanly
    -- （不会输出 "bundle" 或 "drop 2"。）
}
```

哪怕你在同一个表达式里直接写 `Bundle { first: make_trace(1u32, false)!, second: make_trace(2u32, true)! }`，由于 `Bundle` 整体尚未完成构造，当第二个字段求值失败时，编译器也**只会清理已经成功构造完成的第一个字段 `first`**，免对半成品调用 `Bundle.drop`。

---

## 四、 揭开 传统报错之谜——`Drop` 的未来读取前提与不可交换核心规则

现在，我们终于可以回到本篇开头的 `bad_audit`，徹底明白为什么编译器报错了它。

在 Flurry 中，**自动清理（Drop）并非在低层代码生成时才塞进去的“黑盒实现细节”，而是在 Normalized IR 阶段就直接展开并参与全部安全检查的一等公民操作。**

如下所示，对比正确版本 `valid_cleanup` 与错误版本 `bad_audit` 在作用域末尾的区别：

```text
  ✅ 正确版本 (`valid_cleanup`)：
  let text = "ring_init".to_string();              ──► (1) 声明 text
  let observer = PrintOnDrop { text: text.as_str() } ──► (2) 声明 observer (借用 text 的字符分配)
  -- 作用域结束，按逆序展开 Drop：
  -- [自动插入 1] observer.drop()  ──► 读取 `text` 的字符分配（此时 `text` 完好无损，✅ 合法。）
  -- [自动插入 2] text.drop()      ──► 释放 `text` 的字符分配

  ❌ 错误版本 (`bad_audit`)：
  let text = "ring_init".to_string();
  let observer = PrintOnDrop { text: text.as_str() }
  text.push_str("_done");                          ──► ⚡ 可能重新分配缓冲区，使旧字符切片失效。
  -- 作用域结束，按逆序展开 Drop：
  -- [自动插入 1] observer.drop()  ──► 试图解引用读取 `self.text.*`（旧字符分配已失效。💀 静态报错。）
```

非但如此，如果有人故意把声明顺序写反了——先声明 `observer`，后声明 `text`——导致退出作用域时**先销毁被借用的 `text`，再销毁借用者 `observer`**：

```flurry
-- ❌ 编译期直接报错。
fn wrong_declaration_order() {
    var observer = PrintOnDrop { text: "init" }
    let text = "dynamic".to_string();
    observer.text = text.as_str();
    -- 作用域结束。按声明逆序：
    -- 1. 先清理后声明的 `text`（释放了 "dynamic" 的内存。）
    -- 2. 再清理先声明的 `observer`（observer.drop() 试图读取已被释放的 `text`。报错。）
}
```

注意 Flurry 的一条严格规定（详见 `docs/resource-management/drop-elaboration.md`）：
> **当固定的逆序清理顺序无法满足依赖关系时，编译器直接报错该程序；编译器不会为了讨好检查而偷偷交换两个具有可观察行为的析构顺序。**

那如果一个结构体内部只是保存了一个普通裸指针 `*u32`，而它的析构函数**根本没有去解引用这个指针**呢？
Flurry 在这里再次展现了它的精准：如果该类型没有会解引用该指针的用户 `Drop`，仅仅“丢弃一个已经失效的指针位表示本身”并未访问目标内存，因此是允许的；只有当 `Drop` **真正要去读取或修改那个目标区域**时，编译器才强制要求目标必须活到 `Drop` 执行完毕之后。

让我们写出遵守声明顺序与访问契约的完整实现：

```flurry
use std.prelude.*;

struct PrintOnDrop {
    text: str,
}

impl Drop for PrintOnDrop {
    fn drop(*self) {
        println("audit on exit: {}", self.text.*);
    }
}

fn valid_audit() {
    -- 1. 先声明底层资源所有者 text（将在最后被清理）
    let text = "ring_init".to_string();
    -- 2. 先完成所有可能导致缓冲区重分配的修改
    text.push_str("_done");
    -- 3. 再声明依赖当前有效缓冲区的 observer（将在 text 之前最先被清理。）
    let observer = PrintOnDrop { text: text.as_str() }
}

test {
    valid_audit();
    -- 输出：audit on exit: ring_init_done
}
```

---

## 本篇小结

拆开自动清理引擎后，我们掌握了 Flurry 析构系统的四条核心原则：

| 维度 | Flurry 的精确规则 | 与其他语言的关键对比 |
| :--- | :--- | :--- |
| **局部作用域清理顺序** | 按局部绑定的**声明逆序**清理（覆盖赋值维持原状而未改次序，跳过未初始化与已移出） | 与 C++ / Rust 局部变量一致，但严格固定，不暗中交换析构次序 |
| **聚合类型清理顺序** | 先执行**父对象 `Drop`**，再按**逆序**清理剩余已初始化字段/元素 | 比 Rust 的字段正序析构更符合“从前到后构造、从后到前析构”的依赖规律 |
| **构造半路失败 (`!`)** | 仅逆序清理**已成功建立**的子责任；跳过调用尚未构造完成的父对象 `Drop` | 无需像 C 语言那样手写 `goto cleanup_1; cleanup_2;`，编译器自动按路径生成 |
| **`Drop` 的未来访问检查** | 将末尾 `Drop` 展开到 IR 中检查：若 `Drop` 读取了外部目标，该目标必须保持有效直到 `Drop` 完成 | 在编译期避免析构函数踩进已扩容或已提前释放的悬垂内存 |

### 下一篇预告

在今天的所有例子里，我们创建的 `Trace`、`Bundle` 和 `String` 都赋给了有名字的变量（比如 `let text = ...;`）。
但是，在实际写 `RingBuffer` 的链式调用时，代码里到处都是**没有名字的匿名临时值（Temporaries）**：
- 当我们写下 `let id = make_trace(1u32).inspect();` 时，那个匿名的 `make_trace(1u32)` 到底是在 `inspect()` 刚跑完的这一行末尾被 `Drop`，还是等到整个函数结束才被 `Drop`？
- 如果我们在 C++ 或 Rust 里写惯了 `let slice = make_string().as_str();`，指望编译器触发“临时对象生命周期延长（Temporary Lifetime Extension）”，在 Flurry 里为什么会被编译期报错？
- 在 `make_trace(6u32) match { ... }` 中，临时主体的寿命又是怎么精确覆盖被选中的分支臂的？

在下一篇 **《Flurry 演进手记 (045)：那些一闪而过的匿名值——临时值生命周期、视图保活与 `_` 立即丢弃规则》** 中，我们将深入追踪每一颗匿名临时值从诞生到湮灭的完整轨迹。
