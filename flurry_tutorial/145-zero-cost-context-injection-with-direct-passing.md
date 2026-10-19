# Flurry 演进手记 (145)：不用切栈也能注入配置与上下文——Direct Passing（直接传值效应）的零开销魔法

> **系列导读**：在上一期（144）末尾，我们提出了一个直击系统性能命门的问题：如果我们在微型运行时中只是想向深层调用链注入一些只读配置或环境上下文（例如超时上限 `limit()`、画布宽高 `Settings.width()` 或当前请求追踪 ID），难道每次调用 `limit()#` 都要触发一次 Handler 分支函数调用吗？更进一步，如果提供配置的表达式带有初始化副作用（比如读取一次环境变量并打印日志），我们怎么保证它在安装时只执行一次，而不是每次读取配置都重复执行？今天，我们将解锁 Flurry 专为零开销上下文注入打造的杀手锏——**`^attrs.effects.direct_passing`（安装时求值与直接传值效应）**！

---

## 一、普通效应做配置注入的痛点：版本 `v0` 的“重复求值陷阱”

假设我们在微型运行时里声明了一个获取并发连接上限的配置效应 `limit() -> u32`。
如果我们在 `v0` 中直接把它写成一个普通的 `effect`：

```flurry
-- v0：使用普通 effect 传递配置
effect limit_v0() -> u32;

fn sum_limits_v0() -> #[limit_v0] u32 {
    -- 函数内部两次读取配置 limit_v0
    let first = limit_v0()#;
    let second = limit_v0()#;
    return first + second;
}

test {
    let total = sum_limits_v0()# {
        limit_v0() => {
            -- ⚠️ 痛点暴露：因为普通 effect 是“每次请求时才跳进分支求值”，
            -- 所以这句 "install limit" 会被打印【两次】！
            println("install limit");
            8u32
        },
    };
    println("{}", total);
}
```

看到问题所在了吗？
对于像“获取传感器采样硬件随机数”这样的操作，每次请求都重新执行一次 Handler 分支是对的；
但对于像“**提供本次任务的配置常量、阈值或环境句柄**”这样的场景，我们真正想要的语义是：
1. **安装时求值一次（Evaluate Once on Install）**：在进入 `expr# { ... }` 的那一刻，先把配置值算好存起来；
2. **调用链直接取值（Direct Value Passing）**：主体内部无论调用多少次 `limit()#`，都只是像读取隐式传入的寄存器/栈参数一样直接拿到那个算好的值，**绝不重复执行 Handler 分支，更不需要任何切栈开销**！

---

## 二、迭代演进 `v1`：加上 `^attrs.effects.direct_passing`，安装即就绪！

在 Flurry 中，你只需在无参的 `effect` 声明（或 `effect enum` 中的单个变体）前加上内置属性 **`^attrs.effects.direct_passing`**：

```text
direct_passing_effect -> ^attrs.effects.direct_passing effect id() -> expr
```

让我们把刚才的 `limit` 升级为 `v1`，看看它的执行轨迹发生了什么本质变化：

```flurry
-- 使用 ^attrs.effects.direct_passing 声明安装时求值、直接传值的效应！
^attrs.effects.direct_passing
effect limit() -> u32;

fn sum_limits() -> #[limit] u32 {
    -- 这两次请求不再触发分支函数调用，而是直接读取安装时已就绪的值槽！
    let first = limit()#;
    let second = limit()#;
    return first + second;
}

test {
    let total = sum_limits()# {
        limit() => {
            println("install limit");
            8u32
        },
    };
    println("{}", total);
}
```

现在运行这段测试，控制台只会输出**一行** `install limit`，紧接着输出 `16`！

### Code Cartoon：普通 Effect vs Direct Passing Effect 的执行时间线对比

```text
  [ 普通 Effect: effect limit() -> u32 ]
    安装 # { ... } ──► 开始运行 sum_limits()
                           ├── 第 1 次 limit()# ──► 执行分支 (打印 "install limit") ──► 得 8
                           └── 第 2 次 limit()# ──► 再次执行分支 (又打印一次！)   ──► 得 8

  ──────────────────────────────────────────────────────────────────────────────────────

  [ 直接传值 Effect: ^attrs.effects.direct_passing effect limit() -> u32 ]
    安装 # { ... }
       │
       ├─► (1) 【在主体开始前】在外层环境执行一次提供分支 (打印一次 "install limit")
       ├─► (2) 将结果 8u32 存入本次 occurrence 的已就绪值槽（或直接装入隐式参数寄存器！）
       └─► (3) 开始运行 sum_limits()
                  ├── 第 1 次 limit()# ──► 直接零开销读取 8u32！
                  └── 第 2 次 limit()# ──► 直接零开销读取 8u32！
```

### ⚠️ 核心语义契约：这不是“可有可无的编译器优化提示”！

请特别注意：`^attrs.effects.direct_passing` 是**语言级的行为契约**，而不是一个由编译器看心情决定的优化提示：
1. **即使主体 0 次请求，安装求值依然发生**：只要执行流进入了 `expr# { limit() => { println("init"); 8u32 } }`，哪怕 `expr` 在运行期因为某个条件分支根本没有调用 `limit()#`，安装时的 `"init"` 依然会确定性地在 `expr` 开始前执行一次！
2. **严禁携带任何参数（包括 `catch` 参数）**：既然值是在安装 Handler 的那一刻（主体还没开始跑的时候）就预先算好的，那么这个操作当然**不可能接收来自未来调用点的实参**！如果你试图给 `direct_passing` 效应加参数，编译器会当场拒绝：

```flurry
-- ❌ 编译期直接拒绝！
-- direct_passing 操作在安装时就已求值完毕，不能接收请求点的参数 key（也不能带 catch continuation）！
^attrs.effects.direct_passing
effect lookup(key: u32) -> u32;
```

3. **遵守 `Copy` 与线性资源契约**：`direct_passing` 不会替非 `Copy` 的线性资源变出隐式的克隆能力！如果提供的值是 `u32` 等 `Copy` 类型，或者是指向有效对象的指针（保留可达性依赖 `&R`），主体可以读取任意多次；但如果提供的是只能消费一次的 `Once` 资源，编译器依然会严格执行移动与单次消费检查！

---

## 三、终极演进 `v2`：按需拆解大配置组（`effect enum`）与无损局部覆盖

在传统语言中，如果你用 `Context` 结构体传配置，往往面临两大痛点：
- **痛点 1（大结构体捆绑传染）**：一个 `Settings` 结构体里有 50 个配置项，`twice_width` 函数明明只用到了其中的 `width` 一项，却被迫要求调用者把另外 49 个配置项也全部构造齐全才能调它！
- **痛点 2（局部覆盖污染或深拷贝开销）**：当某个子模块想临时把 `limit` 从 `10` 改成 `3` 时，要么不小心改掉了全局/外层的对象，要么得把整个大结构体深拷贝一份！

让我们看看 Flurry 的 `direct_passing` 配合 `effect enum` 是如何在 `v2` 中完美化解这两大痛点的：

### 1. 细粒度按需依赖：`#Settings.width` 只传 `width`，无需构造整组！

```flurry
effect enum Settings {
    ^attrs.effects.direct_passing
    width() -> u32,
    ^attrs.effects.direct_passing
    height() -> u32,
}

-- 只需声明 #Settings.width，编译器只传递 width 这一个隐式值槽，
-- 调用者根本不需要提供 height！
fn twice_width() -> #Settings.width u32 {
    let w = Settings.width()#;
    return w * 2u32;
}

-- 声明 #Settings 则同时要求 width 和 height 两项
fn area() -> #Settings u32 {
    let w = Settings.width()#;
    let h = Settings.height()#;
    return w * h;
}

test {
    -- ✅ 调用 twice_width 时，只需安装 Settings.width()，无需构造完整的 Settings 记录！
    let doubled = twice_width()# {
        Settings.width() => 6u32,
    };

    let size = area()# {
        Settings.width() => 6u32,
        Settings.height() => 4u32,
    };
    println("{} {}", doubled, size); -- 输出：12 24
}
```

### 2. 词法作用域内的零污染局部覆盖（Local Override）

当我们在外层安装了 `limit() => 10u32`，而在内层某个子表达式上临时安装了 `limit() => 3u32` 时，每个安装实例（Occurrence）都拥有自己独立的值槽：内层覆盖只在内层范围内生效，**绝不会篡改外层的值槽**；离开内层后，后续代码立刻无缝恢复使用外层的 `10u32`！

```flurry
^attrs.effects.direct_passing
effect limit() -> u32;

fn sum_limits() -> #[limit] u32 {
    return limit()# + limit()#;
}

fn with_override() -> #[limit] u32 {
    -- 1. 读取外层安装的 limit（10u32）
    let before = limit()#;

    -- 2. 局部安装内层 limit（3u32）：仅在 sum_limits() 内部生效（3 + 3 = 6）！
    let inside = sum_limits()# {
        limit() => 3u32,
    };

    -- 3. 离开内层后，外层的 10u32 完好无损！
    let after = limit()#;
    return before + inside + after;
}

test {
    let result = with_override()# {
        limit() => 10u32,
    };
    println("{}", result); -- 10 + 6 + 10，输出：26
}
```

---

## 四、本节核心概念速查表

| 特性 / 维度 | 普通 `effect` | `^attrs.effects.direct_passing` 效应 |
| :--- | :--- | :--- |
| **求值时机与次数** | 每次执行 `op()#` 请求时调用一次处理分支 | **安装 Handler 时求值一次**，主体未开始前即就绪 |
| **参数限制** | 可带任意参数及 `catch continuation` | **严禁携带任何参数**（包括 `catch` 参数） |
| **底层执行形态** | 按入口调用处理函数（或捕获切栈） | 由 Elaboration 编译为**直接传值/隐式参数读取**，零切栈开销 |
| **`effect enum` 粒度** | 按需检查每个操作 | `#Group.item` 仅传递该单项值，无需在运行时构造整组大结构体 |
| **内层局部覆盖** | 建立新的内层 Handler 边界 | 建立独立的内层值槽，绝不覆盖外层实例，退出内层自动恢复外层值 |

---

## 五、下一期预告

到这里，我们已经见识了代数效应的两种轻量级执行形态：
1. **Direct Passing（直接传值）**：安装时算好一个值，后续读取完全零调用开销；
2. **Direct Return（同步直接返回 / 尾恢复）**：每次请求时像普通函数调用一样运行一下 Handler 分支，拿到结果 `Y` 后立刻在原地继续。

但是，如果我们正在编写微型运行时的**协程调度器（Coroutine Scheduler）**或**流式生成器（Generator）**，当主体发出一个 `yield_number(42)#` 或 `pause()#` 请求时，Handler 并不想让主体立刻继续往下跑，而是想**把主体从当前暂停点往后的“剩余计算（Delimited Continuation）”打包存起来，等过一会儿（或者交由外层循环按需驱动）再恢复它**！

在 Flurry 中：
- 什么样的 `effect` 声明才被允许捕获并暂停剩余计算？
- 为什么 Flurry 的显式 Continuation 坚持采用 **单次恢复（One-Shot）** 契约，而坚决禁止像早期 Scheme `call/cc` 那样多次克隆恢复同一个栈？这与第三卷的线性资源（`Once` / `Drop`）有什么生死攸关的联系？

下一期 **《Flurry 演进手记 (146)：只跑一次、跑在尾部、还是暂停保存？——尾恢复（Tail-Resumptive）与单次限定 Continuation》**，我们将正式踏入限定 Continuation 与用户态协程调度的核心秘境！
