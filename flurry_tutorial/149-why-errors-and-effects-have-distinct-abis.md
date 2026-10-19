# Flurry 演进手记 (149)：错误 !Err 与效应 #Eff 的楚河汉界——为什么两者拥有截然不同的 ABI 与组合规则

> **系列导读**：在上一期（148）中，我们用代数效应 `#` 轻松写出了支持中途提前终止的流式生成器。看到这里，几乎所有深入思考过类型系统的读者都会提出一个灵魂之问：“在很多纯学术语言（如 Eff 或 Koka）里，异常/错误不就是一种‘永远不调用 `resume` 的代数效应’吗？为什么 Flurry 非要把错误 `!Err T` 和效应 `#Eff T` 拆成两个独立的限定前缀，甚至在语言铁律中专门强调**‘错误结果与代数效应具有不同 ABI’**？”今天，我们将从**底层机器码 ABI、行类型去重代数、以及独具匠心的前后置 `catch` 错误切面机制**三个维度，彻底讲透这两者之间的楚河汉界！

---

## 一、学术浪漫 vs 系统工程现实：如果把 `!Err` 当成 `#Eff` 会发生什么？

在纯理论演算中，把“返回错误”建模成“调用一个不恢复的 `effect raise(err: E) -> Never`”，在数学公式上确实很简洁。
但在追求极致性能、确定性内存布局与 C 语言零成本交互的系统级语言中，如果真的把所有日常的错误处理都塞进代数效应 `#` 的机制里，会立刻引爆三大工程灾难：

1. **调用约定（ABI）无谓膨胀**：哪怕是一个最简单的解析函数 `parse_u8(s: str) -> !ParseErr u8`，如果按效应实现，调用方每次调用它都得向栈或寄存器传入 Handler Evidence（处理器入口指针），或者走一套跨越栈帧的非局部跳转协议——而它明明只需要在寄存器里返回一个简单的带标签结果（Tagged Result）！
2. **类型行在嵌套组合时失控膨胀**：如果内层和外层都可能产生 `ParseErr`，按效应的 Scoped Labels 规则，`#ParseErr #ParseErr u8` 会保留两层标签无法自动去重！
3. **丧失对“完整错误结果值”的一等公民操作能力**：在系统编程中，我们经常需要把一个失败的结果原封不动保存下来、打印统一审计日志，然后在外层决定是重试还是向上原样返回——如果错误只是一个瞬态的效应调用，你就失去了直接把 `!RetryErr u32` 当作普通结果值握在手里的能力！

正因如此，Flurry 划出了一条铁律：
> **错误结果（`!Err T`）与代数效应（`#Eff T`）拥有截然不同的底层 ABI、行代数规则与消去控制流！**

---

## 二、第一道分水岭：底层 ABI —— “已退出的结果值” vs “运行中的处理器路由”

让我们用一张对照图，直观看看 `return error InputErr.Missing` 与 `read_number()#` 在机器码层面的天壤之别：

```text
================================================================================
        错误结果 ABI (!Err T)          vs          代数效应 ABI (#Eff T)
================================================================================
  [ 被调函数状态 ]
    已经执行完毕并退栈（Exited）！            仍然存活在栈上（Active / Suspended）！

  [ 底层机器交付方式 ]
    普通函数返回通道（寄存器 / 返回槽）：     通过 Handler Evidence / 环境入口调用：
    交付 Tag (成功/错误分支索引) + 载荷       传入操作实参 X，等待分支交回 Y（或捕获栈）

  [ 是否携带恢复权 (Continuation) ]
    ❌ 绝不保存失败点之后的恢复路径！         ✅ 直接返回 Y 继续，或捕获单次恢复权 K

  [ 跨 C ABI (^attrs.abi(.c)) 边界 ]
    结果值表示清晰确定                        C ABI 入口的效应行必须为空（需在内部消去）
================================================================================
```

### 实战印证：跨越 `^attrs.abi(.c)` 边界时的效应消去包装

因为外部 C 语言调用者根本不知道怎么向 Flurry 函数提供效应处理器（Handler Evidence），所以 Flurry 明确规定：**带有 `^attrs.abi(.c)` 的导出函数，其签名上的效应行必须为空！** 如果内部实现用到了代数效应，必须在 Flurry 包装层内部安装 Handler 将其消去后，再向 C 语言交付纯净的值结果：

```flurry
effect configured_limit() -> u32;

-- 内部 Flurry 函数：携带 #[configured_limit] 效应行
fn limit_times_two() -> #[configured_limit] u32 {
    let limit = configured_limit()#;
    return limit * 2u32;
}

-- ✅ 导出给 C 语言的 ABI 边界函数：
-- 在内部安装 # { ... } 消去全部效应，对外呈现空效应行的纯净签名！
^attrs.abi(.c)
pub fn exported_limit() -> u32 {
    return limit_times_two()# {
        configured_limit() => 8u32,
    };
}
```

---

## 三、第二道分水岭：行类型代数 —— 唯一标签集合 vs 保序嵌套层级

我们在第 143 期已经初窥了这第二道分水岭，现在让我们将两者并排放在一起做一次终极对比：

| 维度 | 错误行（Error Row：`![E1, E2] T`） | 效应行（Effect Row：`#[e1, e2] T`） |
| :--- | :--- | :--- |
| **数学结构** | **有限唯一标签集合（Unique-Label Row）** | **作用域保序标签序列（Scoped-Label Row）** |
| **同标签合并规则** | **自动去重**：`![E] ![E] T == ![E] T` | **严格保留层数**：`#[e] #[e] T != #[e] T` |
| **不同标签交换** | 已证明不等的错误标签可自由交换 | 已证明不等的效应标签可交换，同名 occurrence 严格保序 |
| **物理含义** | 无论经过多少次传播，同一错误类型只是结果值的同一个错误分支 | 每一层同名标签代表一层需要独立 Handler 消去的嵌套边界 |

正因为错误行是**唯一标签集合**，当你在函数里连续调用十次 `checked_byte(x)!`，或者把两层 `!InputErr` 嵌套在一起时，类型系统会自动将它们合并为干净利落的单一 `!InputErr`，绝不会给调用方制造任何多余的消去负担！

---

## 四、第三道分水岭：错误消去专属的“前置与后置 `catch` 切面机制”

既然错误 `!Err T` 是一个实实在在已经返回的**值**，Flurry 为错误消去语法 `expr! { ... }` 赋予了一项代数效应 `# { ... }` 所没有的独门绝技——**前置 `catch`（Pre-Catch）与后置 `catch`（Post-Catch）切面分支**！

让我们看看它的语法结构：

```text
error_elimination -> expr! { error_arm* }
error_arm         -> case_arm | catch_arm
catch_arm         -> catch pattern => (block | statement)
```

在 `expr! { ... }` 的花括号里，除了匹配具体错误变体的普通分支（如 `InputErr.Missing => ...`）之外，你最多可以放置**一个前置 `catch`**（写在所有普通分支之前）和**一个后置 `catch`**（写在所有普通分支之后；如果只写了一个 `catch`，默认按后置 `catch` 处理）！

### Code Cartoon：前置 `catch`、普通错误分支与后置 `catch` 的执行流水线

```text
                [ 求值一次被消去表达式 expr ]
                              │
               ┌──────────────┴──────────────┐
          (若结果为成功值)              (若结果为错误值)
               │                             │
               ▼                             ▼
      [ 直接交付成功值！ ]         [ 1. 执行前置 catch 分支 (若有) ]
      (跳过全部普通分支与 catch)   (绑定完整的原始错误限定值，用于统一日志/审计)
                                             │ (若未显式 return/break/continue)
                                             ▼
                                   [ 2. 匹配并执行对应的普通错误 arm ]
                                             │
                                   是否存在后置 catch 分支？
                                    /                     \
                                 (无)                     (有)
                                  ▼                        ▼
                    [ 普通 arm 的值即为最终结果 ]  [ 3. 丢弃普通 arm 的普通返回值，
                                                        继续进入后置 catch 分支！
                                                        由后置 catch 交付最终结果或控制流跳转！]
```

让我们通过两段极其优雅的代码，看看前后置 `catch` 在实战中是如何大显身手的！

### 实战 1：用前置 `catch` 统一审计，用后置 `catch` 统一兜底返回值

假设 `load_number` 可能返回 `InputErr` 或 `ParseErr` 两种不同的错误。我们想在任何错误发生时：
1. 先用前置 `catch` 打印一句统一的告警前缀 `"before"`；
2. 再在普通分支里针对不同错误打印具体原因 `"missing"` 或 `"invalid"`；
3. 最后用后置 `catch` 统一返回默认安全值 `9u32`（而无需在每个普通打印分支末尾都重复写一遍 `9u32`）！

```flurry
enum InputErr {
    Missing,
}

enum ParseErr {
    Invalid,
}

fn load_number(present: bool, valid: bool) -> ![InputErr, ParseErr] u32 {
    if not present {
        return error InputErr.Missing;
    }
    if not valid {
        return error ParseErr.Invalid;
    }
    return 42u32;
}

test {
    let value = load_number(false, true)! {
        -- 1. 前置 catch：任何错误发生时最先执行
        catch _ => println("before"),

        -- 2. 普通错误分支：分别观察并打印具体错误（返回值 void 随后被丢弃）
        InputErr.Missing => println("missing"),
        ParseErr.Invalid => println("invalid"),

        -- 3. 后置 catch：在普通分支完成后接管，统一交付最终替代结果 9u32！
        catch _ => 9u32,
    };
    -- 依次输出：before、missing、9
    -- （注：若传入 load_number(true, true) 成功，则直接得 42，所有分支均不执行！）
    println("{}", value);
}
```

### 实战 2：利用后置 `catch` 绑定“完整原始错误值”实现优雅的循环重试！

请注意一条极其精妙的规则：
- 普通错误分支（如 `.Again => ...`）匹配的是**剥去错误标签后的载荷（Payload）**；
- 而 `catch failure => ...` 绑定的 `failure`，则是**未经拆包的完整原始错误限定值（如 `!RetryErr u32`）**！

这意味着：在后置 `catch` 里，如果重试次数还没用完，你可以直接 `continue retry_loop` 重新尝试；如果重试次数已经耗尽，你甚至不需要重新构造错误，直接 **`return failure;`** 就能把刚才那个完整的原始错误结果原封不动地向上层返回！

```flurry
enum RetryErr {
    Again,
    Exhausted,
}

fn attempt(number: u32) -> !RetryErr u32 {
    if number < 2u32 {
        return error RetryErr.Again;
    } else {
        return 42u32;
    }
}

fn with_retry() -> !RetryErr u32 {
    var number = 0u32;
    while :retry_loop true {
        let value = attempt(number)! {
            -- 第一步：普通分支记录具体错误日志
            .Again => println("retry requested"),

            -- 第二步：后置 catch 接收完整的原始错误值 failure: !RetryErr u32！
            catch failure => {
                if number < 2u32 {
                    number += 1u32;
                    -- 显式跳转回外层命名循环重试！
                    continue retry_loop;
                }
                -- 重试次数耗尽，直接将完整的原始 failure 向上返回！
                return failure;
            },
        };
        return value;
    }
    return error RetryErr.Exhausted;
}

test {
    -- 注意：在普通 match 中，成功模式用 value!，错误模式用 error pat
    with_retry() match {
        value! => println("{}", value),
        error _ => println("failed"),
    }
    -- 输出两次 retry requested，随后在第 3 次尝试（number == 2）成功输出 42！
}
```

看到上面的重试是如何运作的了吗？这里的重试完全建立在**显式循环（`while :retry_loop`）与独立的值返回 ABI** 之上，清晰、零切栈、零 Continuation 分配开销！

---

## 五、本节核心概念速查表

| 对比维度 | 错误限定类型 (`!Err T`) | 效应限定类型 (`#Eff T`) |
| :--- | :--- | :--- |
| **底层 ABI 本质** | 已退栈函数的带标签返回结果值 | 运行中计算的处理器入口调用 / 限定 Continuation |
| **行类型重数规则** | 唯一标签集合（`!E !E T == !E T`） | 作用域保序序列（`#e #e T != #e T`） |
| **专属消去语法** | 支持前置/后置 `catch`（绑定完整错误结果值） | 支持 `resume expr`（向暂停点交付操作结果 `Y`） |
| **普通 `match` 模式** | 成功分支写 `val! => ...`，错误分支写 `error pat => ...` | 效应不通过普通 `match` 消去，使用 `expr# { ... }` 或 `.use(...)` |

---

## 六、下一期预告

现在，我们已经分别精通了代数效应 `#Effects` 和错误结果 `!Errors`。
但在真实的微型运行时和网络协议栈中，一个函数往往**既需要发出 I/O 或时钟效应请求，又可能在解析数据时返回业务错误**！

比如，一个从虚拟总线读取并解析传感器帧的函数，它的返回类型同时挂着 `#` 和 `!` 两个前缀：
```flurry
fn read_and_parse() -> #[bus_read] !ParseErr Frame
```
当我们面对这种 `#Effects !Errors T`（甚至反过来的 `!Errors #Effects T`）的多层限定组合类型时：
- 为什么在 `#Effects !Errors T` 中，中间的 `!` 不会被语法分析器误认为是作用在 `Effects` 表达式上的后缀错误传播操作符？
- `#Effects !Errors T` 在语义上究竟是怎么一层套一层的？
- 当我们在调用处先用 `#` 还是先用 `!` 时，剥洋葱的顺序是怎样的？

下一期 **《Flurry 演进手记 (150)：当 #Effects !Errors T 组合在一起——限定前缀的嵌套顺序与交互法则》**，我们将迎来本批次的收官之作，用直观的“洋葱模型”彻底打通多层限定类型的组合与消去法则！
