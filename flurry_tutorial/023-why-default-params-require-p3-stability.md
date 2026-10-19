# Flurry 演进手记 (023)：为什么默认参数里不能偷偷打日志——运行时默认值与 P3 稳定观测契约

在 [上一篇 (022)](./022-optional-parameters-and-forward-dependencies.md) 中，我们见识了 Flurry 可选参数 `.id: T = expr` 的威力：它不仅能直接依赖前面的参数（比如 `.count: u32 = total - start`），甚至还能在默认表达式里调用辅助函数！

这就引出了我们在上一篇末尾留下的那个灵魂拷问：
假设团队里有个粗心的同事，在写传感器阈值检查函数时，为了“调试方便”，在默认参数调用的辅助函数 `announce_and_get_limit()` 里偷偷塞了一行 `println("selecting limit")`，甚至顺手把传进来的计数器指针加了 1，或者写了一个忘了终止条件的递归。

结果，当别人看到一行无比清爽的调用：

```flurry
let x = check_sensor(reading);
```

谁能想到，仅仅因为省略了一个可选参数，这行代码竟然在背后偷偷往串口打了日志、篡改了外部状态、甚至直接卡死在了调用准备阶段？！

反过来，如果我们为了防止这种“隐藏副作用”，一刀切地规定：“默认参数里只能写编译期常量（如 `0`、`true`）！”——那又矫枉过正了！因为在实际工程中，我们经常希望默认参数能读取前序参数的值，或者通过前序传入的配置指针 `source: *Limit` 读取当时的配置快照 `.limit: u32 = source.value.*`，甚至调用一个干净的工厂函数 `make_config(disabled)` 来构造默认配置结构体！

**既要彻底封杀隐藏的 I/O、外部状态篡改与死循环，又要允许灵活读取前序参数、解引用只读指针、以及调用干净的工厂函数**——Flurry 是怎么做到鱼与熊掌兼得的？

答案就是：**运行时默认计算的 `P3`（稳定观测且有限完成）契约**！

---

## 第一版：把副作用藏进默认参数的灾难与编译器的铁腕拦截（v0）

让我们先看看，如果有人试图在默认参数里塞进 I/O 输出或外部指针写入，Flurry 编译器会怎么做。

在 `v0` 中，我们写一个带 `println` 输出的辅助函数 `announce_limit()`，并试图拿它当 `.limit` 的默认值：

```flurry
use std.prelude.*;

fn announce_limit() -> u32 {
    -- 注意：这里产生了外部可观察的 I/O 输出！
    println("selecting limit");
    8u32
}

-- ❌ 编译期直接拒绝！
-- 错误：runtime 默认计算包含外部 I/O 输出，违反了 P3 稳定观测契约！
fn invalid_default(.limit: u32 = announce_limit()) -> u32 {
    limit
}
```

哪怕 `announce_limit` 没有标任何特殊关键字，Flurry 编译器也会顺着完整调用图钻进 `announce_limit` 的函数体，一眼揪出里面的 `println`（属于产生外部事件的 `P0` 一般行为），并在定义 `invalid_default` 时**当场报错拒绝**！

同理，如果你试图在默认表达式里修改传入指针的内容，或者写一个无法证明终止的无限递归，也统统会被无情拦截：

```flurry
-- ❌ 编译期直接拒绝！
-- 错误：默认计算试图通过指针修改入口可观察状态 `counter.*`！
fn invalid_mutating_default(counter: *u32, .limit: u32 = do {
    counter.* += 1u32;
    counter.*
}) -> u32 {
    limit
}

fn infinite_loop() -> u32 {
    infinite_loop()
}

-- ❌ 编译期直接拒绝！
-- 错误：默认计算必须证明在合法入口下有限完成（终止），拒绝未证明有限的递归！
fn invalid_diverging_default(.limit: u32 = infinite_loop()) -> u32 {
    limit
}
```

> **💡 重要细节：显式覆盖也不能替违法的默认定义“洗白”！**
> 你可能会问：“如果我定义了 `invalid_default`，但在所有调用点都显式传了 `.limit = 10u32`，从来没触发过默认表达式，编译器能放过我吗？”
> **绝对不能！** 正如规范所言：**“显式覆盖只跳过该次默认求值，不取消默认定义的合法性要求。”** 一个函数的签名契约必须在定义时就自证清白，绝不允许留着一颗“谁省略参数谁踩雷”的定时炸弹。

### 对比：为什么在调用处显式写 `.limit = announce_limit()` 就完全合法？

这时候你可能会好奇：“如果我真的需要在获取阈值时打印一行日志，该怎么写？”
答案简单得令人感动：**把它写在调用处的显式实参里！**

```flurry
use std.prelude.*;

fn announce_limit() -> u32 {
    println("selecting limit");
    8u32
}

-- 定义处使用干净的默认值 8u32（满足 P3）
fn configured(.limit: u32 = 8u32) -> u32 {
    limit
}

test {
    -- ✅ 完全合法！
    -- 这里的 announce_limit() 是调用者自己明明白白写在调用处的显式实参！
    println("{}", configured(.limit = announce_limit()));
}
```

让我们画一张 **Code Cartoon**，看清为什么同样一个 `announce_limit()`，写在定义处默认参数里是 `❌`，写在调用处显式实参里却是 `✅`：

```text
  ❌ 藏在定义处的默认参数里：fn invalid_default(.limit: u32 = announce_limit())
     调用点写的是：invalid_default()
     调用者视角："我只是调了个普通函数，怎么莫名其妙吐出了日志、改了全局状态？！"
     ──► 属于【隐式缺省供应】，必须受 P3（稳定观测 + 有限完成）铁律约束！

  ✅ 写在调用处的显式实参里：configured(.limit = announce_limit())
     调用点写的是：configured(.limit = announce_limit())
     调用者视角："announce_limit() 是我亲手在调用处写的显式实参，我当然知道它会打日志！"
     ──► 属于【显式实参求值】，跳过默认表达式，按普通实参规则自由求值！
```

这就是 Flurry 的设计哲学：**不替调用者隐藏任何意外的副作用，但绝不限制调用者在显式实参里做合法的操作。**

---

## 第二版：揭秘 `P3` 稳定观测——允许读取指针当前状态，且绝不乱缓存！（v1）

刚才我们一直提到 **`P3`（稳定观测，Stable Observation）**。它到底是什么意思？为什么 Flurry 不干脆要求默认参数必须是编译期常量，或者不许读任何可变内存呢？

让我们先快速看一眼 Flurry 的**运行时纯度六层阶梯（`P0` – `P5`）**（我们将在第三卷第 053 篇做全景展开，这里只需看懂 `P3` 所处的位置）：

```text
  Flurry 运行时纯度金字塔（由下向上保证递增）：

        ▲  [ P5：总值纯 (Total Value Pure) ]   ── P4 + 保证在合法入口下有限完成（终止）
        │  [ P4：值纯 (Value Pure) ]           ── 结果只由显式输入的值语义与常量决定（不读外部可变内存）
   ★ ──►│  [ P3：稳定观测 (Stable Observation) ]── 只读观测 + 在当前复用区间内状态版本稳定！
        │                                         （默认参数要求：P3 + 有限完成！）
        │  [ P2：观测只读 (Read-Only) ]        ── 不修改外部可观察状态、不交付新可观察资源身份
        │  [ P1：有界状态变更 (Bounded Mut) ]  ── 允许受契约约束的局部/指定区域读写，无外界事件
        │  [ P0：一般行为 (General) ]          ── 包含 I/O (如 println)、系统调用、未知回调等
```

看明白为什么默认参数选在 **`P3` + 有限完成**，而不是死板的 `P4` 了吗？

因为如果要求 `P4`（值纯），你的默认参数就**不能通过指针读取当前配置对象的内容**了！
而在真实系统编程中，我们经常把传感器的动态上限放在一个结构体 `Limit { value: u32 }` 里，传给函数一个指针 `source: *Limit`，并希望可选参数 `.limit` 在省略时，自动读取 `source` **在本次调用那一刻**的最新值：

```flurry
fn read_limit(source: *Limit, .limit: u32 = source.value.*) -> u32
```

在 `read_limit` 的单次调用准备期间，`source.value.*` 只是读取了一次有效指针的 `Copy` 内容，没有修改任何外部状态，也没有调用未知的外部写入回调——因此在**本次默认求值的区间内**，这次读取是完全稳定且有限完成的，完美符合 **`P3`（稳定观测）**！

更重要的是：如果两次调用 `read_limit(source.ref)` 之间，外部代码修改了 `source.value = 2u32` 呢？
让我们跑一下 `v1` 代码看看：

```flurry
use std.prelude.*;

struct Limit {
    value: u32,
}

-- v1：默认表达式读取前序指针参数 `source.value.*`，完全满足 P3 稳定观测！
fn read_limit(source: *Limit, .limit: u32 = source.value.*) -> u32 {
    limit
}

test {
    let source = Limit { value: 1u32 }

    -- 第一次调用：省略 .limit，读取当时 source.value 的内容 (1)
    let first = read_limit(source.ref);

    -- 在两次调用之间，修改 source.value 的内容！
    source.value = 2u32;

    -- 第二次调用：传入的是同一个指针地址 source.ref，但读取的是最新内容版本 (2)！
    let second = read_limit(source.ref);

    -- 第三次调用：显式供应 .limit = 9，跳过默认读取
    let supplied = read_limit(source.ref, .limit = 9u32);

    println("{} {} {}", first, second, supplied);
}
```

运行输出是：

```text
1 2 9
```

让我们画张图，理解为什么 `first` 是 `1` 而 `second` 是 `2`：

```text
  时间轴 ──►
  [ let source = Limit { value: 1 } ]
         │
         ├──► 第 1 次调用 read_limit(source.ref)
         │    默认求值区间 #1：稳定观测版本 v1 ──► 读出 1
         │
  [ source.value = 2 ]  ◄── (外部状态版本推进到 v2！)
         │
         ├──► 第 2 次调用 read_limit(source.ref)
         │    默认求值区间 #2：稳定观测版本 v2 ──► 读出 2！
         │    (注意：虽然指针地址 source.ref 没变，但绝不会错误地缓存上次的 1！)
         │
         └──► 第 3 次调用 read_limit(source.ref, .limit = 9)
              显式供应 9 ──► 跳过默认求值，直接得 9！
```

这就是 `docs/functions/parameters.md` 与 `docs/resource-management/runtime-purity.md` 反复强调的核心原则：
> **“稳定性只在取得依据的范围内成立。同一个指针在两次调用中的内容可能不同，默认值不能只以指针位值为键永久缓存。”**

---

## 第三版：在默认参数里构造结构体、调用工厂函数与私有资源清理（v2）

现在，我们的传感器通道配置变得更丰富了：不再只是一个裸的 `u32`，而是一个结构体 `Config { enabled: bool }`。
我们希望写一个函数 `select_config(disabled: bool, .config: Config = ...)`：当调用者省略 `.config` 时，根据第一个参数 `disabled` 自动构造出一个默认的 `Config` 实例！

你可能会担心两件事：
1. “构造一个结构体 `Config { enabled: default_enabled(disabled) }`，或者调用一个工厂函数 `make_config(disabled)`，甚至在工厂函数里声明局部变量、使用局部可变变量 `var`、或者创建临时私有对象再清理掉，还能通过 `P3` 检查吗？”
2. “如果 `Config` 不是 `Copy` 类型（即它是仿射拥有型资源 `Once`），默认参数里能构造并交付它吗？”

Flurry 的回答是：**只要默认计算不篡改调用者的外部状态、不消费调用者已有的非 `Copy` 参数、且内部所有局部资源移动和 `Drop` 清理都被证明是有限且稳定的，就完全合法！**

让我们看看编译器是怎么通过**计算边界（Computation Boundary）**与**私有资源验证**来支持这一切的：

```text
  默认表达式的计算边界 B（例如 `.config: Config = make_local_config(disabled)`）：

  外部调用者世界                         默认表达式的私有计算边界 B
  ┌──────────────┐                      ┌─────────────────────────────────────────┐
  │ disabled:    │ ──(只读 Copy 输入)──►│ let config = Config {                   │
  │   bool       │                      │     enabled: default_enabled(disabled)  │
  └──────────────┘                      │ }                                       │
                                        │ -- 允许局部 var、私有临时对象、局部闭包 │
  ❌ 严禁修改外部指针！                 │ -- 若内部触发私有对象的 Drop，该 Drop   │
  ❌ 严禁隐式消费调用者的非 Copy 绑定！ │    也必须是有限且无外部写入的！         │
                                        └────────────────────┬────────────────────┘
                                                             │ (按移动规则交付结果责任)
                                                             ▼
                                                    交付给形参 `config`！
```

来看看 `v2` 的三种进阶写法——直接构造结构体、调用带局部绑定的工厂函数、以及带 `ensures` 后置条件的工厂函数：

```flurry
use std.prelude.*;

struct Config {
    enabled: bool,
}

-- 干净的辅助函数：有限完成且无外部副作用
fn default_enabled(disabled: bool) -> bool {
    not disabled
}

-- 工厂函数：内部先绑定到局部 let 变量，再按移动（Move）规则交付出来
fn make_local_config(disabled: bool) -> Config {
    let config = Config { enabled: default_enabled(disabled) }
    config
}

-- 带后置条件契约的工厂函数
fn checked_config() -> result: Config
where ensures result.enabled
{
    Config { enabled: true }
}

-- 写法 1：默认表达式直接构造带计算字段的结构体
fn select_inline(
    disabled: bool,
    .config: Config = (Config { enabled: default_enabled(disabled) }),
) -> bool {
    config.enabled
}

-- 写法 2：默认表达式调用工厂函数（即使 Config 没有实现 Copy 也完全合法！）
fn select_factory(
    disabled: bool,
    .config: Config = make_local_config(disabled),
) -> bool {
    config.enabled
}

-- 写法 3：默认表达式调用带 ensures 契约的工厂函数
fn select_checked(.config: Config = checked_config()) -> bool {
    config.enabled
}

test {
    println("{} {}", select_inline(false), select_inline(false, .config = Config { enabled: false }));
    println("{}", select_factory(true));
    println("{}", select_checked());
}
```

运行结果输出：

```text
true false
false
true
```

### 警惕伪装者：如果内部临时对象的 `Drop` 偷偷干坏事呢？

假设有人想耍小聪明：“既然编译器允许我在默认表达式里创建私有临时对象并清理掉，那我故意不在函数体里写 `println`，而是把 `println`（或者外部指针修改）藏在那个临时对象的 `Drop` 析构函数里！编译器总抓不到我了吧？”

太小看 Flurry 的静态分析器了！
在 Flurry 的运行时纯度体系中（见 `docs/resource-management/runtime-purity.md` 示例 8）：**任何局部对象退出作用域或后置条件失败时触发的隐式 `Drop` 清理，都完完全全计入该计算边界的行为摘要！**

```flurry
use std.prelude.*;

struct SneakyTrace {
    id: u32,
}

impl Drop for SneakyTrace {
    fn drop(*self) {
        -- 在析构函数里偷偷打印日志（属于 P0 行为）！
        println("sneaky drop {}", self.id.*);
    }
}

fn make_with_sneaky_drop() -> u32 {
    -- 创建了一个局部对象，离开函数时会自动调用 SneakyTrace.drop
    let temp = SneakyTrace { id: 7u32 }
    42u32
}

-- ❌ 编译期直接拒绝！
-- 错误：虽然 make_with_sneaky_drop 正文没有直接写 I/O，
-- 但其退出清理路径选中的 SneakyTrace.drop 包含输出，整体为 P0，不满足默认参数的 P3 要求！
fn bad_default_with_drop(.limit: u32 = make_with_sneaky_drop()) -> u32 {
    limit
}
```

连析构函数里的隐藏副作用都逃不过编译器的法眼！这保证了在 Flurry 中，当你调用任何省略了可选参数的函数时，缺省计算的准备过程永远是**干净、可预测、只读观测且必定有限终止**的。

---

## 本篇小结

通过深挖默认参数背后的行为契约，我们解锁了 Flurry 语言设计中极具特色的一环：

| 场景 / 问题 | Flurry 的处理规则 | 背后的设计哲学 |
| :--- | :--- | :--- |
| **默认参数里打日志 / 改外部状态** | `-- ❌ 编译期直接拒绝！`（违反 `P3` 稳定观测） | 绝不让省略参数成为隐藏副作用的暗门 |
| **调用处显式传 `.x = announce()`** | `✅ 完全合法！`（属于显式实参求值） | 调用者自己在调用处写下的操作一目了然，不受默认契约限制 |
| **默认参数里写死循环 / 无限递归** | `-- ❌ 编译期直接拒绝！`（违反有限完成要求） | 调用准备阶段不能无故挂起或发散 |
| **默认参数通过指针读配置 `p.val.*`** | `✅ 完全合法！`（单次求值内稳定观测，跨调用重新读取最新值） | 比死板的“纯常量”更实用，且绝不以指针地址为键错误缓存旧值 |
| **默认参数调用工厂构造结构体** | `✅ 完全合法！`（私有临时存储与移动不破坏外部纯性） | 支持非 `Copy` 配置的直接构造与交付，但会严格核查所有隐式 `Drop` 路径 |

### 留给下一篇的悬念

到现在为止，不管是普通锚定参数 `total: u32` 还是可选参数 `.start: u32 = 0`，每一个参数名对应的都是**固定数量（一个）**的输入。

但是，在开发传感器分析仪的诊断模块时，我们遇到了一个新需求：
- 有时候一次快照要打包记录 **2 个数据**：`(时间戳: u32, 温度: u8)`；
- 有时候一次快照要打包记录 **3 个不同类型的数据**：`(编号: u32, 激活状态: bool, 标签: str)`；
- 甚至有时候，我们已经事先把 `(2u32, 3u32, 4u32)` 打包成了一个元组变量 `my_shape`，想直接把这个元组整体传给一个接收变参的函数 `volume`！

在 C 语言里，可变参数 `...`（`va_list`）完全没有类型检查，传错一个类型直接段错误；在很多现代语言里，可变参数 `...items` 强迫所有参数必须是同一种类型（变成同质数组切片 `&[T]`）。

如果既想要传任意多个**不同类型（异构）**的位置参数，又想要 100% 的编译期静态类型安全，还能直接用 `.shape = (2, 3, 4)` 整体供应而不会尴尬地多套一层变成 `((2, 3, 4),)`——该怎么做？

在下一篇 **《Flurry 演进手记 (024)：把异构参数打包进元组——位置变参 `...items` 与 `.items = tuple` 完整供应》** 中，我们将揭开 Flurry 位置变参收集与直接供应的奥秘！
