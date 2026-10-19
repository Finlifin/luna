# Flurry 演进手记 (151)：告别“红蓝函数染色”——async effect、async { ... } 块与 .await 的效应本质

在软件工程界，Bob Nystrom 那篇著名的《What Color is Your Function?（你的函数是什么颜色？）》道出了现代主流语言（JS、Python、Rust、C#）异步编程最深沉的痛：
一旦你在语言里引入了 `async fn`（红色函数）与普通 `fn`（蓝色函数）的二元对立，你的整个代码库就会被劈成两半：
1. **调用传染与生态割裂**：蓝色函数不能直接调红色函数；标准库的高阶函数 `map`、`filter` 或者 हमने在第五卷刚写好的通用编解码器 `Encode`，被迫纠结到底是写成同步版还是 `async` 版；
2. **执行模型僵化**：在许多语言里，调一个 `async fn` 只是惰性地造出一个冷冰冰的状态机结构体，不 `.await` 就连任务都没启动；或者反过来，一旦启动就跟某个全局死板的运行时强行绑死。

而在 Flurry 中，当你翻开 `docs/algebraic-effect-and-delimited-continuation/async-algebraic-effect.md` 时，你会发现一个令人豁然开朗的设计：
**Flurry 根本没有把“异步”做成一套与普通函数对立的染色状态机，而是把它建立在代数效应（Algebraic Effects）之上——即 `async effect`！**

在我们的边缘物联网网关中，我们需要同时向两台远程 Modbus 传感器发起异步计算/采集请求，等两个请求都发出（Submitted）之后，再分别 `.await` 收集它们的结果（包含可能的除零或校验错误）。
今天，我们就来看看 `async effect`、`async` 处理器分支、`.await` 与 `.use(runtime)` 是如何联手终结“函数染色”噩梦的！

---

## 尝试 v0：传统异步里的“串行假并发”与状态机割裂

在很多传统异步写法里，新手想并发发起两个请求，往往会不小心写出这样的“伪并发”代码：

```
-- 💭 传统异步模型的常见痛点：
-- 如果每次调用都必须立刻 await，那 second 必须等 first 完全跑完才开始！
let left = fetch_first().await;
let right = fetch_second().await;
```
为了同时启动两个请求，你又得去引入 `join!` 宏、`spawn` 全局任务或者复杂的 `FuturesUnordered` 组合子。更麻烦的是，如果你在单元测试里想把这个异步操作换成同步内存计算，由于函数签名已经被染成了 `async fn`，你还是得启动一整套笨重的异步 Executor 才能跑通测试！

---

## 演进 v1：`async effect`——请求即登记子工作，按需 `.await` 交付结果！

在 Flurry 中，异步操作就是一种声明了 `async` 修饰符的代数效应：

```flurry
async effect double(value: u32) -> u32;
```

请仔细观察这行声明的语义契约（来自 `docs/algebraic-effect-and-delimited-continuation/async-algebraic-effect.md`）：
1. **请求点交付 `Future` 并立即启动子工作**：
   虽然 `double` 声明的操作完成结果类型是 `u32`，但当你在代码中通过后缀 `#` 发起效应请求 **`let first = double(20u32)#;`** 时，Handler 的处理工作会被交给一个**异步子任务（Child Task）**！
   请求点**在发出请求的那一刻就已经登记了子工作**，并立即拿到代表该子工作的 `Future` 对象继续往下走，**不需要等到 `.await` 才启动工作**！
2. **处理端使用 `async pattern => ...` 分支并接入 `.use(runtime)`**：
   在 `# { ... }` 处理块中，用 `async double(value) => ...` 声明该分支在子任务的执行栈上运行，并在外层通过 **`.use(runtime)`** 显式接入你选择的运行时实例（由该 `runtime` 封装并供应底层栈池与调度器）！
3. **等待端使用后缀 `.await`**：
   当你真正需要那个 `u32` 结果时，写下 `first.await`。如果子工作已经就绪，立即走快路径取出结果；如果尚未就绪，则通过 `runtime` 安装的可恢复效应优雅地交出当前计算（挂起），等完成事件到达后再恢复！

```
+-------------------------------------------------------------------------+
|  🎨 Code Cartoon：async effect 的“请求即启动”与 .await 协作挂起         |
|                                                                         |
|  主计算流 (Parent Work)                      子任务栈 (Child Tasks)     |
|  ──────────────────────                      ──────────────────────     |
|  1. let first = double(20u32)#;  ─────────►  [Task 1 已登记: 20 * 2]    |
|     (立即取得 Future `first` 并继续！)              │                   |
|                                                     │                   |
|  2. let second = double(1u32)#;  ─────────►  [Task 2 已登记: 1 * 2]     |
|     (立即取得 Future `second` 并继续！)             │                   |
|                                                     ▼                   |
|  3. println("submitted");                     (由所选 runtime 安排执行) |
|                                                     │                   |
|  4. let left = first.await;      ◄──────────────────┘                   |
|     • 若已就绪：直接按契约取出 40u32                                    |
|     • 若未就绪：通过代数效应交出当前计算，完成后唤醒恢复！              |
|                                                                         |
|  5. let right = second.await;    ◄────────────────── [交付 2u32]        |
|  6. left + right  ===>  42u32                                           |
+-------------------------------------------------------------------------+
```

让我们把这段来自官方规范（`docs/algebraic-effect-and-delimited-continuation/async-algebraic-effect.md`）的标准示例完整写出来：

```flurry
async effect double(value: u32) -> u32;

let total = (do {
    -- 先连续发出两个异步效应请求，各自立即登记子工作并取得 Future！
    let first = double(20u32)#;
    let second = double(1u32)#;
    println("submitted");

    -- 需要结果时才分别 .await 等待
    let left = first.await;
    let right = second.await;
    left + right
}# {
    -- async 分支：在子 task 的执行栈上运行
    async double(value) => value * 2u32,
}).use(runtime);

println("{}", total);
```
运行这段代码，两个请求都在第一次 `.await` 之前就已经发出并打印了 `"submitted"`，最终输出 `42`！

---

## 演进 v2：当 `async effect` 遇上 `!Error` 错误行——正交组合，绝不搅成一锅粥！

如果异步子任务在执行过程中可能失败（比如远程校准计算遇到除零错误 `DivideErr.Zero`），在 Flurry 中会发生什么？
错误会不会像某些语言的 `Promise.reject` 那样变成另一种破坏控制流的特殊异常信号？

**绝不会！** 在 Flurry 中，**异步效应（`async effect`）与错误行（`!Err`）是完全正交的**：
- 当你声明 `async effect divide(numerator: u32, denominator: u32) -> !DivideErr u32;` 时，子任务的完成结果类型 `Y` 就是 `!DivideErr u32`；
- 当你对 `pending` 调用 **`pending.await`** 时，`.await` 交付出来的就是一个普通的错误限定值 `!DivideErr u32`；
- 随后，你直接用熟悉的 `!` 错误传播或 `completed! { .Zero => ... }` 错误消去语法处理它即可！

```flurry
enum DivideErr { Zero, }

async effect divide(numerator: u32, denominator: u32) -> !DivideErr u32;

let value = (do {
    -- 1. 发起异步除法请求，取得 Future `pending`
    let pending = divide(12u32, 0u32)#;

    -- 2. .await 等待子工作完成，取得结果 `completed : !DivideErr u32`
    let completed = pending.await;

    -- 3. 使用普通错误消去语法 `! { ... }` 处理 DivideErr！
    completed! { .Zero => 0u32, }
}# {
    async divide(numerator, denominator) => {
        if denominator == 0u32 {
            error DivideErr.Zero
        } else {
            numerator / denominator
        }
    },
}).use(runtime);

println("{}", value);
```

运行结果输出 `0`！你看，异步等待（`.await`）只管把子任务算出来的 `!DivideErr u32` 交到你手上，错误处理依然走纯粹的 `!` 错误通道，两者井水不犯河水！

---

## 编译期安全红线：`async effect` 必须匹配 `async` 处理分支，且 `Future` 不可违约重复消费！

在使用 `async effect` 时，有两条核心编译期防线需要牢记：

### 红线 1：删掉处理器分支上的 `async` 关键字，绝不等于“同步返回一个已完成 Future”！
有些同学在写单元测试 Mock 时可能会想：“既然我在测试里只是算个 `value * 2u32`，我能不能把 Handler 分支里的 `async double(value) => ...` 前面的 `async` 删掉，写成普通同步分支？”

**不行！** `docs/algebraic-effect-and-delimited-continuation/async-algebraic-effect.md` 明确指出：
> **`async` 声明与 `async` 分支相对应……删去 `async` 不表示“自动构造已完成 `Future`”，而是入口类别不符合该声明！**
即使你在测试里只做简单计算，也仍须写 `async double(value) => value * 2u32`，以保留子工作归属与 `Future` 交付契约！

```flurry
async effect double(value: u32) -> u32;

fn bad_missing_async_arm() {
    let total = (do {
        let f = double(10u32)#;
        f.await
    }# {
        -- ❌ 编译期直接拒绝！
        -- `double` 被声明为 `async effect`，请求点期望交付 Future 并登记子工作，
        -- 处理分支缺少 `async` 修饰导致入口 ABI 类别与声明不匹配！
        double(value) => value * 2u32,
    }).use(runtime);
}
```

### 红线 2：不能默认复制一个只能消费一次（`Once`）的异步结果！
当 `first.await` 交付的结果类型 `Y` 是一个只允许消费一次的仿射资源（非 `Copy` 的拥有权对象）时，第一次 `.await` 就已经把结果的所有权移交给了调用者；此时如果你试图对同一个 `first` 再次 `.await`，或者试图复制其内部的等待 continuation，编译器会依据结果交付协议与仿射所有权规则直接拦截！

---

## 本篇小结

| 特性 / 语法 | 形式与规则 | 核心设计优势 |
| :--- | :--- | :--- |
| **异步效应声明** | `async effect op(params...) -> Y;` | 将异步操作纳入代数效应行 `#[op]`，无需制造与普通 `fn` 割裂的“红蓝染色”体系 |
| **发起异步请求** | `let fut = op(args...)#;` | 请求时立即向所选 `runtime` 登记子工作并返回 `Future`，无需等到 `.await` 才启动 |
| **异步处理分支与运行时** | `expr# { async op(...) => ... }.use(runtime)` | `async` 分支在子 task 栈上运行；由 `.use(runtime)` 显式供应调度器与栈池存储 |
| **等待与错误正交** | `let res = fut.await;`<br>`res! { .Err => ... }` | `.await` 就绪走快路径、未就绪通过可恢复效应挂起；错误结果 `!Err Y` 在 `.await` 后按普通错误消去处理 |

---

### 留给下一篇的悬念

仔细回想一下刚才的过程：
既然 `let pending = report(7u32)#;` **在发出请求的那一刻就已经启动了后台子任务**——
那如果我们在函数体里发出了 `let pending = report(7u32)#;`，然后**根本没有写 `pending.await`**，就直接走到了 `# { async report(...) => ... }` 作用域的末尾，会发生什么？！

1. 在传统语言的 `spawn` / `go func()` 里，这个没人等待的后台任务会变成一个脱离父作用域监管的**“孤儿任务（Orphan / Detached Task）”**——如果它里面借用了父函数栈上的局部变量，父函数一退出，孤儿任务立刻踩爆已释放的栈内存（Use-After-Free）！
2. 或者反过来，假设有人以为“只要我把 `pending` 这个 `Future` 变量丢弃掉，后台任务就会自动取消”——真的是这样吗？
3. 如果外层块因为遇到了 `error` 提前异常退出了，那些还在跑的兄弟子任务又该如何安全取消并完成 `Drop` 析构清理？

下一篇 **`152-structured-concurrency-and-cancellation-safety.md`**，我们将深入拆解 Flurry 异步效应的灵魂安全护栏：**结构化并发（Structured Concurrency）、作用域任务归属与取消清理协议**！
