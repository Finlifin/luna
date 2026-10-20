# Flurry 演进手记 (007)：状态机分支大扫除——表达式化的 if/when 与带 while 守卫的跳转

在 [上一篇 (006)](./006-three-faces-of-nothingness.md) 中，我们理清了 `()`、`null`、`_` 与临时值的生命周期。现在，让我们打开传感器监控固件里最臃肿的一个文件：**告警等级分类器（Alarm Classifier）**。

硬件工程师给我们的分类规则是这样的：
1. 如果传感器通道尚未使能（`not enabled`），或者正在自检校准中（`calibrating`），或者读数为 `0`（空心跳），直接跳过告警分类，提前返回 `.ignored`；
2. 否则，综合评估当前的**温度 `temp`**（摄氏度）和**连续噪声计数 `noise_count`**：
   - 当 `temp >= 100` 或者 `noise_count >= 10` 时，定级为严重故障 `.critical`；
   - 当 `temp >= 85` 且 `noise_count >= 3` 时，定级为高温伴随噪声 `.danger`；
   - 当 `temp >= 70` 时，定级为警告 `.warning`；
   - 其他所有情况，定级为正常 `.normal`。

如果按照传统 C 语言或早期命令式语言的写法，这段代码会变成什么样？先看基础写法。

---

## 一、 满屏的 `if-else` 阶梯与先声明后赋值的 `var`

先看传统的命令式写法：

```flurry
use std.prelude.*;

enum AlarmLevel {
    ignored,
    normal,
    warning,
    danger,
    critical,
}

fn classify_alarm_initial(
    enabled: bool,
    calibrating: bool,
    reading: u8,
    temp: i32,
    noise_count: u32,
) -> AlarmLevel {
    if not enabled {
        return .ignored;
    }
    if calibrating {
        return .ignored;
    }
    if reading == 0u8 {
        return .ignored;
    }

    -- 为了在分支里拿到结果，被迫用 `var` 先给一个假初始值。
    var level: AlarmLevel = .normal;

    if temp >= 100i32 or noise_count >= 10u32 {
        level = .critical;
    } else if temp >= 85i32 and noise_count >= 3u32 {
        level = .danger;
    } else if temp >= 70i32 {
        level = .warning;
    } else {
        level = .normal;
    }

    level
}
```

审视这段 30 多行的 初始代码，它有三个非常典型的问题：
1. **虚假的可变状态（`var level = .normal`）**：`level` 本来应该是一个一旦算出就不再改变的常量绑定（`let`）。但因为把 `if-else` 当成了不产生值的“语句”，我们被迫用 `var` 先填一个毫无意义的占位值 `.normal`，再在每个分支里手动写 `level = ...`。万一某个分支漏写了赋值，编译器也帮不了你。
2. **头重脚轻的 `else if` 链条**：当有四五个复合布尔条件并列时，`if ... else if ... else if ...` 既有视觉上的噪音，又掩盖了“这是一个多路条件分发表”的本质。
3. **占了整整 9 行的前置卫语句（Guard Clauses）**：开头那三个简单的提前返回检查，写了三遍 `if ... { return .ignored; }`，把真正的核心业务逻辑挤到了屏幕下半截。

让我们分两步，用 Flurry 的**表达式化分支（`if` / `when`）**与**`while` 转移守卫**把这段代码彻底重构。

---

## 二、 一切分支皆表达式——从 `if` 到并列多条件 `when`

在 Flurry 中，**`if` 和 `when` 都是純正的表达式（Expressions）**（详见 `docs/control-flow/basic.md`）。这意味着每个分支块的最后一个表达式，都会直接作为整个分支结构的结果值交付出来。

### 1. 作为表达式的 `if`（以及它没有 `else` 时的秘密）

对于简单的二元判断，你可以直接把 `if ... else ...` 赋给 `let`：

```flurry
let status_str = if temp >= 85i32 {
    "HOT"
} else {
    "OK"
} -- 记得第 004 篇的规则吗？以 `}` 结尾，省略分号。
```

这时你可能会问两个刁钻的问题：
- **问题 A：如果两个分支返回的类型不一样怎么办？**
  编译器会要求所有可达分支的结果类型必须能够统一（Unify）：

```flurry
-- ❌ 编译期直接拒绝。
let bad_if = if temp >= 85i32 {
    "HOT"   -- str 类型
} else {
    404i32  -- i32 类型，与 str 无法统一。
}
```

- **问题 B：如果一个 `if` 根本没有写 `else` 分支，或者某个分支直接 `return` 退出了，它的类型是什么？**
  Flurry 的类型系统对此有着极其严密的定义（详见 `docs/control-flow/basic.md`）：

```text
  情况 1：无 `else` 的普通 `if` ──► 正常完成时类型为 `void`（值为 `()`）
  if ready {
      println("ready");
  }
  (因为当 ready 为 false 时什么都不做，所以整体只能保证产出 `void`!)

  情况 2：某个分支确定退出（如 `return` / `break`）──► 该分支类型为底类型 `never`。
  let val: i32 = if ready {
      42i32          -- 类型为 i32
  } else {
      return .ignored; -- 类型为 never。never 可以融入任何类型，因此整体类型就是 i32。
  }

  情况 3：编译期常量条件确定必然退出 ──► 整个 `if` 的类型就是 `never`。
  if true {
      return .ignored;
  }
  -- 编译器知道控制流绝不会跨过上面这个 `if`，后面哪怕不写返回值也合法。
```

### 2. 用 `when { ... }` 消灭冗长的 `else if` 阶梯

既然 `if` 已经是表达式了，为什么 Flurry 还要引入 **`when`** 表达式？
对比一下你就明白了：
- **`expr match { ... }`**：是针对**同一个主语 `expr`** 的结构与模式拆解；
- **`if ... else ...`**：最适合**非此即彼（二选一）**的分支；
- **`when { cond1 => ..., cond2 => ..., else => ... }`**：专门用来处理**多个互不相同的布尔条件按顺序并列判断**的场景。

来看看 `when` 的工作原理图：

```text
  when {
      temp >= 100i32 or noise_count >= 10u32  => .critical,  ──► [第 1 步] 求值条件 1：若为 true，直接返回 .critical。
      temp >= 85i32 and noise_count >= 3u32   => .danger,    ──► [第 2 步] 若 1 为 false，求值条件 2...
      temp >= 70i32                           => .warning,   ──► [第 3 步] 若 2 为 false，求值条件 3...
      else                                    => .normal,    ──► [兜底步] 前面全为 false 时，执行 else 分支。
  }
```

注意 `when` 的几条贴心规则：
1. **按书写顺序短路求值**：从上到下依次计算 `=>` 左边的布尔表达式，一旦遇到**第一个为 `true`** 的分支，立刻执行它右边的代码块或语句，后面的条件不再计算。
2. **逗号分隔**：和 `match` 一样，`when` 的各个分支臂（arm）属于一般列表项，彼此之间用**逗号 `,`** 分隔。
3. **无 `else` 时的行为**：如果 `when` 没有写 `else => ...` 兜底臂，正常完成时它的类型也是 `void`（除非编译器按顺序分析发现前面有常量 `true => return ...` 导致所有可达路径都必然退出，此时类型为 `never`，且前面已经可达并正常完成的分支不会被后面的退出分支错误覆盖）。

---

## 三、 为什么提前退出守卫写 `while cond` 而不是 `if cond`？

用 `when` 干掉了 `else if` 阶梯后，我们再来看看函数开头那三个啰嗦的前置检查：

```flurry
if not enabled { return .ignored; }
if calibrating { return .ignored; }
if reading == 0u8 { return .ignored; }
```

在 Flurry 中，所有的控制转移语句（`return`、`break`、`continue`）都支持在末尾直接挂载一个**后置条件守卫**。
但是，请看清楚它的关键字——**它写作 `while cond`，绝对不写作 `if cond`。**

```flurry
return .ignored while not enabled;
return .ignored while calibrating;
return .ignored while reading == 0u8;
```

很多第一次见到这个语法的程序员都会好奇地问：
> **“在英语和 Ruby/Perl 里，后置条件不都写 `return x if cond` 吗？为什么 Flurry 非要选 `while` 这个词？”**

这绝不是拍脑袋的怪癖，而是因为：**在 Flurry 中，`if` 和 `when` 是合法的表达式前缀。**

下面来看看如果语言允许后置守卫用 `if`，解析器（和读代码的人）会遭遇怎样的语法车祸：

```text
  假设允许写 `return <expr> if <cond>`（问题假想图）：

  当你写下：
  return   a   if   cond   { ... }   else   { ... }
               ▲▲
               └── 救命。这个 `if` 到底是什么意思？
                   解释 A：返回变量 `a`，但仅当后面的 `cond { ... }` 成立时才返回？
                   解释 B：`a` 后面漏写了一个运算符（比如本来想写 `a + if cond { ... }`），
                           后面其实是一个完整的 `if` 表达式？
```

看到了吗？一旦你在 `return` 的表达式里有半点手滑（比如漏敲了一个 `+` 号，或者在复杂嵌套里），后置守卫的 `if` 和表达式开头的 `if` 就会在语法树里死死缠成一团，导致编译器报出不知所云的错误，或者悄无声息地曲解你的控制流。

而选用 **`while`** 作为语句转移守卫（`return expr? (while expr)?`、`break id? (while expr)?`、`continue id? (while expr)?`）时：
1. **在语法上 100% 零歧义**：因为 `while` 在 Flurry 里是**纯语句关键字**，永远不可能出现在普通值表达式的开头。解析器在 `return expr` 后面看到 `while` 的一瞬间，就知道返回值表达式已经彻底结束，接下来是转移守卫条件。
2. **在语义上同样自然**：英语中 `while` 本就有“在……成立的条件下（as long as / whereas）”的含义——`return .ignored while not enabled;` 读作“在未使能的条件下，返回 `.ignored`”，清晰、简练、一行搞定。

> **🔍 语法小贴士：求值顺序是先算返回值，还是先算 `while` 条件？**
> 当你写 `return compute_expensive() while should_exit;` 时，千万不用担心在 `should_exit` 为 `false` 时白白执行了 `compute_expensive()`。
> 根据 `docs/control-flow/basic.md` 的运行时规范：`return` / `break` / `continue` 带 `while` 守卫时，**永远先求值 `while` 后面的布尔条件**；只有当条件为 `true` 时，才会求值 `return` 的返回值表达式并执行跳转。

现在，让我们把 `while` 守卫与 `when` 表达式结合起来，看看最终版 `classify_alarm` 有多么赏心悦目：

```flurry
use std.prelude.*;

enum AlarmLevel {
    ignored,
    normal,
    warning,
    danger,
    critical,
}

fn classify_alarm(
    enabled: bool,
    calibrating: bool,
    reading: u8,
    temp: i32,
    noise_count: u32,
) -> AlarmLevel {
    -- 1. 用 `return ... while cond;` 一行一个卫语句，干脆利落。
    return .ignored while not enabled or calibrating or reading == 0u8;

    -- 2. 用 `when` 表达式直接算出最终结果，零可变状态 (`var`)。
    when {
        temp >= 100i32 or noise_count >= 10u32 => .critical,
        temp >= 85i32 and noise_count >= 3u32  => .danger,
        temp >= 70i32                          => .warning,
        else                                   => .normal,
    }
}

test {
    assumes classify_alarm(false, false, 10u8, 120i32, 0u32) == .ignored;
    assumes classify_alarm(true, false, 5u8, 90i32, 4u32) == .danger;
    assumes classify_alarm(true, false, 5u8, 40i32, 0u32) == .normal;
}
```

从 35 行缩减到了核心逻辑不到 10 行：没有一个多余的 `var`，没有一层啰嗦的 `else if`，从上到下像阅读表格一样一目了然。

---

## 本篇小结

在这一篇里，我们对 Flurry 的分支与提前跳转进行了全面升级：

| 构造 | 语法形态 | 是表达式还是语句？ | 核心规则与设计理由 |
| :--- | :--- | :--- | :--- |
| **`if` / `else`** | `if cond { ... } else { ... }` | **表达式** | 各分支类型须统一；无 `else` 时类型为 `void`（若确定退出则为 `never`） |
| **`when`** | `when { c1 => e1, else => e2 }` | **表达式** | 按序求值布尔条件，执行首个为真的臂；臂间用逗号 `,` 分隔 |
| **转移守卫** | `return x while cond;` / `break while cond;` | **语句后置守卫** | 先求值 `cond`，为真才求值并跳转；用 `while` 而非 `if` 以彻底消除与前缀 `if` 表达式的歧义 |

### 下一篇预告

分支表达式让我们告别了混乱的 `if-else`。但是，我们的传感器并不是孤零零的一个——在电路板上，排布着一个 **$4 \times 4$ 的二维传感器阵列网格（Sensor Grid）**，同时主控芯片还运行着一个永不停止的**后台守护主循环（Watchdog Loop）**。

当我们开始写循环时，三个新的问题跳了出来：
1. 既然 `if` 和 `when` 都是表达式，为什么 Flurry 规定 **`for` 和 `while` 是语句而不是表达式**？为什么 `break` 后面不允许像 Rust 那样带返回值？
2. 当我们在双层循环 `for:row` 和 `for:col` 里扫描二维传感器网格时，如果想直接跳出外层循环，标签该怎么写？如果循环内部不小心声明了一个和标签同名的局部变量 `let row = 1;`，写 `break row;` 会发生什么？
3. 在写主控守护循环 `fn wait_ready() -> bool { while true { if check() { return true; } } }` 时，为什么我们不需要在 `while true { ... }` 的大括号后面再写一行机械的 `return false;` 来哄编译器开心？

在下一篇 **《Flurry 演进手记 (008)：跳出深层嵌套迷宫——带标签的 `for:label`、`while:label` 与 `never` 类型推导》** 中，我们将深入 Flurry 的循环控制、统一命名空间与出口可达性分析。
