# Flurry 演进手记 (008)：跳出深层嵌套迷宫——带标签的 for:label、while:label 与 never 类型推导

在 [上一篇 (007)](./007-branching-with-if-when-and-while-guards.md) 中，我们用表达式化的 `if`/`when` 和后置 `while` 守卫把分支逻辑打扫得干干净净。

今天，硬件团队把单个传感器升级成了一块 **$4 \times 4$ 的二维红外传感器阵列（Sensor Grid）**。我们需要编写两个核心循环函数：
1. **网格扫描器**：按行（`row`）和列（`col`）遍历二维阵列。如果遇到读数为负数的校准盲区就跳过当前单元格；如果整行第一个单元格就是睡眠标志（`-999`），就直接跳到**下一行**；一旦找到第一个超过告警阈值的过热点，就立刻停止全部扫描并记录坐标。
2. **硬件就绪守护循环**：在系统启动时，不断轮询硬件状态寄存器，直到硬件就绪并返回 `true`。

如果你写惯了 C、Java 或者 Rust，这两个需求看似简单，却常常逼出两类繁琐的代码：
- 为了从内层循环跳出外层循环，严禁不定义一堆 `var found = false; var skip_row = false;` 布尔标志位，在每层循环末尾写 `if found { break; }`；
- 在写 `while true { if ready() { return true; } }` 时，明明知道循环难以正常走完，某些编译器却非逼着你在循环大括号后面再写一行永远执行不到的 `return false;`（死代码）。

让我们从使用布尔标志位的传统写法开始，看看 Flurry 是如何用**统一命名空间的循环标签**与**保守精确的 `never` 出口分析**解决这两个问题的。

---

## 一、 繁琐的布尔标志位与讨好编译器的“假返回值”

来看看没有用上循环标签、也不信任编译器出口分析时写出来的 初始代码：

```flurry
use std.prelude.*;

struct Grid4x4 {
    -- 用一维或简单计算模拟读取 (r, c) 处的传感器值
    fn get(*self, r: i32, c: i32) -> i32 {
        (r * 10i32) + c
    }
}

fn scan_hotspot_initial(grid: *Grid4x4, limit: i32) -> ?Tuple<i32, i32> {
    var result: ?Tuple<i32, i32> = null;
    var stop_all = false;

    var r = 0i32;
    while r < 4i32 {
        var c = 0i32;
        while c < 4i32 {
            let val = grid.get(r, c);
            if val == -999i32 {
                -- 想直接跳到下一行 r + 1，却只能 break 内层循环。
                break;
            }
            if val >= limit {
                result = (r, c);
                stop_all = true; -- 设标志位，先跳内层，再去跳外层。
                break;
            }
            c += 1i32;
        }
        if stop_all {
            break;
        }
        r += 1i32;
    }
    result
}

fn wait_hardware_ready_initial(port: *bool) -> bool {
    while true {
        if port.* {
            return true;
        }
    }
    -- 尴尬：担心编译器以为 while 循环会正常结束，硬着头皮补一行永远走不到的假返回值。
    false
}
```

看看 `scan_hotspot_initial` 里那个 `var stop_all = false;`：每次在内层循环做了决定，都要像打电报一样把标志位传给外层循环再判断一次。万一有三层循环，你就得维护两三个标志位，稍微漏写一个 `if stop_all { break; }` 就是严重的逻辑 Bug。

为什么我们不直接给循环贴个名字（标签）呢？在贴标签之前，我们先弄清楚 Flurry 循环的一个基本设计哲学。

---

## 二、 为什么循环是语句？怎么给 `for:label` 和 `while:label` 贴标签？

### 1. 为什么 `if`/`when` 是表达式，而 `for`/`while` 却是语句？

在上一篇里，我们刚夸赞过 `if` 和 `when` 作为表达式有多么好用。那为什么到了 `for` 和 `while`，Flurry 却明确规定（详见 `docs/control-flow/basic.md`）：
- **`for` 和 `while` 是语句（Statements），不是表达式。**
- **`break` 和 `continue` 不携带返回值。**
- **无限循环直接写 `while true { ... }`，不额外发明 `loop` 关键字。**

为什么不像 Rust 那样引入专门的 `loop { break 42; }` 让循环也返回值呢？
原因在于**控制流职责的正交与纯粹**：
1. 在 `if` 或 `when` 中，每一条分支都是为了计算并汇合出一个结果值；
2. 而循环的本质是**状态迭代与重复执行**。如果允许 `while cond { break 42; }` 返回值，那当 `cond` 一开始就是 `false`、循环体一次都没跑的时候，整个 `while` 表达式该返回什么值？你严禁不为此再去发明 `while ... else` 或者强迫返回 `?T`，反而把简单的循环搞得复杂晦涩。
3. 在 Flurry 中，如果你需要在找到目标时直接把结果交出去，要么直接用函数级的 **`return (r, c) while val >= limit;`**，要么更新明确声明在循环外的状态变量；而无限循环就是字面意义上的 **`while true`**，无需多背一个 `loop` 关键字。

### 2. 循环标签 `for:label` 与 `while:label`

为了让你在多层嵌套循环中指哪打哪，Flurry 允许在 `for` 和 `while` 后面紧跟一个 `:id` 形式的**标签（Label）**：

```text
  语法形态：
  while :outer cond { ... }     (冒号两侧可紧贴写 `while:outer` 或留空 `while :outer`)
  for   :row   r in 0..4 { ... }

  跳转形态（同样支持后置 `while` 守卫。）：
  break outer while should_abort;     ──► 直接跳出名为 `outer` 的外层循环。
  continue row while is_row_asleep;   ──► 直接进入名为 `row` 的外层循环的下一轮。
```

下面来看看标签是如何为嵌套循环架设“直达电梯”的：

```text
   ┌──► while:row r < 4i32 {
   │        while:col c < 4i32 {
   │            continue col while val < 0i32;    ──┐ (跳到内层 col 下一轮)
   │  ┌──────── continue row while val == -999i32;  │
   │  │  ┌───── break row while val >= limit;       │
   │  │  │      ...                               ◄─┘
   │  │  │  }
   └──┼──┘  r += 1i32; ◄── (注意：在 while 中 continue 会跳回条件检查；
      │                     在 for:row 中 continue row 则自动推进到下一个元素。)
      ▼  }
   [直接跳出整个双层循环，到达循环外部。]
```

### 3. 统一命名空间核心规则：当标签撞上同名局部变量会发生什么？

现在，思考一个微妙的安全问题：
在 C、Rust 或 Java 中，循环标签（如 Rust 的 `'label` 或 Java 的 `label:`）往往存放在一个**独立的标签命名空间**里。这意味着就算你在循环体内部声明了一个叫 `let retry = 1;` 的变量，外层的标签 `retry` 依然活在平行世界里。

但在 Flurry 中，为了避免“不同名称类别之间的隐式回退查找与视觉欺骗”，语言采用了严格的**统一命名空间（Unified Namespace）**（详见 `docs/code-organization/scope-and-paths.md`）：

> **类型、值、模块与循环标签（Label）共用同一个命名空间。内层作用域的同名局部绑定可以遮蔽（Shadow）外层的标签；一旦 `break name` 查找到的内层绑定并非一个合法的循环标签，编译器会当场静态报错，不偷偷跳过该变量去外层找同名标签。**

来看看这段被编译器静态拦截的危险代码：

```flurry
-- ❌ 编译期直接报错。
fn shadowed_label_bug() {
    while:retry true {
        -- 在循环内部声明了一个叫 `retry` 的普通 u32 变量，遮蔽了外层的 `:retry` 标签。
        let retry: u32 = 1u32;

        -- 此时从最内层作用域向外查找 `retry`，命中的是上面的 `u32` 变量。
        -- 编译器发现 `retry` 是数值绑定而非循环标签，立即报错，
        -- 不会隐式地跳过 `let retry` 去跳外层的循环。
        break retry;
    }
}
```

为什么要这么严格？
想象一下在一个长达 80 行的循环里，外层有个标签叫 `:target`，结果维护代码的同事在第 50 行加了一个局部变量 `let target = ...;`。如果编译器允许在不同的命名空间里各找各的，读代码的人看到 `target` 时容易产生严重混淆。统一命名空间遵循最简单的两步法：**第一步只按词法作用域由内向外锁定唯一声明身份；第二步再验证这个身份能否用在这里**——不因为第二步验证不过就倒退回第一步换个人选。

---

## 三、 没有正常出口的循环与 `never` 类型推导

现在来看第二个需求：等待硬件就绪的主控守护循环 `wait_hardware_ready`。

在上面的初始写法中，我们在 `while true { ... }` 后面被迫补了一行 `false`，因为担心编译器抱怨“函数声明了返回 `bool`，但最后没有返回值”。

在 Flurry 中，**这行多余的 `false` 全然无须写。**

为什么？因为根据 `docs/control-flow/basic.md`，当一个循环**没有可达的正常出口（No Reachable Normal Exit）**时，编译器会将其内部完成类型推导为底类型 **`never`**。

下面来看看编译器的**出口可达性分析器（Exit Reachability Analyzer）**是怎么工作的：

```text
  函数 1：纯粹的死循环
  fn spin_forever() -> bool {
      while true {}   ──► 条件恒为 true，内部没有任何 `break`
  }                   ──► 正常出口不可达。循环后方为不可达区域 (never)，无需补 bool。

  函数 2：只通过 `return` 离开的守护循环
  fn wait_hardware_ready(port: *bool) -> bool {
      while true {
          return true while port.*;  ──► 唯一离开路径是直接从函数 `return true`。
      }                              ──► 没有任何 `break` 能跳到循环大括号外面。
  }                                  ──► 循环正常出口为 never，大括号后无需补假返回值。
```

非但如此，Flurry 编译器的出口分析还能穿透布尔逻辑组合（`not` / `and` / `or`）以及**已知常量构造值的 `match`**。
比如在编写硬件仿真测试时，如果我们构造了一个已知的信号枚举去跑状态循环：

```flurry
enum Signal {
    idle,
    ready(Tuple<bool, bool>),
}

fn simulate_watchdog() -> bool {
    while true {
        -- 主语是已知的常量构造值 Signal.ready((true, false))
        Signal.ready((true, false)) match {
            -- 编译器按分支顺序静态判定：每次必然命中第一个分支并 `continue`。
            -- 下面的 `_ => break` 分支永无可能被选中。
            .ready((true, false)) => continue,
            _ => break,
        }
    }
    -- 编译器确知 `_ => break` 不可达，整个 while true 依然没有正常出口 (never)，
    -- 因此函数末尾无需补任何 bool 返回值。
}
```

当然，编译器在这件事上保持着严格的**诚实与保守**（详见 `docs/control-flow/basic.md`）：
- 如果 `break` 受一个**运行时未知条件**控制（比如 `break while port.*;`），编译器就会认为循环有可能正常结束，此时如果你不在循环后面提供返回值，编译器就会报错：

```flurry
-- ❌ 编译期直接报错。
fn might_break_without_return(port: *bool) -> bool {
    while true {
        -- `port.*` 是运行时未知的。如果它为 true，break 就会跳出循环到达函数末尾。
        break while port.*;
    }
    -- 错误：循环存在可达的正常出口，但函数末尾缺少 bool 返回值。
}
```

- 同样地，在嵌套循环中，编译器会按 `break` 的**目标标签**精确记账：内层循环里的 `break;` 只给内层循环提供正常出口，免给外层的 `while:outer true` 提供出口；反之，内层循环里的 `break outer;` 则会直接记作外层 `outer` 循环的可达出口。

现在，让我们用 `for:row` / `for:col` 标签、`while` 守卫与 `never` 类型推导，写出简洁清晰的 进阶版本：

```flurry
use std.prelude.*;

struct Grid4x4 {
    fn get(*self, r: i32, c: i32) -> i32 {
        (r * 10i32) + c
    }
}

-- 使用带标签的循环与转移守卫，告别 `stop_all` 布尔标志位。
fn scan_hotspot(grid: *Grid4x4, limit: i32) -> ?Tuple<i32, i32> {
    var r = 0i32;
    while:row r < 4i32 {
        var c = 0i32;
        while:col c < 4i32 {
            let val = grid.get(r, c);

            -- 1. 遇到睡眠行标志 (-999)，直接跳出当前列循环或进入下一行
            break col while val == -999i32;

            -- 2. 遇到负数盲区，跳过当前单元格继续下一列
            if val < 0i32 {
                c += 1i32;
                continue col;
            }

            -- 3. 找到过热点，直接带着坐标从函数返回。
            return (r, c) while val >= limit;

            c += 1i32;
        }
        r += 1i32;
    }
    null
}

-- 守护循环：无需在循环后拼接任何死代码返回值。
fn wait_hardware_ready(port: *bool) -> bool {
    while true {
        true while port.*
    }
}

test {
    let grid = Grid4x4 {}
    let pos = scan_hotspot(grid.ref, 22i32);
    assumes pos is (2i32, 2i32)?;
}
```

---

## 本篇小结

通过重构二维传感器阵列扫描与守护循环，我们掌握了 Flurry 循环控制流的三大支柱：

| 机制 | 语法形态 | 核心语义与安全契约 |
| :--- | :--- | :--- |
| **语句化循环** | `while cond { ... }` / `for pat in expr { ... }` | 循环是语句而非表达式；`break` 未带值；无限循环直接写 `while true` |
| **循环标签与统一命名空间** | `while:label`、`for:label`、`break label`、`continue label` | 标签与普通变量共用同一命名空间；被同名局部绑定遮蔽时直接静态报错，不跨层跳跃查找 |
| **出口可达性与 `never`** | 无可达 `break` 的 `while true` | 静态确认无正常出口时循环完成类型为 `never`，后续无需补假返回值；按目标标签精确追踪嵌套 `break` |

### 下一篇预告

在今天的代码里，我们的 `while` 循环条件还只是简单的布尔判断（如 `r < 4i32` 或 `true`）。
但在真实的传感器固件中，最常见的循环不是数数字，而是**消费一个源源不断的事件流（Event Stream）**：
- 每次调用 `queue.pop()`，它返回的是一个可选类型 **`?SensorEvent`**（队列空了就返回 `null`）；
- 拿到 `event` 后，我们还要检查它是否 `.sample(raw_byte)` 变体；
- 拿到 `raw_byte` 后，还要调用 `decode_packet(raw_byte)` 检查是否成功值 `packet!`；
- 拿到 `packet` 后，还要检查它是否 `.data { payload }`……

如果我们用普通的 `while true` 加上一层套一层的 `match`，代码就会像楼梯一样一路向右缩进，变成常见的**“右移金字塔（Pyramid of Doom）”**。

在下一篇 **《Flurry 演进手记 (009)：把流式解析写得像呼吸一样自然——`while ... is ... do` 与 `let ... else`》** 中，我们将把这座缩进金字塔完全展平。
