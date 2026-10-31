# Flurry 演进手记 (011)：当 NaN 闯入浮点数匹配——区间覆盖证明与纯守卫的穷尽性分析

在 [上一篇 (010)](./010-pattern-precedence-grouping-and-ranges.md) 的结尾，我们遇到了一个让人后背发凉的场景：

传感器的整数读数经过校准公式计算后，变成了 64 位浮点数 `f64`（在后文中我们也用 `typealias Float = f64;` 表示）。算法团队要求我们写两个核心函数：
1. **归一化区间检测**：判断校准后的读数是否落在单位半开区间 `0.0..1.0`（即大于等于 `0.0` 且严格小于 `1.0`）内。
2. **全量正负分类器**：把所有可能的 `Float` 读数分成“负数”、“非负数”以及“非法噪声（`NaN`）”，而且**最好切勿采用粗暴的 `_` 通配符掩耳盗铃**，以便让编译器帮我们证明每一个分支都严丝合缝。

听起来像是小学数学里的数轴划分？
可一旦你把整数范围模式的直觉搬到 IEEE 754 浮点数（`f32` / `f64`）上，那个被称为 **`NaN`（Not-a-Number）** 的幽灵，马上就会给你上一堂刻骨铭心的系统编程课。

---

## 一、 把浮点数当成实数数轴——为什么 `..0.0` 和 `0.0..` 漏网了？

如果我们把 `Float` 当成高中数学里连续的实数轴 $\mathbb{R}$，最容易写出来的 传统分类器长这样：

```flurry
use std.prelude.*;

typealias Float = f64;

-- ❌ 编译期直接报错：（Non-exhaustive match：未穷尽所有可能的 Float 值。）
fn classify_float(value: Float) -> str {
    value match {
        ..0.0 => "negative",
        0.0.. => "non-negative",
    }
}
```

当你满怀信心地按下编译键时，Flurry 的穷尽性检查器（Exhaustiveness Checker）立刻亮起了红灯：**这个 `match` 没有覆盖全部 `Float` 值。**

你一拍脑袋：“啊。我忘了浮点数里还有一个 `Float.NaN`（比如 `0.0 / 0.0` 产生的无效值）。那我补上一行 `Float.NaN => "nan"` 不就行了吗？”
于是你写下了第二个尝试：

```flurry
-- ❌ 编译期依然直接报错。
fn classify_float_v0_attempt2(value: Float) -> str {
    value match {
        ..0.0 => "negative",
        0.0.. => "non-negative",
        -- 妄图用 NaN 常量做相等匹配来补齐缺口：
        Float.NaN => "nan",
    }
}
```

结果——**编译器再一次直接报错。**

为什么连加上 `Float.NaN` 都无法穷尽？
下面来看看 IEEE 754 浮点数在比较运算下到底是怎么运作的：

```text
  ══════════════════════ Float (f64) 的真实全域地图 ══════════════════════

     有序负无穷与负数             零点               正数与正无穷          无序幽灵岛
  ◄──────────────────────┼──────────────────┼───────────────────────►   ┌─────────┐
  -inf ...     -0.0001   │  -0.0  ==  +0.0  │   +0.0001    ...   +inf   │   NaN   │
  └──────────┬───────────┘└────────┬────────┘└───────────┬──────────┘   └────┬────┘
             │                     └──────────┬──────────┘                   │
       匹配 `..0.0`                     匹配 `0.0..`                   任何有序比较都为 false。
    (即 value < 0.0)                 (即 0.0 <= value)                 - value < 0.0   ──► false
                                                                       - 0.0 <= value  ──► false
                                                                       - value == NaN  ──► false (!!)
```

看清楚右下角那座孤零零的“无序幽灵岛”了吗？
1. **为什么 `..0.0` 和 `0.0..` 抓不住 `NaN`？**
   在 Flurry 中，浮点数的范围模式使用与普通比较表达式完全一致的**有序比较（Ordered Comparison）**：
   - `..0.0` 检查的是 `value < 0.0`；
   - `0.0..` 检查的是 `0.0 <= value`。
   当 `value` 是 `NaN` 时，它与任何数的 `<`、`<=`、`>`、`>=` 比较结果**统统是 `false`**。所以 `NaN` 既不进第一支，也不进第二支。
2. **为什么 `Float.NaN => "nan"` 也抓不住 `NaN`？**
   因为字面量/常量模式（如 `Float.NaN`）在底层定义就是“与该值做相等比较 `value == Float.NaN`”。而 IEEE 754 最著名的一条核心规则就是：**`NaN` 有别于任何值，甚至有别于它自己（`NaN != NaN`）。** 因此，`Float.NaN` 这个相等模式永远是 `false`，根本补不上 `NaN` 的缺口。

---

## 二、 精确的浮点区间语义——为什么右开上界禁写成 `not (upper <= value)`？

既然搞清了有序比较的脾气，我们先来实现第一个需求：判断读数是否落在单位半开区间 `0.0..1.0` 内。

在 Flurry 中（详见 `docs/control-flow/pattern-matching/index.md`）：
- **半开范围 `lower..upper`**：严格要求 **`lower <= value and value < upper`**；
- **闭范围 `lower..=upper`**：严格要求 **`lower <= value and value <= upper`**；
- **正负零相等**：按比较规则，`-0.0 == 0.0` 为真，因此 `-0.0` 也会满足 `0.0 <= value`。
- **无穷值受开闭边界约束**：比如 `0.0..Float.Infinity`（右开）排除正无穷，而 `0.0..=Float.Infinity`（右闭）包含正无穷。

来看看 `in_unit_interval` 的标准写法：

```flurry
use std.prelude.*;

typealias Float = f64;

fn in_unit_interval(value: Float) -> bool {
    value match {
        0.0..1.0 => true,
        _ => false,
    }
}

test {
    -- 0.0 和 -0.0 都匹配成功。
    assumes in_unit_interval(0.0);
    assumes in_unit_interval(-0.0);
    -- 右开边界 1.0 和 NaN 都失配，走 `_ => false`。
    assumes not in_unit_interval(1.0);
    assumes not in_unit_interval(Float.NaN);
}
```

> **💡 编译器设计深水区：为什么右开上界 `value < 1.0` 无法优化成 `not (1.0 <= value)`？**
> 在整数的世界里，“小于 10（`x < 10`）”和“小于或等于等于 10（`not (10 <= x)`）”是 100% 等价的。
> 但在浮点数的世界里，如果某个糊涂的编译器把 `0.0..1.0` 的上界检查实现成了 `not (1.0 <= value)`，会发生什么？
> 假设有人写了一个单侧右开范围 `..1.0`，并传入了 `value = Float.NaN`：
> - 正确的 `value < 1.0`：对于 `NaN` 返回 **`false`**（失配）。
> - 错误的 `not (1.0 <= value)`：因为 `1.0 <= NaN` 是 `false`，取反后变成了 **`true`**（把 `NaN` 当成了小于 `1.0` 的数）。
> 因此，Flurry 明确规定：右开上界必须直接检查 `value < upper`，不与 `not (upper <= value)` 混同。

### 那如果我在模式外面套一层 `not` 呢？——范围与 `not` 的完备覆盖证明

仔细体会刚才那句话：
- **`0.0..1.0` 本身**要求有序比较 `0.0 <= value and value < 1.0`，所以它**失配 `NaN`**。
- 那如果我们写 **`not (0.0..1.0)`** 呢？
  在 Flurry 中，前缀模式 `not P` 的定义是**反转模式 `P` 的完整匹配结果**。既然 `NaN` 让 `0.0..1.0` 匹配失败（`false`），那么反转之后，**`not (0.0..1.0)` 就会稳稳地接纳 `NaN`。**

更厉害的是：Flurry 的静态穷尽性分析器在内部专门**保留了 `NaN` 这一独立类别**。
这意味着，如果你无须 `_`，而是把 `0.0..1.0` 和它的取反 `not (0.0..1.0)` 放在同一个 `match` 里：

```flurry
-- ✅ 编译期 100% 静态证明穷尽。（无需写 `_` 分支。）
fn partition_unit_interval(value: Float) -> str {
    value match {
        0.0..1.0 => "inside [0.0, 1.0)",
        not (0.0..1.0) => "outside or NaN",
    }
}
```

让我们画图看看编译器的穷尽性求解器是怎么算账的：

```text
  主语 value : Float 的全集 = { 负无穷..0.0,  [0.0..1.0),  1.0..正无穷,  NaN }
                                      │            │            │         │
  第 1 臂: `0.0..1.0`       ──────────┼────────────✓────────────┼─────────┼──► 划掉 [0.0..1.0)
                                      │                         │         │
  剩余未覆盖集合             = { 负无穷..0.0,               1.0..正无穷,  NaN }
                                      │                         │         │
  第 2 臂: `not (0.0..1.0)` ──────────✓─────────────────────────✓─────────✓──► 全部划掉。
                                                                               (剩余为空集 ∅，穷尽通过。)
```

---

## 三、 不靠 `_` 抓出 `NaN`——纯比较守卫 `number == number` 的神来之笔

现在，让我们回到开头最棘手的挑战：
如果我不想把 `NaN` 和区间外的正常数字混在同一个 `not (...)` 或 `_` 里，而是想**清清楚楚地把“有序浮点数（Ordered）”和“无序的 `NaN`”拆成两个分支**，且弃用兜底的 `_`，在 Flurry 里做得到吗？

答案是：**用纯比较守卫（Pure Comparison Guard）。**

既然 IEEE 754 规定：
- 任何有序的浮点数（包括正负零、正负无穷）都满足自反律 **`number == number`**；
- 唯独 `NaN` 满足 **`number != number`**。

那么，如果我们在模式守卫里直接写 `if number == number` 和 `if number != number`，Flurry 的穷尽性分析器能看懂这个数学性质吗？

来看下面的实现代码：

```flurry
use std.prelude.*;

typealias Float = f64;

-- ✅ 编译期直接证明穷尽。无需 `_` 兜底。
fn is_ordered(value: Float) -> bool {
    value match {
        _ as number if number == number => true,
        _ as number if number != number => false,
    }
}

-- 甚至可以把区间划分与 NaN 守卫组合起来，实现 100% 无 `_` 的四路精确分类。
fn classify_sensor_reading(value: Float) -> str {
    value match {
        ..0.0 => "negative",
        0.0..1.0 => "unit_interval",
        1.0.. => "large_positive",
        _ as number if number != number => "sensor_nan_fault",
    }
}

test {
    assumes is_ordered(0.0);
    assumes is_ordered(-3.14);
    assumes not is_ordered(Float.NaN);

    println("{}", classify_sensor_reading(-0.5));
    println("{}", classify_sensor_reading(0.5));
    println("{}", classify_sensor_reading(2.5));
    println("{}", classify_sensor_reading(Float.NaN));
}
```

看看 `classify_sensor_reading` 的四个分支：
1. `..0.0` 覆盖了所有有序负数 `(-inf, 0.0)`；
2. `0.0..1.0` 覆盖了 `[0.0, 1.0)`（包括 `-0.0` 和 `+0.0`）；
3. `1.0..` 覆盖了 `[1.0, +inf]`；
4. 此时整个 `Float` 域里**唯一**还没被覆盖的漏网之鱼，就只剩下 `NaN` 类别。
5. 第 4 臂 `_ as number if number != number` 登场——穷尽性分析器静态推导出 `number != number` 恰好精确命中 `NaN` 类别，四臂合璧，剩余未覆盖空间归零，**编译期直接放行**。

### 局部布尔 `let` 证据与静态分析的诚实边界

更酷的是，即使你在守卫里用局部布尔 `let` 变量把比较条件拆开写，穷尽性分析器依然能顺藤摸瓜保留真值证据：

```flurry
fn is_ordered_with_local_let(value: Float) -> bool {
    value match {
        _ as number if do {
            let ok = number == number;
            ok
        } => true,
        _ as number if do {
            let ok = number == number;
            not ok
        } => false,
    }
}
```

但是，Flurry 的编译器有着严格的**诚实边界（Honesty Boundary）**：
- **不在穷尽性分析中运行任意用户代码**：穷尽性分析只分析常量端点、纯比较（`<`, `<=`, `>`, `>=`, `==`, `!=`）及其布尔组合（包括局部布尔 `let`、完整的 `if-else` 和嵌套布尔 `match`）。
- **未知函数调用无法伪装成覆盖证据**：如果你写了一个黑盒函数 `fn mysterious_check(x: Float) -> bool`，哪怕它在运行时碰巧对所有数都返回 `true`，编译器亦免于相信它能穷尽模式。

```flurry
fn mysterious_check(x: Float) -> bool { true }

-- ❌ 编译期直接报错。
fn bad_exhaustive_claim(value: Float) -> bool {
    value match {
        -- 哪怕先存进局部变量，未知函数调用的结果亦无法当作静态覆盖证明。
        _ as x if do {
            let flag = mysterious_check(x);
            flag
        } => true,
        -- 哪怕你以为上一行已经返回 true 了，没有穷尽证据且没有 `_` 时，编译器照样报错。
    }
}
```

同时，切勿试图用 `x < 0.0 or x >= 0.0` 这种条件去短路跳过 `NaN` 路径上的未知函数调用——因为编译器一清二楚地记得：**当 `x` 是 `NaN` 时，`x < 0.0` 和 `x >= 0.0` 全是 `false`。**

---

## 本篇小结

通过上述步骤，我们驯服了浮点数模式匹配中最狡猾的 `NaN`：

| 版本 | 尝试的写法 | 暴露的陷阱 / 揭示的机制 |
| :--- | :--- | :--- |
| **初版** | 仅写 `..0.0` 与 `0.0..`，或加 `Float.NaN` 分支 | `NaN` 使所有有序比较返回 `false`；且 `NaN != NaN` 导致常量相等模式永远无法匹配 `NaN` |
| **改进版** | `0.0..1.0` 与 `not (0.0..1.0)` | 半开区间严格检查 `lower <= v and v < upper`（`-0.0` 与 `0.0` 均匹配）；`not` 反转完整匹配结果从而接纳 `NaN`，编译器可静态证明 `R` 与 `not R` 穷尽全域 |
| **进阶版** | `..0.0` + `0.0..` + `_ as n if n != n` | 穷尽性分析器保留 `NaN` 类别与纯比较/局部布尔 `let` 的真值证据，无需执行用户代码即可静态证明 `n == n` 与 `n != n`（或全区间 + `n != n`）完备覆盖 `Float` |

---

### 下一篇预告

现在，我们的传感器数据既能用位向量拆解，又能用整数和浮点数区间滴水不漏地分类了。
接下来，我们要把校准后的传感器参数组装进一个配置结构体 `Window` 或 `SensorConfig` 里，例如：

```flurry
struct Window {
    width: u32,
    height: u32 = default_height(),
}
```

这时，一位写惯了 C++ 和 Rust 的同事走过来，写出了一行结构体构造代码：

```flurry
let win = Window {
    height: read_register_a(),
    width:  read_register_b(),
}
```

他问了你三个关键问题：
1. “我故意在花括号里**先写了 `height`，后写了 `width`**。那硬件寄存器到底是先读 A 还是先读 B？”
2. “既然 `height` 在定义 `struct Window` 时有默认值 `= default_height()`，当我显式传入 `height: read_register_a()` 时，`default_height()` 是否会在后台偷偷执行一次？”
3. “如果 Flurry 规定了固定的字段初始化顺序，那是否意味着结构体在内存里的物理字节排列顺序也被锁死了，没法像 Rust 那样自动重排字段来消除内存对齐空洞（Padding）了？”

在下一篇 **《Flurry 演进手记 (012)：字段顺序有别于内存顺序——`struct` 声明序求值、默认字段与物理布局解耦》** 中，我们将揭开 Flurry 结构体设计中最精妙的“时间与空间解耦术”。
