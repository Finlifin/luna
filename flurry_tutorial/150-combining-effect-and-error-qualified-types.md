# Flurry 演进手记 (150)：当 #Effects !Errors T 组合在一起——限定前缀的嵌套顺序与交互法则

> **系列导读**：在上一期（149）中，我们厘清了错误限定 `!Err T` 与效应限定 `#Eff T` 在底层 ABI、行去重代数以及消去控制流上的楚河汉界。但在真实的系统工程中，这两者绝不是老死不相往来的孤岛——恰恰相反，在我们的微型运行时中，几乎每一个网络或传感器采集函数都**既需要发出 `#Io` / `#Clock` 效应请求，又可能在校验报文时返回 `!ParseErr` 错误**！当函数的返回类型写成 `#Effects !Errors T` 时，这两层限定前缀是如何像洋葱一样层层嵌套的？为什么中间的 `!` 不会被误解析为后缀操作符？在调用端我们又该如何一层一层把它们优雅地剥开？今天，我们将彻底打通多层限定类型的组合与消去法则！

---

## 一、语法设计之美：“类型偏前缀，表达式偏后缀”的镜像哲学

在深入 `#Effects !Errors T` 之前，让我们先退后一步，欣赏一下 Flurry 语法体系（详见 `docs/grammar/index.md`）中最具美感的一条核心对称法则：
> **在 Flurry 中，类型表达式倾向于使用【前缀（Prefix）】组合，而值表达式倾向于使用【后缀（Postfix）】链式消去！**

让我们把这两侧的算子并排放在一起，你会立刻发现一幅严丝合缝的“镜像对称图”：

| 语义维度 | 类型一侧（前缀嵌套：由外向内包裹） | 表达式一侧（后缀链式：由外向内剥壳！） |
| :--- | :--- | :--- |
| **指针 / 引用** | `*T` | `ptr.*` / `val.ref` |
| **可选值（Option）** | `?T` | `opt?` |
| **错误结果（Error）** | `!Errors T` | `res!` / `res! { ... }` |
| **代数效应（Effect）** | `#Effects T` | `comp#` / `comp# { ... }` |
| **动态接口（Trait Object）** | `dyn TraitExpr` | `val.dyn(TraitExpr)` |

### Code Cartoon：“类型前缀包裹洋葱”与“表达式后缀剥洋葱”

理解了这层镜像关系，当你看到一个返回类型为 **`#[read_raw] !ParseErr u32`** 的函数时，你脑海里应该立刻浮现出一个两层结构的“洋葱模型”：

```text
  [ 类型声明（前缀由外向内包裹） ]
        #read_raw          !ParseErr          u32
    ┌─── 外层：效应壳 ─── ┌── 内层：错误壳 ── ┌─────────┐ ──┐ ───┐
    │                    │                  │ 核心值   │   │    │
    │                    │                  │  u32    │   │    │
    └────────────────────┴──────────────────┴─────────┴───┴────┘
          │                      ▲                ▲
          ▼ (第 1 步：后缀 #)     │ (第 2 步：后缀 !)│
     comp# 剥开外层效应壳 ────────┘                │
     comp#! 再剥开内层错误壳 ──────────────────────┘
```

1. **最外层是效应壳 `#read_raw`**：表示要运行这段计算，首先需要在执行过程中回答它提出的 `read_raw` 效应请求；
2. **往里一层是错误壳 `!ParseErr`**：表示当这段计算的效应请求都被满足、函数执行完毕退栈后，它交付回来的返回值是一个可能成功（`u32`）、也可能失败（`ParseErr`）的错误限定值；
3. **最核心是 `u32`**：当错误也被成功解包或消去后，最终拿到的业务整数！

---

## 二、语法解析揭秘：为什么 `#Effects !Errors T` 里的 `!` 不会产生歧义？

很多对编译器语法分析器（Parser）感兴趣的朋友看到 `#Effects !Errors T` 时，第一反应往往是：
“等等！既然在 Flurry 中类型本身也是编译期表达式，而 `!` 又是表达式的后缀错误传播运算符（如 `foo()!`），那编译器在解析 `#Effects !Errors T` 时，为什么不会把中间的 `!` 误解析成作用在 `Effects` 身上的后缀操作符 `(#[Effects!]) Errors T` 呢？！”

答案就在于 Flurry 语法对**限定类型前缀（Qualified Type Prefix）**的精确边界设计：
1. **前缀链式嵌套识别**：在限定类型位置，`#expr expr` 和 `!expr expr` 的前缀操作数明确不把紧随其后的下一层限定前缀 `!` 当成前一个行表达式的后缀操作，而是直接保存为**外层效应限定 + 内层错误限定**的两层嵌套结构！
2. **与函数箭头及外层谓词的优先级边界**：
   限定前缀的主体会自动包含函数的返回箭头 `->`，但**绝不会贪婪吞掉外层的相等比较 `==`、Trait 约束 `:-` 或布尔逻辑连接词 `and` / `or`**！

```flurry
-- 1. 比较两个限定类型是否相等：
-- == 位于限定主体边界之外，比较的是整个 (#Effects fn() -> T) 与 OtherType！
-- #Effects fn() -> T == OtherType

-- 2. 对限定类型施加 Trait 约束：
-- :- 位于限定主体边界之外！
-- #Effects T :- SomeTrait
```

> 💡 **语法贴士**：如果某个复杂的编译期限定行表达式内部确实需要使用可能与下一层前缀产生歧义的运算，只需用方括号 `#[...]` 或圆括号 `(...)` 明确划定范围即可。

---

## 三、深度法则：同类前缀可合并，跨类前缀（`#` 与 `!`）严禁随意交换！

在上一期（149）和第 088 期中，我们知道**同一类别**的限定前缀在连续叠加时，会遵循各自的行连接/合并代数：
- **同类错误前缀叠加**：自动合并唯一标签，即 `!ErrA !ErrB T` 等价于 `![ErrA, ErrB] T`，且 `!ErrA !ErrA T == !ErrA T`；
- **同类效应前缀叠加**：按行连接并保留同名层级，即 `#eff_a #eff_b T` 等价于 `#[eff_a, eff_b] T`（而空限定 `#[ ] T` 或 `![ ] T` 自动消去还原为 `T`）。

但是，**不同类别的限定前缀（比如 `#Eff` 与 `!Err`）之间，绝对不会自动交换顺序！**
为什么 **`#Eff !Err T` 不等于 `!Err #Eff T`**？

```text
  [ 形态 A：#Eff !Err T（最常见：先执行带效应的计算，最终返回带错误的结果） ]
    执行过程请求 #Eff ──► 函数结束返回结果 ──► 结果是一个值：要么是 T，要么是 error Err

  ──────────────────────────────────────────────────────────────────────────────────

  [ 形态 B：!Err #Eff T（先判断一个纯结果分支，若成功则交付一段带 #Eff 的计算对象） ]
    外层是一个无需效应即可判别的错误结果，成功载荷内部才包裹着 #Eff T
```

正因为两者的物理含义和控制流次序完全不同，编译器绝不会擅自把外层的 `#Eff` 和内层的 `!Err` 颠倒过来！

---

## 四、实战演进：从 `v0` 到 `v2`，花式剥开 `#[read_raw] !ParseErr u32` 这颗洋葱！

现在，让我们在微型运行时中编写一个最典型的传感器帧读取与校验函数 `fetch_valid_sample()`：
1. 它通过 `#read_raw` 效应从底层虚拟总线读取一个原始 `u32` 刻度；
2. 如果读到的刻度等于硬件故障码 `0xFFFFu32`（即 `65535u32`），它就返回 `!ParseErr` 错误；否则返回合法的测量值。

```flurry
enum ParseErr {
    HardwareFault,
    OutOfRange(u32),
}

effect read_raw() -> u32;

-- 同时携带外层效应 #[read_raw] 与内层错误 !ParseErr 的核心函数！
fn fetch_valid_sample() -> #[read_raw] !ParseErr u32 {
    -- 1. 发起效应请求，从外部环境取得原始读数 raw: u32
    let raw = read_raw()#;

    -- 2. 校验数据合法性，返回 !ParseErr u32
    if raw == 65535u32 {
        return error ParseErr.HardwareFault;
    }
    if raw > 1000u32 {
        return error ParseErr.OutOfRange(raw);
    }
    return raw;
}
```

当上层函数调用 `fetch_valid_sample()` 时，我们有三种极其灵活的“剥洋葱”方式：

### 方式 1：一路向外传播——连写后缀 `#!`（先传 `#`，再传 `!`）

如果中转函数 `double_sample()` 既不想自己处理 `read_raw` 效应，也不想自己处理 `ParseErr` 错误，它只需要按照由外向内的洋葱顺序，紧挨着写上后缀 **`#!`**！

```flurry
-- 保持外层 #[read_raw] !ParseErr u32 签名，用 #! 一键双传！
fn double_sample() -> #[read_raw] !ParseErr u32 {
    -- 拆解过程：
    -- 1. fetch_valid_sample()   类型是 #[read_raw] !ParseErr u32
    -- 2. fetch_valid_sample()#  剥开外层效应，得到 !ParseErr u32
    -- 3. fetch_valid_sample()#! 再剥开内层错误（若错则短路退栈），得到纯净的 u32！
    let val = fetch_valid_sample()#! ;
    return val * 2u32;
}
```

### 方式 2：先用 `# { ... }` 消去外层效应，再用 `! { ... }` 消去内层错误

在测试或顶层调度入口处，我们要把效应和错误全部消去，变成一个纯净的 `u32`。
按照由外向内剥洋葱的顺序：
1. 先对 `double_sample()` 安装 **`# { read_raw() => ... }`** 消去外层的 `#[read_raw]` 效应，此时整个 `# { ... }` 块产出的结果类型就是内层的 **`!ParseErr u32`**；
2. 紧接着在这个块后面接上 **`! { ... }`**，消去内层的 `!ParseErr` 错误，最终拿到 `u32`！

```flurry
test {
    -- 场景 A：虚拟总线返回正常值 40u32
    let ok_val = (double_sample()# {
        read_raw() => 40u32,
    })! {
        ParseErr.HardwareFault => 0u32,
        ParseErr.OutOfRange(_) => 1000u32,
    };

    -- 场景 B：虚拟总线返回超限值 2500u32
    let clamped_val = (double_sample()# {
        read_raw() => 2500u32,
    })! {
        ParseErr.HardwareFault => 0u32,
        -- 命中 OutOfRange 分支，截断为上限 1000u32！
        ParseErr.OutOfRange(_) => 1000u32,
    };

    println("{} {}", ok_val, clamped_val); -- 输出：80 1000
}
```

### 方式 3：先在内部消去错误，保留效应留给外层提供！

那如果我们想写一个“永不报错、遇到故障自动归零，但依然把 `read_raw` 效应留给外层测试驱动提供”的函数 `fetch_or_zero() -> #[read_raw] u32` 呢？
同样简单无比：先用 `fetch_valid_sample()#` 把外层效应传播到当前函数环境，得到 `!ParseErr u32`，然后立刻原地接上 `! { ... }` 把错误消去！

```flurry
-- 消去了内层 !ParseErr，只向外暴露 #[read_raw] u32！
fn fetch_or_zero() -> #[read_raw] u32 {
    -- 1. 后缀 # 将 read_raw 效应向上传播到 fetch_or_zero 的允许行中；
    -- 2. 后缀 ! { ... } 就地将 !ParseErr 消去为 u32！
    return fetch_valid_sample()#! {
        ParseErr.HardwareFault => 0u32,
        ParseErr.OutOfRange(_) => 0u32,
    };
}

test {
    let v = fetch_or_zero()# {
        read_raw() => 65535u32,
    };
    println("{}", v); -- 输出：0
}
```

---

## 五、本节核心概念速查表

| 语法组合 | 解析与类型结构 | 核心消去与传播规则 |
| :--- | :--- | :--- |
| `#Effects !Errors T` | 外层效应限定 `#Effects`，包裹内层错误限定 `!Errors T` | `!` 绝不会被误解析为 `Effects` 上的后缀操作符；跨种类前缀不可随意交换 |
| `expr#!` | 先执行 `expr#`（传播效应），再执行 `!`（传播错误） | 一步剥开外层 `#Effects` 与内层 `!Errors` 两层洋葱壳，直取核心值 `T` |
| `expr#! { err_arms }` | 先用 `#` 将效应传至当前函数，再用 `! { ... }` 就地消去错误 | 将 `#Effects !Errors T` 转化为不含错误的 `#Effects T` |
| `(expr# { eff_arms })! { err_arms }` | 先用 `# { ... }` 消去外层效应，再用 `! { ... }` 消去内层错误 | 依次消去两层限定，最终得到纯净的无限定值 `T` |

---

## 六、下一期预告：进军异步代数效应（Async Effects）与结构化并发！

恭喜你！到这里，我们已经走完了第七卷前半程（141–150）的壮阔旅途：
从代数效应 `effect` 的声明、`#` 与 `resume` 的往返控制流、`effect enum` 与 Scoped Labels 嵌套转发、基于 Trait 的 `.use(handler)` 注入、`^attrs.effects.direct_passing` 零开销传值，到单次限定 Continuation、隐藏中止通道的确定性 `Drop` 清理、50 行代码手写流式生成器，直至 `#Effects !Errors T` 的洋葱组合模型！

但是，在前面这 10 期里，我们所有的控制流都还是在**同步顺序或单步挂起恢复**的范畴内。
在第七卷的后半程（151–160），我们将把微型运行时推向真正的高并发深水区——**异步代数效应（`async effect`）与结构化并发（Structured Concurrency）**！
- 在其他语言（Rust / JS / C++）里，一旦一个函数标上了 `async fn`，它就会产生臭名昭著的**“函数染色问题（Function Coloring）”**，并且返回的 `Future` 与代数效应格格不入；
- 而在 Flurry 中，异步根本不是硬编码在所有函数上的枷锁，而是直接作为 **`async effect`** 融入代数效应体系！当主体请求一个 `async effect` 时，为什么 Handler 启动的是一个并发子工作（Child Future），而调用者自己可以继续往下跑？
- 当并发任务组中有一个子任务失败退出时，Flurry 的结构化退出协议是如何保证**所有兄弟异步任务全部安全取消并完成 `Drop` 清理、绝不留下任何后台孤儿协程（Orphan Tasks）**的？

下一期 **《Flurry 演进手记 (151)：告别“红蓝函数染色”——async effect、并发子工作与按需等待 Future》**，让我们开启并发与异步的全新篇章！
