# Flurry 演进手记 (146)：只跑一次、跑在尾部、还是暂停保存？——尾恢复（Tail-Resumptive）与单次限定 Continuation

> **系列导读**：在上一期（145）中，我们体验了 `^attrs.effects.direct_passing` 在安装时求值一次、零切栈传递配置的威力。回顾前五期（141–145），无论是直接传值还是普通直接处理，被调主体在发出 `op()#` 请求后，都不需要把剩余的调用栈打包带走。但今天，我们要迈出构建微型运行时最激动人心的一步——当协程调用 `pause()#` 或生成器调用 `yield_number(x)#` 时，我们希望**把从当前暂停点到 Handler 边界之间的“剩余计算（Delimited Continuation）”捕获下来，由我们决定何时恢复**！我们将揭秘 `catch continuation: builtin.Continuation` 的语法契约、尾恢复（Tail-Resumptive）与非尾恢复的区别，以及为什么 Flurry 坚持**单次恢复（One-Shot）**铁律以守护第三卷的资源安全大厦！

---

## 一、代数效应的三级执行阶梯：并不是所有效应都需要“切栈”！

很多刚听说“代数效应与 Continuation”的系统程序员，往往会有一个误区：“只要用了代数效应，是不是每次调用都要分配协程栈、保存寄存器？”

**完全不是！** Flurry 的编译器与 ABI 将效应请求精确划分为**三级按需递增的执行阶梯**——你不使用的能力，绝不为你增加一分一毫的开销：

```text
================================================================================
                    Flurry 代数效应的三级执行阶梯（Zero-Cost Ladder）
================================================================================
  [ 第 1 阶：Direct Passing（安装时求值传值） ]
    声明：^attrs.effects.direct_passing effect limit() -> u32;
    开销：安装时求值 1 次，调用链直接读取隐式值/寄存器（0 函数调用、0 切栈）
            │
            ▼
  [ 第 2 阶：Direct / Tail-Resumptive（同步直接返回 / 尾恢复） ]
    声明：effect read_number() -> u32; （无 catch 参数！）
    开销：等价于通过环境入口做一次普通函数调用，拿到 Y 后原地继续（0 栈捕获！）
            │
            ▼
  [ 第 3 阶：Capturing Delimited Continuation（显式捕获限定延续） ]
    声明：effect yield_number(val: u32, catch continuation: builtin.Continuation);
    能力：允许将请求点到当前 Handler 边界之间的剩余计算打包为单次恢复权！
================================================================================
```

你看，只有当你在 `effect` 声明中**显式写出了 `catch continuation: builtin.Continuation` 参数**，赋予了该操作“捕获许可（Capture Permission）”时，编译器才会启用第三阶的限定 Continuation 机制！

---

## 二、迭代演进 `v1`：声明 `catch` 参数、连接 `.use(pool)` 与多次挂起

让我们看看如何在 Flurry 中声明并使用一个允许捕获剩余计算的效应：

```text
catch_param      -> catch parameter
algebraic_effect -> async? inline? effect id(parameter*) (-> result)? clauses?
```

这里有三条必须牢记的语法与调用规则：
1. **在 `effect` 声明处加 `catch`**：使用 `catch continuation: builtin.Continuation` 声明捕获许可；
2. **在请求处（Perform）不传 `continuation`**：调用者写 `yield_number(value)#` 时，**只需传入普通实参 `value`**，`continuation` 会由底层控制协议自动打包提供！
3. **在 Handler 分支模式中不重复写 `catch`**：在 `expr# { yield_number(value, continuation) => ... }` 的匹配模式里，直接用普通标识符 `continuation`（或通配符 `_`）绑定该恢复权即可，**不要在模式里重复写 `catch` 关键字**！

此外，因为可捕获的边界需要地址稳定的执行栈存储，我们在独立使用同步限定 Continuation 时，只需在 Handler 外层用 **`.use(pool)`** 接入栈池实例（或用 `.use(runtime)` 接入封装了栈池的运行时）。

让我们看一段包含 3 次循环挂起、且带有一个 `Drop` 资源追踪器 `ProducerTrace` 的完整示例：

```flurry
-- 1. 声明带 catch 参数的效应：授予捕获剩余计算的许可！
effect yield_number(value: u32, catch continuation: builtin.Continuation);

struct ProducerTrace {}

impl Drop for ProducerTrace {
    fn drop(*self) -> void {
        println("cleanup producer");
    }
}

fn produce() -> #[yield_number] void {
    -- 在挂起栈帧上创建一个带 Drop 责任的资源对象 trace
    let trace = ProducerTrace {};
    for value in 0u32..3u32 {
        -- 请求者只传普通参数 value；每次挂起都会向 Handler 交出当前的 continuation！
        yield_number(value)#;
    }
    println("finished");
}

test {
    -- 2. 在分支模式中用普通参数名 continuation 接收恢复权，并通过 .use(pool) 供应栈池
    (produce()# {
        yield_number(value, continuation) => {
            println("{}", value);
            -- 消费当前 continuation，尾切换恢复 produce() 继续下一轮循环！
            resume;
        },
    }).use(pool);
}
```

运行这段代码，控制台依次输出：
`0`、`1`、`2`、`finished`、`cleanup producer`！

---

## 三、深度拷问：为什么 Flurry 坚决采用“单次恢复（One-Shot）”而非“多次克隆恢复（Multi-Shot）”？

仔细观察上面 `produce()` 的执行过程：
在 `for` 循环开始前，我们在栈上创建了一个拥有唯一析构责任（`Once` + `Drop`）的局部变量 `let trace = ProducerTrace {};`。随后，循环挂起了 3 次，产生了 3 个先后接力的 `continuation`（记为 $K_1, K_2, K_3$）。

这时候，有些学过早期函数式语言（如 Scheme 的 `call/cc` 或某些纯学术代数效应语言）的读者会问：
“为什么 Flurry 规定 **`continuation` 不可 `Copy`、不可 `Clone`，每次 `resume` 都必须唯一消费掉当前的恢复权（One-Shot）**？如果允许我把 $K_1$ 复制两份，恢复两次，不是能直接用来写回溯搜索吗？”

### Code Cartoon：如果允许“多次恢复（Multi-Shot）”会发生什么内存惨案？

让我们画一下，在一个拥有线性资源（`Once`）和确定性析构（`Drop`）的系统级语言里，如果允许把挂起栈帧 $K_1$ 克隆并恢复两次，会发生什么灭顶之灾：

```text
  [ produce() 的栈帧：持有唯一资源 trace: ProducerTrace (含堆指针或文件句柄) ]
                                  │
                     在第 1 次 yield_number(0)# 处挂起，捕获恢复权 K1
                                  │
           ┌──────────────────────┴──────────────────────┐
           ▼ (第 1 次恢复 K1)                            ▼ (假设允许非法克隆并第 2 次恢复 K1！)
   跑完循环，退出 produce() 作用域                又跑完一次循环，再次退出 produce() 作用域
   执行 trace.drop() ──► 释放底层资源！          再次执行 trace.drop() ──► 💥 Double Free 内存崩溃！
```

看到了吗？！
在没有垃圾回收（GC）、依靠确定性移动（`Once`）与 `Drop` 管理切身系统资源的语言里，**挂起栈帧上存活的局部变量本身就是线性资源**！
如果你把一个暂停的栈帧恢复两次，就等于把栈帧上所有的唯一指针、文件描述符、互斥锁守卫全部非法复制并双重析构（Double Free）了两次！

因此，Flurry 制定了坚如磐石的**单次恢复（One-Shot）契约**：
1. **`builtin.Continuation` 属于严格的 `Once` 线性资源**：它既不实现 `Copy`，也不提供 `Clone`；
2. **一次恢复消费一个令牌**：每次执行 `resume`（或把 `continuation` 移入调度队列）都会**消耗掉当前这一个恢复权**；如果恢复后的计算在后续循环中再次调用 `yield_number(1)#` 挂起，那会产生一个**全新的恢复权 $K_2$**，绝不会复用已经消费掉的 $K_1$！
3. **逃逸受可达性（`&R`）严密保护**：你可以把 `continuation` 移出当前分支（比如放进协程就绪队列里稍后恢复），但编译器会通过第三卷学过的可达性机制，确保它所依赖的栈池 `pool`、Handler 环境与外部借用对象在它完成前始终有效！

---

## 四、尾恢复（Tail-Resumptive `resume`）vs 显式非尾恢复（Non-Tail Resumption）

在捕获分支内部恢复计算时，Flurry 还做了一个极其重要的性能与控制流区分：**尾恢复（Tail Resumption）**与**非尾恢复（Non-Tail Resumption）**。

### 1. 标准 `resume` 语句：保证零残留等待帧的“尾恢复”

当你在捕获分支里写 `resume expr;`（或无返回值的 `resume;`）时，它的语义是：**立即交付操作结果 `Y`，消费当前恢复权，并直接结束当前 Handler 分支**！
因为当前分支在 `resume` 之后没有任何后续代码需要执行，编译器保证将其编译为**尾切换（Tail Switch）**——绝对不会在调用栈上留下一层仅仅为了等待恢复完成而存在的僵尸分支帧！

### 2. 显式非尾恢复：等待主体算出整体结果 `B` 后，再继续加工！

那如果我们确实想在 Handler 分支里“**先把值喂给暂停的主体让它跑完得到结果 `B`，拿到 `B` 之后，还在当前分支里给 `B` 加个 `1` 再返回**”呢？
这就叫**非尾恢复（Non-Tail Resumption）**。在底层控制协议中，可以通过显式的恢复原语（如下例中演示底层契约的 `unsafe fn resume_u32`）来实现：

```flurry
-- 演示底层非尾恢复契约的适配声明：
-- 消费 continuation，向暂停点交付 value: u32 (即 Y)，并等待被处理主体完成返回整体结果 u32 (即 B)！
unsafe fn resume_u32(continuation: builtin.Continuation, value: u32) -> u32;

effect ask(catch continuation: builtin.Continuation) -> u32;

fn twice_answer() -> #[ask] u32 {
    let answer = ask()#;
    return answer * 2u32;
}

test {
    let result = (twice_answer()# {
        ask(continuation) => unsafe {
            -- 1. 非尾恢复：向 ask()# 交付 Y = 6u32，
            --    等待 twice_answer() 跑完 6 * 2 得到主体结果 B = 12u32！
            let resumed = resume_u32(continuation, 6u32);
            -- 2. 主体跑完后回到这里，继续执行后续工作 + 1u32！
            resumed + 1u32
        },
    }).use(pool);

    println("{}", result); -- 输出：13
}
```

请注意：在非尾恢复中，因为分支在恢复之后还有 `+ 1u32` 的后续工作要等 `twice_answer()` 跑完才能做，所以当前分支的栈帧在恢复期间必须保持存活（不能当成尾恢复消去）。

---

## 五、本节核心概念速查表

| 概念 / 语法 | 写法与契约 | 核心设计价值 |
| :--- | :--- | :--- |
| **捕获许可声明** | `effect op(x: T, catch c: builtin.Continuation);` | 显式声明允许捕获从请求点到选定 Handler 边界之间的剩余计算 |
| **请求与匹配写法** | 请求：`op(x)#`；匹配：`op(x, c) => { ... }` | 请求点不传 `c`；分支模式用普通标识符绑定 `c`，**不重复写 `catch`** |
| **单次恢复（One-Shot）** | `builtin.Continuation` 为非 `Copy` 的线性资源 | 每次恢复消耗当前令牌，完美兼容栈上 `Once` 唯一资源与 `Drop`，杜绝双重释放 |
| **尾恢复 `resume`** | `resume expr;` | 交付 `Y` 并立即结束当前分支，保证尾切换，不累积等待帧 |
| **显式栈池接入** | `(expr# { ... }).use(pool)` | 通过处理协议供应地址稳定的有栈存储，无需污染业务函数参数列表 |

---

## 六、下一期预告

在今天第 2 节的 `produce()` 例子里，我们的 Handler 乖乖地对每一次 `yield_number` 都执行了 `resume;`，让 `produce()` 的循环从头跑到尾，最后正常退出并打印了 `cleanup producer`。

但是，请思考一个让无数协程库和生成器实现头疼不已的边界问题：
**如果 Handler 在收到第 1 个值（`0u32`）之后，觉得“我已经拿到想要的数据了，不想再继续往下跑了”，于是直接退出了 Handler 分支，没有调用 `resume`（即放弃 / Abort 了手里的 `continuation`）！**

这时候，被暂停在半空中的 `produce()` 栈帧上，那个还没来得及跑完循环的局部变量 `let trace = ProducerTrace {};` 会怎么样？
- 它会像 C 语言的 `longjmp` 或粗暴释放协程栈那样，**直接被丢弃造成严重的资源与锁泄漏**吗？
- 如果不会泄漏，Flurry 的运行时是如何通过一条**隐藏中止通道（Hidden Abort Channel）**钻回挂起的栈帧里，把尚未完成的局部变量按逆序逐一 `Drop` 干净的？
- 为什么这条隐藏中止通道**绝对不能**伪装成普通的错误 `!Err`（否则万一被内层用户写的 `catch` 错误分支误截获就糟了）？

下一期 **《Flurry 演进手记 (147)：当 Continuation 被中止（Abort）时——跨越挂起栈帧的 Drop 逆序清理保证》**，我们将揭开挂起栈帧安全清理与 C 栈帧屏障的底层奥秘！
