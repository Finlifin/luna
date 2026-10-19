# Flurry 演进手记 (143)：组织一整套系统能力——effect enum 操作组、嵌套处理与未覆盖效应自动转发

> **系列导读**：在上一期（142）中，我们厘清了代数效应 `#` 与错误 `!` 在控制流上的本质分野，掌握了 `resume` 原地恢复执行的威力。但在构建微型运行时或大型驱动子系统时，我们绝不可能让每个细碎的操作都散落成顶层独立的 `effect`：一个遥测汇报器、一个虚拟文件系统或一个协程调度器，往往由多个紧密协作的操作组成。此外，当我们在已有 Handler 内部再套一层拦截器（Middleware / Decorator）时，嵌套的同名效应是如何做到既不死循环、又能只拦截一部分请求并将剩余请求透明转发给外层的？今天，我们将深入探索 **`effect enum` 操作组**与 **Scoped Labels（带层级的效应行）**！

---

## 一、零散效应的痛点：版本 `v0` 的“签名爆炸”

假设我们在微型运行时中设计一个遥测汇报子系统，它需要两个操作：
1. 获取当前批次的采样计数 `count() -> u32`；
2. 输出一条阶段标记文本 `message(text: str) -> void`。

如果我们只用上一期学过的独立 `effect` 声明（`v0` 写法）：

```flurry
-- v0：零散的顶层 effect 声明
effect report_count() -> u32;
effect report_message(text: str);

-- 痛点：只要一个子系统有 5 个操作，所有使用它的函数签名都得把 5 个名字挨个抄一遍！
fn report_twice_v0() -> #[report_count, report_message] u32 {
    report_message("begin")#;
    let c = report_count()#;
    report_message("end")#;
    return c * 2u32;
}
```

如果一个虚拟文件系统有 `open`、`read`、`write`、`seek`、`close` 5 个操作，难道每个业务函数的返回类型都要写成 `#[fs_open, fs_read, fs_write, fs_seek, fs_close] u32` 吗？
反过来，你也不能把它们硬塞进一个普通的 `enum` 里——因为 `count()` 返回的是 `u32`，而 `message(text: str)` 返回的是 `void`！普通 `enum` 的变体只能挂载数据载荷，根本无法表达**“不同变体拥有各自独立的参数列表与不同的返回类型 `Y`”**！

---

## 二、迭代演进 `v1`：用 `effect enum` 组织异构签名的操作组

为了解决这个痛点，Flurry 提供了专门的 **`effect enum`（效应操作组）** 语法：

```text
effect_enum    -> effect enum id clauses? { effect_variant* }
effect_variant -> attribute* async? inline? id(parameter*) (-> result)?
```

请注意 `effect enum` 的三大核心特征：
1. **变体之间以逗号 `,` 分隔**，每个变体都是一个完整的**操作签名**（省略 `-> result` 时默认返回 `void`）；
2. **各自拥有独立签名与返回类型**：`Report.count()` 可以返回 `u32`，而 `Report.message(text: str)` 返回 `void`，编译器会针对每个具体操作分别检查其实参类型与返回类型 `Y`；
3. **支持整组引用与单项精确引用**：
   - 在类型前缀中写 **`#Report u32`**，代表允许请求 `Report` 组内的整组操作；
   - 在类型前缀中写 **`#Report.count u32`**，则精确声明该函数**只使用** `Report` 组里的 `count` 这一个操作！

让我们用 `effect enum Report` 重构出清爽无比的 `v1` 代码：

```flurry
-- 声明一个包含两个不同签名操作的效应组 Report
effect enum Report {
    count() -> u32,
    message(text: str),
}

-- 用 #Report 一次性涵盖整组操作需求！
fn report_twice() -> #Report u32 {
    Report.message("begin")#;
    let count = Report.count()#;
    Report.message("end")#;
    return count * 2u32;
}

test {
    let result = report_twice()# {
        -- 分别按各自的操作返回类型 Y 交付结果：
        -- count() 交付 u32，message(text) 交付 void！
        Report.count() => 4u32,
        Report.message(text) => println("{}", text),
    };
    println("{}", result); -- 依次输出：begin、end、8
}
```

---

## 三、多层嵌套与同名转发：为什么内层 Handler 调 `read_number()#` 不会死循环？

在系统架构中，最强大的设计模式莫过于**中间件 / 装饰器（Middleware / Decorator）**：
假设底层硬件或测试驱动在外层提供了一个基础的 `read_number` 处理器（返回原始值 `12u32`）。现在，我们想在中间加一层透明的“缩放适配器” `with_halved_input`：它拦截内部发出的每一次 `read_number()#` 请求，先向外层索要原始数值，除以 `2` 之后再交回给内部主体！

让我们先看这段代码在 Flurry 中是如何写的：

```flurry
effect read_number() -> u32;

fn twice_input() -> #[read_number] u32 {
    let first = read_number()#;
    let second = read_number()#;
    return first + second;
}

-- 中间件层：拦截内层的 read_number，向更外层的 read_number 请求并折半！
fn with_halved_input() -> #[read_number] u32 {
    return twice_input()# {
        read_number() => {
            -- ⚠️ 灵魂拷问：这里也写了一句 read_number()#，
            -- 为什么它不会死循环跳回当前这个花括号分支自己身上？！
            let original = read_number()#;
            original / 2u32
        },
    };
}

test {
    let result = with_halved_input()# {
        read_number() => 12u32,
    };
    println("{}", result); -- 两次请求各得 6u32，相加输出：12
}
```

### Code Cartoon：Deep Handler 的边界法则与重装机制

为什么 `with_halved_input` 里的 `read_number()#` 不会死循环？为什么 `twice_input` 里的第二次 `read_number()#` 又能继续回到内层除二处理器？

让我们用一张图看懂 Flurry **Deep Handler（深处理器）** 的黄金执行法则：

```text
  ┌─────────────────── [ 外层 Handler H_outer: 返回 12u32 ] ───────────────────┐
  │                                                                            │
  │   ┌─────────────── [ 内层 Handler H_inner: 除 2 转换 ] ───────────────┐    │
  │   │  被包裹主体 twice_input():                                        │    │
  │   │    1. 发起第 1 次请求 read_number()#  ───────┐                    │    │
  │   │    4. 恢复后【重装 H_inner】，发起第 2 次请求 ─┼───► 同样命中 H_inner!   │
  │   └──────────────────────────────────────────────┼────────────────────┘    │
  │                                                  ▼                         │
  │   2. H_inner 的处理分支在【H_inner 边界之外、H_outer 边界之内】执行！      │
  │      因此分支内的 read_number()# 直接向上命中 H_outer (拿到 12u32)，        │
  │      算出 12 / 2 = 6u32 后，带着 H_inner 保护壳交回给主体！                │
  └────────────────────────────────────────────────────────────────────────────┘
```

请牢记这两条精妙至极的规则：
1. **分支在当前边界外执行**：当 `H_inner` 拦截到请求并开始运行它的处理分支时，该分支本身处于 **`H_inner` 的边界之外（即 `H_outer` 的环境内）**！因此分支里发出的同名请求 `read_number()#` 天然发往外层 `H_outer`，绝不会自己咬自己的尾巴。
2. **恢复时自动重装（Re-install on Resume）**：当 `H_inner` 的分支把 `6u32` 交回给 `twice_input` 的暂停点时，`twice_input` 剩余的计算依然包裹在 `H_inner` 之内！因此紧接着发出的第二次请求依然由 `H_inner` 拦截，不会因为处理完第一次就“卸载失效”。

---

## 四、部分拦截与透明转发：当内层只处理 `ask(true)` 时会发生什么？

更进一步：如果一个效应操作带有参数（如 `inline effect ask(value: bool) -> bool;`），而内层 Handler **只写了匹配 `ask(true)` 的分支**，没有覆盖 `ask(false)`，会发生什么？

1. **静态安全检查**：因为内层 Handler 没有完全覆盖所有可能的 `ask(value)` 请求，编译器知道 `ask(false)` 会逃逸出去，因此要求外层环境**必须提供合法的 `#ask` 处理入口**（若外层没有提供，编译期直接拒绝，绝不把“没匹配上”当成隐式跳过！）。
2. **转发不卸载内层 Handler**：当 `ask(false)#` 穿透内层转发给外层，并由外层回答完之后，控制流回到主体继续往下跑时，**内层 Handler 依然稳稳地包裹着主体**！随后发出的第 3 次请求 `ask(true)#` 照样由内层 Handler 拦截！

```flurry
inline effect ask(value: bool) -> bool;

inline fn produce() -> #ask bool {
    let a = ask(true)#;   -- 第 1 次：传入 true
    let b = ask(false)#;  -- 第 2 次：传入 false
    let c = ask(true)#;   -- 第 3 次：再次传入 true！
    return not a and b and not c;
}

-- 内层过滤器：只拦截 ask(true) 并固定返回 false；
-- 未覆盖的 ask(false) 自动向外层转发，所以函数签名仍保留 #ask！
inline fn filtered() -> #ask bool {
    return produce()# {
        ask(true) => false,
    };
}

fn entry() -> bool {
    -- 外层兜底处理器：回答所有逃逸出来的 ask(value)
    return filtered()# {
        ask(value) => not value,
    };
}
```

看看 `entry()` 的执行轨迹：
- `a = ask(true)#`：命中内层 `filtered`，得到 `false`；
- `b = ask(false)#`：内层未覆盖，自动转发到外层 `entry`，得到 `not false = true`；
- `c = ask(true)#`：**内层并未因刚才的转发而丢失！** 再次命中内层 `filtered`，得到 `false`；
- 最终 `not false and true and not false` 结果为 `true`！

---

## 五、回望第四卷：为什么 `#[e] #[e] T != #[e] T`？（Scoped Labels 铁律）

现在，我们可以彻底理解在第 088 期学过的那条深刻的行类型（Row Types）定律了：
为什么在错误行中，重复的错误会自动合并去重（`![InputErr] ![InputErr] u32 == ![InputErr] u32`）；而在效应行中，**重复的效应标签绝对不能合并去重（`#[read_number] #[read_number] u32 != #[read_number] u32`）**？

```flurry
enum InputError {
    OutOfRange(u32),
}

effect read_number() -> u32;

typealias OneError = ![InputError] u32;
typealias RepeatedError = ![InputError] ![InputError] u32;

typealias OneLayer = #[read_number] u32;
typealias TwoLayers = #[read_number] #[read_number] u32;

comptime {
    -- 1. 错误行是唯一标签集合：无论经过多少层传播，同一个错误类型只是同一个返回分支！
    assumes OneError == RepeatedError;

    -- 2. 效应行是带作用域层级的标签序列（Scoped Labels）：
    -- 两层 read_number 代表需要【两层嵌套的 Handler 实例（Occurrences）】来分别消去！
    assumes OneLayer != TwoLayers;
}
```

> **核心总结**：
> - 在同一个函数里**顺序调用 100 次** `read_number()#`，它们共用同一层处理器，类型依然是**一层** `#[read_number] u32`；
> - 只有当类型本身携带了**两层嵌套的待消去效应边界**时，才会出现 `#[read_number] #[read_number] u32`！**效应行中的重数（Multiplicity）代表的是 Handler 嵌套层数（Occurrences），而不是运行期调用次数！**

---

## 六、本节核心概念速查表

| 语法 / 机制 | 写法示例 | 核心语义规则 |
| :--- | :--- | :--- |
| **效应操作组** | `effect enum Report { count() -> u32, msg(s: str) }` | 组内各项拥有独立参数与返回类型 `Y`；变体间用逗号分隔 |
| **整组 vs 单项引用** | `#Report T` vs `#Report.count T` | `#Report` 要求整组入口，`#Report.count` 仅要求该单项入口 |
| **同名效应向外请求** | 在 `e() => { ... e()# ... }` 分支内调用 `e()#` | 分支运行在当前 Handler 边界之外，同名请求发往外层 occurrence，绝不自我死循环 |
| **部分处理与自动转发** | 仅匹配部分模式或 guard 的 `# { ... }` | 未覆盖请求自动转发给外层（要求外层提供入口），且主体恢复后仍保留内层 Handler |
| **Scoped Labels 行规则** | `#[e] #[e] T != #[e] T` | 不同标签在证明不等时可交换；同名标签严格保序且不去重，重数对应处理层数 |

---

## 七、下一期预告

在前三期（141–143）中，我们都是用匿名花括号语法 `expr# { ... }` 当场手写效应的处理分支。
但在真实的微型运行时工程里，一个虚拟时钟 `VirtualClock`、一个递增计数器 `Counter` 或一个协程栈池 `StackPool`，往往是一个**带有内部状态字段的结构体对象**，甚至是由第三方库预先封装好的组件！

如果我们想把一个实现了特定 Trait 的结构体对象 `handler` 直接挂载为某段计算的处理器：
- 为什么 Flurry 的设计铁律明确规定：**“函数签名不添加 `handles`”**？
- 我们该如何用模式绑定 **`let handler handles LimitErr = ...`** 和后缀 **`expr.use(handler)`** 把对象化的处理能力优雅地注入到计算中？

下一期 **《Flurry 演进手记 (144)：为什么函数签名里不写 handles——基于 Trait 的处理能力（Handling Capability）与 .use(handler)》**，我们将打通第五卷 Trait 系统与第七卷代数效应的任督二脉！
