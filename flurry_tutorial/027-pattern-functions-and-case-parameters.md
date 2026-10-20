# Flurry 演进手记 (027)：直接按数据形状分派函数——case 参数与多输入模式函数

在 [上一篇 (026)](./026-implicit-parameters-and-post-do-blocks.md) 中，我们学会了用 `implicit` 参数和 `post do` 尾随块把上下文传递和收尾逻辑整理得井井有条。

现在，我们的传感器分析仪要接入一套远程控制指令协议 `Command`。主控台会向传感器节点下发三种不同的命令：
1. `.idle`：让节点保持待机，不关联任何寄存器槽位（返回 `null`）；
2. `.read { slot }`：读取指定槽位的寄存器，返回被操作的槽位号 `slot`；
3. `.write { slot, value }`：向指定槽位写入新值，同样返回被操作的槽位号 `slot`。

此外，我们还需要写一个传感器双通道择优函数 `smaller_present(first: ?u32, second: ?u32) -> u32`：比较主备两个通道的可选读数，如果两个都没数就返回 `0`；只有一个有数就返回那一个；两个都有数就返回较小的那个。

在传统语言（或者我们前几篇的习惯）里，你会怎么写这两个函数？你一定会在函数体的第一行写一个大大的 `command match { ... }`，或者对 `(first, second)` 打包成元组再 `match`。
但当每个分支的处理逻辑越来越长、甚至一个命令路由器里有几十种指令时，所有分支全都挤在同一个巨大的 `{ ... }` 花括号里，缩进深得让人窒息。

在 Flurry 中，既然函数的职责本来就是按参数的形状分别处理，**为什么不直接把模式写在函数参数列表里，把一个大函数拆成几段干净利落的函数级分支呢？**
这一篇，我们就来解锁 **函数级模式（Pattern Functions）与 `case` 参数**（详见 `docs/functions/pattern-functions.md`）。

---

## 一、 把所有分支塞进同一个大肚子里的痛

让我们先看看传统的单函数体 `match` 写法：

```flurry
use std.prelude.*;

enum Command {
    idle,
    read { slot: u32 },
    write { slot: u32, value: u32 },
}

-- 所有变体的处理逻辑都挤在一个函数体内部
fn describe_local(command: Command) -> ?u32 {
    command match {
        .idle => {
            println("idle");
            null
        },
        .read { slot } => {
            println("read {}", slot);
            slot
        },
        .write { slot, value } => {
            println("write {} = {}", slot, value);
            slot
        },
    }
}
```

当只有 3 个三行的小分支时，`describe_local` 看起来还挺清爽。
但在真实的固件工程或编译器 Pass 里：
- 如果 `Command` 有 25 个变体，每个变体的处理逻辑有 30 行，这个 `match` 表达式就会膨胀成 800 行的巨无霸。
- 如果你想为 `(first: ?u32, second: ?u32)` 两个参数做联合匹配，你还得先在函数体里构造 `(first, second) match { ... }`，白白多套一层元组括号和一层缩进。

能不能先声明一个统一的**公共函数签名（Public Signature）**，然后像数学里的分段函数一样，针对每种输入模式单独写一段函数体？

---

## 二、 `case` 参数登场——把 `match` 提升到函数签名层

在 Flurry 中，你可以在包（Package）内部先声明一个不带函数体的**公共签名**，随后使用 **`case pattern`** 作为参数，为不同的输入形态分别编写函数体。

来看看用函数级模式重写后的 `describe`：

```flurry
use std.prelude.*;

enum Command {
    idle,
    read { slot: u32 },
    write { slot: u32, value: u32 },
}

-- 1. 公共接口签名：规定参数类型、返回类型与调用契约（注意末尾是分号 `;`）
fn describe(command: Command) -> ?u32;

-- 2. 分支定义：在对应锚定位置写 `case 模式`，成功后绑定直接在函数体内可用。
fn describe(case .idle) -> ?u32 {
    println("idle");
    null
}

fn describe(case .read { slot }) -> ?u32 {
    println("read {}", slot);
    slot
}

fn describe(case .write { slot, value }) -> ?u32 {
    println("write {} = {}", slot, value);
    slot
}

test {
    -- 调用者看到的依然是唯一的公共接口 `describe(cmd)`。
    let target = describe(Command.write { slot: 2u32, value: 7u32 });
    target match {
        null => println("no target"),
        slot? => println("target {}", slot),
    }
}
```

下面来看看编译器是如何把这 1 个公共签名和 3 个 `case` 分支组装成同一个函数的：

```text
  调用方: describe(Command.write { slot: 2, value: 7 })
             │
             ▼
  ┌──────────────────────────────────────────────────────────┐
  │ 公共接口契约: fn describe(command: Command) -> ?u32;      │
  │ (实参按普通调用规则只求值一次  case 不增加额外 runtime 参数)│
  └──────────────────────────┬───────────────────────────────┘
                             │ 编译器合成的匹配决策树
        ┌────────────────────┼────────────────────┐
        ▼                    ▼                    ▼
  [ case .idle ]    [ case .read {slot} ]  [ case .write {slot, value} ]
   绑定: (无)         绑定: slot             绑定: slot = 2, value = 7
   主体:              主体:                  主体: (✅ 命中执行。)
   println("idle")    println("read ..")     println("write 2 = 7")
   return null        return slot            return 2 (注入 ?u32)
```

> **💡 核心心智模型：`case` 参数不是“基于静态类型的 C++ 函数重载”。**
> 仔细看上面的三个分支：它们接收的静态类型全都是公共签名规定的 `Command`。
> 1. **`case` 不改变调用方式**：调用者依然传一个普通的 `Command` 值，`case` 绝不会多造出一个运行时参数，也不会把参数变成可选输入。
> 2. **包内封闭（Package-Closed）**：这种分散定义的函数（在规范中称为 `open fn`）的分支集合在它所属的 `package` 内是**封闭**的——外部依赖包不能随便给你的函数塞进新分支，因此编译器在编译当前包时就能 100% 静态检查所有分支是否互斥、是否穷尽。
> 3. **不依赖文件扫描顺序**：不要写互相重叠的模式去赌“谁先写在前面谁就先匹配”。各个分支应当构成互斥且完整的输入分区。

### 编译期拒绝例：分支间的绑定绝不串门

每个 `case` 分支都有自己完全独立的成功绑定环境。哪怕另外两个分支都解构出了名为 `slot` 的变量，在没有绑定 `slot` 的 `.idle` 分支里也绝对无法访问：

```flurry
fn invalid_describe(command: Command) -> u32;

-- ❌ 编译期直接拒绝：.idle 模式没有产生绑定 `slot`。
-- 就算下面两个分支都有同名的 `slot`，也绝不会泄漏到 .idle 分支里。
fn invalid_describe(case .idle) -> u32 {
    slot
}

fn invalid_describe(case .read { slot }) -> u32 {
    slot
}

fn invalid_describe(case .write { slot, value: _ }) -> u32 {
    slot
}
```

---

## 三、 多输入联合模式分派与 `quote fn` 代码接入

当函数有多个参数时，`case` 参数的威力才真正显现出来。
回到我们开头的第二个需求：写一个比较主备两个通道读数 `first: ?u32` 和 `second: ?u32` 的函数 `smaller_present`。

如果用 `case` 参数，我们根本不需要在函数体里手动构造元组 `(first, second)`，直接在两个参数位置上分别写 `case null` 和 `case val?` 即可。

```flurry
use std.prelude.*;

-- 公共签名：接收两个 ?u32
fn smaller_present(first: ?u32, second: ?u32) -> u32;

-- 组合 1：两个通道都为空
fn smaller_present(case null, case null) -> u32 {
    0u32
}

-- 组合 2：只有主通道有读数
fn smaller_present(case first?, case null) -> u32 {
    first
}

-- 组合 3：只有备用通道有读数
fn smaller_present(case null, case second?) -> u32 {
    second
}

-- 组合 4：两个通道都有读数，取较小值
fn smaller_present(case first?, case second?) -> u32 {
    if first < second {
        first
    } else {
        second
    }
}

test {
    println("{}", smaller_present(null, null)); -- 0
    println("{}", smaller_present(7u32, null)); -- 7
    println("{}", smaller_present(null, 9u32)); -- 9
    println("{}", smaller_present(7u32, 9u32)); -- 7
}
```

看看这四个分支：它们就像一张 $2 \times 2$ 的真值表一样直观。

```text
  多参数模式覆盖检查（必须覆盖整个笛卡尔积空间。）：

                 second 是 null          second 是 second?
              ┌───────────────────────┬─────────────────────────┐
  first 是    │ (case null,           │ (case null,             │
  null        │  case null)    ──► 0  │  case second?) ──► sec  │
              ├───────────────────────┼─────────────────────────┤
  first 是    │ (case first?,         │ (case first?,           │
  first?      │  case null)    ──► fst│  case second?) ──► min  │
              └───────────────────────┴─────────────────────────┘
```

> **🔍 语法与语义小贴士：单个参数穷尽 $\neq$ 组合穷尽。**
> 假设你只写了前三个分支，删掉了第四个 `(case first?, case second?)`。
> 此时虽然在第一个参数位置上你既写过 `null` 也写过 `first?`，在第二个参数位置上也既写过 `null` 也写过 `second?`，但**整组二维输入组合缺了 `(有值, 有值)` 这一格**。编译器会立刻捕获这个缺失组合并报未覆盖错误：

```flurry
-- ❌ 编译期直接拒绝：缺少同时匹配 (first?, second?) 的分支，多参数组合未穷尽。
fn bad_smaller(first: ?u32, second: ?u32) -> u32;
fn bad_smaller(case null, case null) -> u32 { 0u32 }
fn bad_smaller(case first?, case null) -> u32 { first }
fn bad_smaller(case null, case second?) -> u32 { second }
```

### 顺便剧透：`quote fn` 加上 `case` 参数，就是可复用的匹配分支（Quoted Arm）

在编译期代码生成（我们将在第 034 篇和第六卷深入展开）中，如果你把 `fn` 换成 **`quote fn`**，再配上 `case` 参数，会发生什么？
它会直接把“输入模式 + 成功绑定 + 分支主体”打包成一个可在调用处直接内联展开的匹配结构，而且**严格保证实参表达式只求值一次**：

```flurry
use std.prelude.*;

quote fn value_or_zero(value: ?u32) -> u32;

quote fn value_or_zero(case null) -> u32 {
    0u32
}

quote fn value_or_zero(case value?) -> u32 {
    value
}

fn fetch_sensor() -> ?u32 {
    println("sensor polled once!");
    7u32
}

test {
    -- fetch_sensor() 只会执行一次，打印一次 "sensor polled once!"，随后输出 7
    println("{}", value_or_zero(fetch_sensor()));
}
```

---

## 本篇小结

| 特性维度 | 函数体内 `match` (`初版`) | 函数级模式 `fn foo(case pat)` (`改进版`/`进阶版`) |
| :--- | :--- | :--- |
| **代码组织** | 所有分支挤在同一个函数体花括号内 | 先声明公共签名 `fn id(...) -> R;`，再按模式分散编写独立主体 |
| **多参数联合匹配** | 需先在函数体内手动打包元组 `(a, b) match` | 直接在各自参数位置写 `fn id(case p1, case p2)`，静态检查组合穷尽性 |
| **绑定作用域** | 限于各自的 match arm 内 | 限于各自的分支函数体内，同名绑定绝不跨分支共享或泄漏 |
| **封闭性与顺序** | 按 arm 书写顺序尝试 | 定义集合在所属 `package` 内封闭，不依赖跨文件扫描顺序 |

---

### 下一篇预告

现在，无论是用后缀 `match` 还是用 `case` 参数函数，我们已经能轻松解构传入的数据结构了。
但是在写一个复杂的传感器报文校验分支时，我们遇到了一个非常挠头的场景：
> “我要先匹配出数据包里的 `{ raw_reading, checksum }`，然后**立刻调用一个辅助函数 `compute_crc(raw_reading)` 算出校验值**，再紧接着用模式匹配检查算出来的结果是不是等于 `.valid(expected)`。如果不等，就自动回退到下一个匹配分支。”

在普通语言里，模式匹配只能拆解**已经现成存在**的字段，一旦你想在匹配中途“调个函数算个新值、或者写几行局部变量做个转换，再继续匹配那个结果”，你的 `match` 就被迫中断了。你只能在分支体里再嵌一层 `match`，而且一旦内层匹配失败，你已经回不到外层 `match` 的下一个候选分支了。

**能不能在模式匹配的半路上，直接插入一段计算表达式甚至局部代码块，把算出来的新值接着往下匹配，而且一旦匹配失败还能干干净净地撤销回退？**

在下一篇 **《Flurry 演进手记 (028)：在模式匹配中途算个值——and expr is pat 链式顺序匹配与局部观察块》** 中，我们将揭开 `and ... is ...` 链式匹配的底层机制。
