# Flurry 演进手记 (142)：发起请求与就地拦截——后缀 # 传播、expr# { ... } 效应消去与 resume

> **系列导读**：在上一期（141）中，我们用 `effect` 和 `#Effects T` 开启了第七卷的大门，把“请求什么操作（What）”与“如何提供实现（How）”彻底解耦。很多从第二卷一路学过来的读者看到 `expr#` 和 `expr# { ... }` 时，都会产生一种强烈的既视感：“这不就和我们在第 029–030 期学过的错误传播 `expr!` 与错误消去 `expr! { ... }` 长得一模一样吗？”今天，我们就来正面回答这个最核心的问题：为什么语法上如此对称的 `!` 和 `#`，在控制流本质上却是**“一去不复返的短路退出”**与**“有去有回的原地恢复（`resume`）”**的天壤之别！

---

## 一、直观对比：当错误 `!` 与效应 `#` 面对同一段乘法计算

为了看清 `!`（错误）与 `#`（效应）的根本差异，让我们写出两段结构几乎完全对称的代码：一段用 `!InputErr` 请求输入，另一段用 `#[read_number]` 请求输入，拿到数字后都乘以 `2u32`。

### 场景 A：错误限定 `!` —— 失败即中止，永远回不到乘法现场！

```flurry
enum InputErr {
    Missing,
}

fn require_number(present: bool) -> !InputErr u32 {
    if present {
        return 6u32;
    } else {
        return error InputErr.Missing;
    }
}

fn doubled_err(present: bool) -> !InputErr u32 {
    -- 使用后缀 ! 传播错误：一旦失败，立即从 doubled_err 短路退出！
    let number = require_number(present)!;
    -- ⚠️ 当 present == false 时，下面这行乘法永远不会执行！
    return number * 2u32;
}

test {
    let result = doubled_err(false)! {
        -- 这里的 5u32 是整个 doubled_err(false) 失败后的【最终替代结果 B】，
        -- 而不是交回给 let number = ... 继续去乘 2 的输入！
        .Missing => 5u32,
    };
    println("{}", result); -- 输出：5（而不是 10！）
}
```

### 场景 B：代数效应 `#` —— 回答问题后，回到暂停点继续完成乘法！

```flurry
effect read_number() -> u32;

fn doubled_eff() -> #[read_number] u32 {
    -- 使用后缀 # 发起/传播效应请求：向外部环境索要一个 u32
    let number = read_number()#;
    -- ✅ 外部 Handler 回答 5u32 后，控制流回到这里，继续执行 5u32 * 2u32！
    return number * 2u32;
}

test {
    let result = doubled_eff()# {
        -- 这里的 5u32 是回答给 read_number()# 那一次请求的【操作结果 Y】！
        read_number() => 5u32,
    };
    println("{}", result); -- 输出：10！
}
```

### Code Cartoon：一张图看懂 `!`（短路替换）与 `#`（往返恢复）的控制流鸿沟

```text
  [ 错误消去 doubled_err(false)! { .Missing => 5u32 } ]
    doubled_err 内部：
      let number = require_number(false)!  ──(发生错误 Missing)──┐
      return number * 2u32; (❌ 被彻底跳过！)                    │
                                                                 ▼
    外层 ! { .Missing => 5u32 }  ◄─────────(一去不复返)──────────┘
    最终结果 = 5u32

  ────────────────────────────────────────────────────────────────────

  [ 效应消去 doubled_eff()# { read_number() => 5u32 } ]
    doubled_eff 内部：
      let number = read_number()# ─────────(1. 发起请求)─────────┐
          ▲                                                      │
          └──────(3. 带着 Y = 5u32 恢复！)───────────────────────┤
      return number * 2u32; (✅ 算出 10u32 正常返回！)           ▼
                                            (2. 外层 Handler 分支求值出 5u32)
```

这就是理解代数效应的第一把金钥匙。在 Flurry 的类型系统中，我们用三个字母严格区分 Handler 涉及的**三个结果类型**：
- **`Y`（操作结果 Operation Result）**：单次 `effect` 操作签名声明的返回类型（由 Handler 分支交付给 `expr#` 调用点）；
- **`A`（主体正常结果 Body Result）**：被包裹的计算主体正常执行到底后产出的结果类型；
- **`B`（整体处理结果 Handled Result）**：整个 `expr# { ... }` 表达式最终对外交付的结果类型（默认保持 `B = A`）。

---

## 二、迭代演进 `v1`：用 `resume` 在分支内提前交付操作结果 `Y`

在上面的简单例子 `read_number() => 5u32` 中，Handler 分支直接以表达式 `5u32` 结尾，编译器会自动将分支的末尾结果作为操作结果 `Y` 交回给请求点。

但如果我们的 Handler 分支内部有复杂的条件判断（比如先检查参数是不是特殊值 `0u32`，如果是就**立刻提前交回默认值 `1u32` 并回到请求点继续**，否则再执行后续缩放计算），该怎么写？

这时候，千万不能在 Handler 分支里写 `return 1u32;`！
为什么？因为 **`return` 的词法目标是直接退出外层整个函数**！为了表示“**结束当前这次效应请求的处理分支，将结果 `Y` 交回给暂停的请求点，让被处理主体继续往下跑**”，Flurry 提供了专门的关键字——**`resume`**！

```text
resume_statement -> resume expr? (while expr)?
```

让我们通过 `use_scale` 的演进代码，亲眼看看 `resume` 是如何运作的：

```flurry
-- 声明一个带参数、返回 u32 的效应操作
effect scale(value: u32) -> u32;

fn use_scale() -> #[scale] u32 {
    -- 第 1 次请求：传入 0u32
    let first = scale(0u32)#;
    -- 第 2 次请求：传入 4u32
    let second = scale(4u32)#;
    return first + second;
}

test {
    let result = use_scale()# {
        scale(value) => {
            -- 当 value == 0u32 时，使用 resume 1u32; 提前向请求点交付操作结果 1u32！
            -- 注意：resume 只结束【当前这一次请求】的处理分支，
            -- 主体拿到 first = 1u32 后继续往下走，依然处于当前 Handler 的保护伞下，
            -- 紧接着发出的第二次请求 scale(4u32)# 依然会进入这个 Handler！
            if value == 0u32 {
                resume 1u32;
            }
            value * 2u32
        },
    };
    -- first 得到 1u32，second 得到 8u32，相加输出 9！
    println("{}", result);
}
```

### ⚠️ 核心控制流边界：`resume` vs `return` / `break` / `continue`

在 Handler 分支 `{ ... }` 内部写代码时，请务必分清这四个控制流出口的去向：

| 语句 | 在 Handler 分支内的真实语义与去向 |
| :--- | :--- |
| **`resume expr;`** | 提前交付本次效应请求的操作结果 `Y = expr`，结束当前分支，**恢复被处理主体从请求点继续执行**（若操作返回 `void`，直接写 `resume;`） |
| **分支末尾值 `expr`** | 与在分支末尾执行 `resume expr;` 等价，将 `expr` 作为操作结果 `Y` 交回请求点 |
| **`return expr;`** | 保留外层函数的词法目标：**直接终止被处理主体及当前外层函数**（触发控制后端的退出与逆序 `Drop` 清理协议，绝不把 `expr` 当成操作结果 `Y` 交回主体！） |
| **`break` / `continue`** | 若指向安装位置外层的词法循环，同样按退出协议清理被退出的主体与中间层，再跳转到外层循环目标 |

---

## 三、终极演进 `v2`：当效应操作自身返回错误限定类型 `!Err T`

在构建微型运行时的虚拟 I/O 层时，我们经常会遇到一个既需要“可插拔效应 Handler”，又需要“向调用点报告某次读取失败”的场景：
比如，我们声明一个效应操作 `effect try_read() -> !InputErr u32;`。

这时候，效应 `#` 和错误 `!` 是怎样完美配合、互不越界的？
1. **对 Handler 而言**：操作签名规定的操作结果类型 `Y` 就是 **`!InputErr u32`**！因此，Handler 分支既可以返回一个正常的 `u32`（成功注入为 `!InputErr u32`），也可以合法地返回 `error InputErr.Missing`——因为对效应协议来说，交付一个 `error InputErr.Missing` **就是成功地回答了这次 `try_read` 请求（交回了一个类型为 `!InputErr u32` 的值 `Y`）**！
2. **对被调主体而言**：`try_read()#` 解开的是外层的效应 `#`，拿到的返回值 `input` 类型正是 `!InputErr u32`。接下来主体是用 `input!` 继续传播错误，还是用 `input! { .Missing => 0u32 }` 就地消去错误，完全由主体自己决定！

```flurry
enum InputErr {
    Missing,
}

-- 操作结果 Y 本身是一个错误限定类型 !InputErr u32
effect try_read() -> !InputErr u32;

fn read_or_zero() -> #[try_read] u32 {
    -- 1. 后缀 # 发起 try_read 请求，得到操作结果 input: !InputErr u32
    let input = try_read()#;
    -- 2. 主体自己使用 ! { ... } 消去其中的 InputErr 错误！
    return input! {
        .Missing => 0u32,
    };
}

test {
    let result = read_or_zero()# {
        -- Handler 分支合法返回一个 !InputErr u32 值（即 error InputErr.Missing），
        -- 交回给 read_or_zero 里的 let input = ... 继续执行！
        try_read() => error InputErr.Missing,
    };
    println("{}", result); -- 输出：0
}
```

反过来，如果某个效应声明的是 `effect must_read() -> u32;`（返回类型里没有 `!InputErr`），而 Handler 分支里却妄图返回 `error InputErr.Missing`：

```flurry
effect must_read() -> u32;

fn use_must_read() -> #[must_read] u32 {
    return must_read()#;
}

fn bad_handler_error() -> u32 {
    return use_must_read()# {
        -- ❌ 编译期直接拒绝！
        -- must_read 的操作结果 Y 是 u32，不含 !InputErr，
        -- 效应 Handler 不能绕过操作签名私自把错误值塞给期望 u32 的调用点！
        must_read() => error InputErr.Missing,
    };
}
```

---

## 四、本节核心概念速查表

| 特性 / 机制 | 错误限定 `!Err T` | 效应限定 `#Eff T` |
| :--- | :--- | :--- |
| **操作语义本质** | 已完成的**结果分支**（成功值或错误载荷） | 向外层环境发出的**操作请求**（等待回答 `Y` 后继续） |
| **后缀 `!` vs `#`** | `expr!`：成功则解包，失败则从当前函数短路返回 | `expr#`：发起效应请求或将内层效应向外层环境传播 |
| **`expr! { ... }` vs `expr# { ... }`** | 匹配错误载荷并给出**整个失败表达式的替代结果 `B`** | 拦截效应请求并交付**单次操作结果 `Y`**，使主体从暂停点继续执行 |
| **`resume expr;`** | 不适用（错误不保存失败点之后的恢复路径） | 在 Handler 分支中提前交付操作结果 `Y` 并恢复主体执行；主体仍留在原 Handler 作用域内 |

---

## 五、下一期预告

在今天和上一期的例子里，我们每次都只声明了一个孤零零的 `effect read_number() -> u32;`。
但在真实的微型运行时中，一个子系统往往包含**一整组密切相关的操作**：
- 比如遥测报告子系统 `Report` 同时包含获取计数的 `count() -> u32` 和输出消息的 `message(text: str)`；
- 比如虚拟时钟子系统 `Clock` 同时包含 `now() -> u64` 和 `advance(delta: u64)`。

此外，如果我们在外层安装了一个 Handler，在里面**又嵌套安装了一个同名效应的内层 Handler**（比如内层 Handler 把外层读到的数字除以 2 再交给主体），当内层 Handler 分支自己也调用了 `read_number()#` 时，为什么它不会陷入无限自我递归？如果内层 Handler 只拦截了 `ask(true)`，没拦截 `ask(false)`，未覆盖的请求又是如何自动转发给外层的？

下一期 **《Flurry 演进手记 (143)：组织一整套系统能力——effect enum 操作组、嵌套处理与未覆盖效应自动转发》**，我们将揭开 `effect enum` 与多层嵌套效应行（Scoped Effect Rows）的路由法则！
