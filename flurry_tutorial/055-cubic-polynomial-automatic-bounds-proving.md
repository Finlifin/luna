# Flurry 演进手记 (055)：当编译器能自动算清下标——三次以内多元多项式的自动算术安全证明

在 [上一篇 (054)](./054-what-pure-fn-really-guarantees.md) 的结尾，我们提出了一个让所有高性能系统程序员热血沸腾的场景：

我们的传感器分析仪需要高频查询一个平铺在一维定长数组 `values: Array<i32, n>` 里的**三维空间传感器校准表（3D Lookup Table）**。
这张三维表的尺寸是 `width` $\times$ `height` $\times$ `depth`（全部是运行时传入的 `usize` 变量），我们要读取坐标 `(x, y, z)` 处的校准值。
按照标准的行主序（Row-Major）平铺公式，它在数组中的一维下标 `offset` 是：

$$\text{offset} = (z \times \text{height} + y) \times \text{width} + x$$

在传统系统语言里，当你写下 `values[(z * height + y) * width + x]` 时，你立刻掉进了一个两难深渊：
- **要么忍受每次循环里的运行时越界与溢出检查**：Rust 或 Zig 的编译器看不懂 `x < width`、`y < height`、`z < depth` 和 `depth * height * width <= n` 之间的非线性乘法关系，只能在最内层热点循环里每次都插入 `if offset >= n { panic!() }`。
- **要么捏着鼻子写 `unsafe`（如 `get_unchecked`）**：一旦某个上游调用者传进来的 `width = 65536, height = 65536, depth = 65536` 在 32 位机器上发生了**乘法溢出回绕**，算出来的 `width * height * depth` 变成了 `0 <= n`，你的 `unsafe` 代码就会直接冲出数组边界，酿成高危内存越界漏洞！

我们能不能**既不写一行 `unsafe`，又不在实际下标读取 `values[offset]` 处留下冗余的分支检查，还能让编译器从数学和机器算术两个层面，100% 自动证明中间每一步乘法和加法都绝不溢出、且最终的 `offset` 严格小于数组容量 `n`**？

今天，我们就来揭秘 Flurry 在默认 `safe` 模式下内置的核武器：**总次数不超过三次（Degree $\le 3$）的多元多项式自动算术证明器**！

---

## 第一版：“先乘出来再比大小”的机器整数溢出陷阱（v0）

很多程序员在写三维数组边界检查时，第一反应（`v0`）是这样写的：
“要证明 `(z * height + y) * width + x < n`，我只要在函数开头检查一下 `x < width`、`y < height`、`z < depth`，然后再检查一下 `width * height * depth <= n` 不就行了吗？”

让我们画出这个直觉写法 `v0` 的致命漏洞：

```flurry
-- ❌ 编译期直接拒绝！
-- 错误：在证明 `width * height` 与 `plane * depth` 不会发生 usize 机器乘法溢出之前，
-- 直接执行 `width * height * depth <= n` 是非法的！
fn read_3d_naive_v0(
    values: Array<i32, n>,
    width: usize,
    height: usize,
    depth: usize,
    x: usize,
    y: usize,
    z: usize,
) -> i32
where n: usize
{
    assumes x < width and y < height and z < depth;
    -- 致命陷阱！如果 width * height 已经超出了 usize 的最大值，
    -- 先执行机器乘法再拿溢出截断后的结果跟 n 比较，根本无法证明原乘法合法！
    assumes width * height * depth <= n;
    let offset = (z * height + y) * width + x;
    values[offset]
}
```

为什么 Flurry 编译器会毫不留情地拒绝 `assumes width * height * depth <= n;`？
请背诵 `docs/resource-management/contracts-and-verification.md`（§默认推导的范围）里的这句警世恒言：

> **“普通机器运算仍须在执行前满足溢出、除零和范围前提。把可能溢出的乘法先执行再检查结果大小，不能证明原乘法合法。”**

让我们画一张 **Code Cartoon**，看看在有限位宽的机器整数（比如假设某个 8 位无符号整数上限是 `255`）里，“先乘再比大小”是怎么被黑客利用的：

```text
  为什么“先算 a * b，再检查结果 <= n”是致命漏洞？（以 8 位整数 0..=255 为例，设 n = 100）：

  黑客传入：width = 16, height = 16 (数学乘积 = 256 > 100，本来严重越界！)
       │
       ▼
  ❌ 错误做法：先执行机器乘法 `width * height`
     16 * 16 = 256 ──(8位无符号溢出回绕!)──► 变成 0！
     再去检查 `0 <= 100` ──► 居然通过了检查 (true)！随后引发灾难性内存越界！

  ───────────────────────────────────────────────────────────────────

  ✅ Flurry 的正确做法：用除法在乘法【发生之前】守门！
     1. 先确保分母非零：`assumes width > 0usize;`
     2. 用除法检查上限：`assumes height <= n / width;`
        (在数学上等价于 `width * height <= n <= MAX`，且 `n / width` 绝不可能溢出！)
     3. 此时编译器获得铁证：`width * height <= n`！
     4. 放心执行机器乘法：`let plane = width * height;`（100% 静态证明零溢出！）
```

看懂这张图，你就掌握了系统级安全算术的第一性原理：**永远用不会溢出的除法 `b <= limit / a`（配合 `a > 0`），去为接下来的乘法 `a * b` 建立不溢出的静态通行证！**

---

## 第二版：三次以内多元多项式（Degree $\le 3$）的自动证明艺术（v1）

一旦我们用除法守门消除了机器乘法溢出的风险，接下来就要面对第二个问题：
我们已经建立了：
1. `width > 0`、`height > 0`、`depth > 0`
2. `x < width`、`y < height`、`z < depth`
3. `plane = width * height <= n` 且 `depth * plane <= n`（即 $depth \times height \times width \le n$）

现在，当我们写下：

```flurry
let row = z * height + y;
let offset = row * width + x;
values[offset]
```

编译器はどうやって在**不需要任何人工定理证明提示**的情况下，自动从上述前提推导出：
1. `z * height + y` 不会溢出，且严格小于 `depth * height`；
2. `row * width + x` 不会溢出，且严格小于 `depth * height * width <= n`；
3. 因此 `offset < n` 恒成立，`values[offset]` 100% 越界安全？！

答案就在 Flurry 的 **`safe` 自动算术证明范围契约**（`docs/resource-management/contracts-and-verification.md`）：

> **`safe` 自动算术限制在总次数不超过三次（$\le 3$）的多元多项式片段，并受展开项数和步骤预算约束。总次数是单项式中指数之和：三因子乘积是三次，两个平方的乘积是四次。**

让我们用一张 **Code Cartoon** 拆解三维寻址公式的“多项式次数账单”：

```text
  三维寻址不等式的单项式总次数（Total Degree）拆解：

  目标不等式：
     (z * height + y) * width + x  <  depth * height * width  <=  n
          │
          ▼ 展开为多元多项式（变量：x, y, z, width, height, depth）：
     [ z * height * width ]  +  [ y * width ]  +  [ x ]   <   [ depth * height * width ]
       └─── 3 个变量相乘 ───┘     └─ 2 个变量 ─┘    └ 1 ┘       └──── 3 个变量相乘 ────┘
          单项式次数 = 3           次数 = 2       次数 = 1          单项式次数 = 3

  ► 整个不等式的最高总次数 = max(3, 2, 1, 3) = 3 （三次多元多项式！）
  ► 刚好落在 Flurry 的 `safe` 自动算术判定预算（Degree <= 3）之内！

  数学推导链（由编译器自动完成）：
  1. 因为 z < depth (即 z <= depth - 1) 且 y < height：
     row = z * height + y <= (depth - 1) * height + y < depth * height
  2. 因为 row < depth * height (即 row <= depth * height - 1) 且 x < width：
     offset = row * width + x <= (depth * height - 1) * width + x < depth * height * width <= n
  3. 结论：offset < n 静态得证！`values[offset]` 零额外越界检查直接放行！
```

为什么 Flurry 把 `safe` 模式的自动证明界限精确划在 **三次（Degree $\le 3$）**？
- 如果像普通编译器那样只支持**一次线性不等式（Degree $\le 1$，如 $i + 1 < n$）**：你连二维图像寻址 `y * width + x`（二次）和三维空间寻址 `(z * height + y) * width + x`（三次）都算不出来！
- 如果无限制地试图在每次编译时自动求解任意高次（四次、五次……）非线性算术：又会导致编译时间爆炸甚至不可判定！
- 将界限定在 **三次以内（$\le 3$）**，正好把一维切片、二维矩阵/图像、三维体素/张量寻址全部囊括进自动化舒适区，同时保证了编译速度与证明结果的可预测性！

让我们写出教科书般严谨、零 `unsafe` 的三维寻址函数 `read_3d`（`v1`）：

```flurry
use std.prelude.*;

-- v1：先用除法证明中间机器乘法不溢出，再由三次多项式自动证明器推导 offset < n！
fn read_3d(
    values: Array<i32, n>,
    width: usize,
    height: usize,
    depth: usize,
    x: usize,
    y: usize,
    z: usize,
) -> i32
where n: usize
{
    -- 1. 确认三维尺寸均为正数，且各坐标位于对应维度内
    assumes width > 0usize and height > 0usize and depth > 0usize;
    assumes x < width and y < height and z < depth;

    -- 2. 先用除法证明 `width * height` 不会超过 n（因此也绝不会发生 usize 乘法溢出）
    assumes height <= n / width;
    let plane = width * height;

    -- 3. 再用除法证明 `depth * plane`（即三次项 depth * height * width）不超过 n
    assumes depth <= n / plane;

    -- 4. 此时三次多项式关系已完备：row 和 offset 的机器算术及 values[offset] 的下标访问全部静态得证！
    let row = z * height + y;
    let offset = row * width + x;
    values[offset]
}

test {
    let lut: Array<i32, 24> = [
        1i32, 2i32, 3i32, 4i32, 5i32, 6i32,
        7i32, 8i32, 9i32, 10i32, 11i32, 12i32,
        13i32, 14i32, 15i32, 16i32, 17i32, 18i32,
        19i32, 20i32, 21i32, 22i32, 23i32, 24i32,
    ];
    -- 查询 4(width) x 3(height) x 2(depth) 网格中坐标 (x=2, y=1, z=1) 的值：
    -- offset = (1 * 3 + 1) * 4 + 2 = 18，对应第 19 个元素 (19i32)
    let val = read_3d(lut, 4usize, 3usize, 2usize, 2usize, 1usize, 1usize);
    println("3D LUT value: {}", val);
}
```

运行输出：

```text
3D LUT value: 19
```

---

## 第三版：当遇到四维/四次多项式（Degree $> 3$）时该怎么办？（v2）

这时候你一定会好奇：
> “如果我的张量或校准表是 **四维（4D）** 的，需要判断 $a \times b \times c \times d \le \text{limit}$（四个变量相乘，总次数是 **四次**，或者两个平方相乘 $a^2 \times b^2$ 也是 **四次**），超出了 `safe` 自动证明的三次预算，难道我就没辙了吗？”

注意规范里的这句话：
> **“该限制针对自动证明，不限制程序本身能计算什么。引入任意多的中间等式也不能绕过分析预算，不能据此宣称所有高次关系都被自动证明。”**

当遇到四次（或更高次）关系时，我们既不需要企图用一堆中间变量去“骗”自动三次求解器，也不需要退化到不安全的 `unsafe`——我们可以把高次容量判定写成一个**分步用除法守门的可执行 `predicate`（程序谓词）**！

来看看 `docs/resource-management/contracts-and-verification.md`（§高次判定可以有安全的执行算法）给出的优雅解法 `four_dimensions_fit`：

```flurry
use std.prelude.*;

-- v2：为四维容量关系 (a * b * c * d <= limit) 编写安全的可执行判定算法！
-- 注意：每一步都先判断是否为 0，再用不会溢出的除法 `> limit / ...` 拦截，
-- 只有在确认不溢出后才执行下一步乘法！
predicate four_dimensions_fit(a: usize, b: usize, c: usize, d: usize, limit: usize) {
    if a == 0usize or b == 0usize or c == 0usize or d == 0usize {
        true
    } else {
        if b > limit / a {
            false
        } else {
            let ab = a * b;
            if c > limit / ab {
                false
            } else {
                let abc = ab * c;
                d <= limit / abc
            }
        }
    }
}

fn check_dimensions(a: usize, b: usize, c: usize, d: usize, limit: usize) {
    assumes four_dimensions_fit(a, b, c, d, limit);
    println("4D dimensions {}x{}x{}x{} safely fit within {}!", a, b, c, d, limit);
}

test {
    check_dimensions(2usize, 3usize, 4usize, 5usize, 120usize);
}
```

运行输出：

```text
4D dimensions 2x3x4x5 safely fit within 120!
```

> **💡 重要安全细节：为什么零维路径 `a == 0 or ...` 返回 `true`，却不能随便乱乘？**
> 仔细看 `four_dimensions_fit` 的第一行：只要 `a, b, c, d` 中有一个是 `0`，数学乘积 $a \times b \times c \times d = 0 \le \text{limit}$ 当然为 `true`。
> 但是，正如规范所警告的：**“零维路径判定为 `true`，也不意味着任意重排的机器连乘都安全；某些顺序会在乘到零之前溢出。”**
> 举个例子，如果 `a = 1000000, b = 1000000, c = 1000000, d = 0`，虽然数学乘积是 `0`，但如果你在机器上按顺序算 `a * b * c * d`，在乘到最后的 `d = 0` 之前，前面的 `a * b * c` 早就发生整数溢出了！而我们在 `else` 分支里一步步通过 `b > limit / a` 和 `c > limit / ab` 建立起来的局部乘法，每一步都有前置条件严格保护，滴水不漏！

---

## 本篇小结

通过本篇的学习，我们摸清了 Flurry 算术安全验证的边界与方法论：

| 场景 / 维度 | 多项式总次数 | Flurry 的验证策略与写法 |
| :--- | :--- | :--- |
| **一维下标 `i + offset < n`** | 一次（Degree $1$） | `safe` 自动证明；需先保证加法不溢出（如 `offset <= n - i`） |
| **二维矩阵 `y * w + x < n`** | 二次（Degree $2$） | `safe` 自动证明；先用 `h <= n / w` 守门证明乘法不溢出 |
| **三维空间 `(z * h + y) * w + x < n`** | 三次（Degree $3$） | `safe` 自动证明的最高上限；分层用除法建立 `plane` 容量后，一步证明 `offset < n` |
| **四维及以上 `a * b * c * d <= n`** | 四次及以上（Degree $\ge 4$） | 超出 `safe` 自动三次预算；编写分步除法守门的 `predicate` 或交由 `verified` 证明 |
| **错误示范：`assumes a * b <= n`** | 任意次 | **`-- ❌ 编译期直接拒绝！`** 严禁先执行可能溢出的机器乘法再拿结果比大小 |

### 留给下一篇的悬念

在这一篇和前几篇的代码里，有一个关键字几乎每篇都在为我们保驾护航——那就是 **`assumes`**！
比如：
- `assumes valid_index(index, n);`
- `assumes height <= n / width;`
- `assumes four_dimensions_fit(a, b, c, d, limit);`

但你有没有深入想过：**`assumes expr;` 在 Flurry 编译器里到底做了什么？**
- 它和 C 语言的 `assert(expr)`、或者 LLVM 的 `__builtin_assume(expr)`（不检查直接盲信，一旦为假直接 Undefined Behavior）有什么本质区别？
- 什么时候 `assumes` 会在编译期被**彻底消除（零运行时开销）**？什么时候它又会自动在危险动作之前变成一条**安全的运行时检查指令**？
- 更重要的是：如果在第 1 行写了 `assumes below_limit(source, 10);`（建立了 `source.* < 10` 的事实），结果在第 2 行有人通过指针把 `source.*` 改成了 `12`——刚才 `assumes` 发布出来的事实会发生什么？

在下一篇 **《Flurry 演进手记 (056)：把动态检查变成静态事实——`assumes` 断言、事实版本失效与分支精化》** 中，我们将彻底解剖 `assumes` 从动态检查到静态事实发布、再到状态版本失效的全生命周期！
