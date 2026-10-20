# Flurry 演进手记 (158)：当闭包捕获了效应 Handler——%c 环境限定、可达性与效应能力的安全传递

在上一篇末尾，我们提出了一个让许多支持代数效应的实验性语言（如早期未加区域限制的效应系统）栽过跟头的经典难题：
假设在一个函数或块内部，我们安装了一个局部效应处理器或局部状态对象，然后在里面创建了一个闭包（Lambda）`|| ...`，最后试图把这个闭包作为返回值带出该块，在外围调用它。

这时候，两个致命的核心问题摆在编译器面前：
1. **效应与局部存储的逃逸危机（Effect / Storage Escape）**：如果闭包内部引用了块内的局部变量指针 `local.ref`，或者依赖了块内已经卸载的局部 Handler 环境，一旦块退出，栈帧已经销毁，这时候在外面调用闭包岂不是当场踩爆悬垂内存？
2. **闭包调用后的环境状态演进（Post-Call Environment State）**：如果闭包捕获了一个只能消费一次的单次资源（比如一张重连凭证 `Ticket` 或一个单次恢复权），或者捕获了两个带有 `Drop` 析构责任的资源 `first` 和 `second`——在 Rust 里要靠 `Fn` / `FnMut` / `FnOnce` 三个僵硬的 Trait 分类来区分，而在 Flurry 中，编译器是怎么不用 `Fn/FnMut/FnOnce` 就能精确追踪每次调用后**哪个环境字段被移走了、哪个字段还活着、退出时按什么顺序逆序 `Drop`** 的？

今天，我们就深入 `docs/functions/closure-environments.md` 与 `handling-capability.md`，揭开 Flurry **`%c` 闭包环境限定、字段级状态追踪与跨边界逃逸拦截**的精妙设计。

---

## 为什么把闭包简写成“黑盒函数指针”会捅出大篓子？

在传统的粗粒度闭包类型设计里，一旦你写了一个闭包 `|| do_something()`，编译器要么把它擦除成一个模糊的 `Fn()` 接口，要么一刀切地把整个闭包环境标成“只能调一次（`FnOnce`）”或“随便调多少次（`Fn`）”。

这种粗粒度模型有三个明显的盲区：
- **看不见局部指针逃逸**：即使闭包的参数列表是空的 `|| -> u32`，如果它的环境 `%c` 里藏着一个指向即将销毁的栈变量 `local` 的指针 `source`，光看 `fn() -> u32` 的表面签名根本发现不了悬垂引用。
- **看不见环境字段的部分消耗**：如果闭包捕获了 `first` 和 `second` 两个对象，调用时只把 `first` 按值移走（Move）了，`second` 还在环境里好好待着——谁来负责在闭包销毁时清理剩下的 `second`，同时确保绝不重复清理已经移走的 `first`？
- **看不见未消去的效应需求**：如果闭包体里发起了 `ask()#` 效应请求，这个效应需求必须反映在闭包主体的效应行上，或者其绑定的有状态处理器环境必须在调用时刻依然存活。

---

## 告别 `Fn/FnMut/FnOnce` 三分法——用 `%c` 环境与字段状态机精确制导

翻开 `docs/functions/closure-environments.md`，你会发现 Flurry 做出了一个比传统 `Fn/FnMut/FnOnce` 更加精准、正交的设计：
**Flurry 不引入 `Fn`、`FnMut`、`FnOnce` 这套三选一的接口作为默认模型，而是直接使用“主体类型 + `%c` 环境限定（如 `%{name: T = value} F`）+ 字段级资源状态机”来刻画闭包的完整生命周期。**

让我们看看同一个 `|| ...` 语法在不同捕获行为下，编译器是如何精确推导它能否再次调用的：

### 1. 读取 `Copy` 捕获与通过指针修改状态——可安全重复调用

```flurry
test {
    -- 情况 A：读取 Copy 捕获 (offset: u32)，原捕获在调用后依然完好可用
    let offset: u32 = 4;
    let add_offset = |value: u32| value + offset;
    println("{}", add_offset(1));   -- 输出 5
    println("{}", add_offset(2));   -- 输出 6

    -- 情况 B：通过指针 remaining 修改外部状态 quota，并可多次调用。
    let quota: u32 = 2;
    let remaining = quota.ref;
    let take_one = || {
        if remaining.* == 0 {
            false
        } else {
            remaining.* -= 1;
            true
        }
    }
    println("{}", take_one());      -- 输出 true (quota 变为 1)
    println("{}", take_one());      -- 输出 true (quota 变为 0)
    println("{}", take_one());      -- 输出 false
    println("remaining {}", quota); -- 输出 remaining 0
}
```

### 2. 交出非 `Copy` 捕获——第一次调用移出字段，第二次调用当场编译期拦截

如果闭包把捕获的非 `Copy` 单次资源 `ticket: Ticket` 按值交给了 `finish(ticket)`，编译器在内部环境状态机中会将 `ticket` 字段的状态从 **`持有有效值（Held）`** 推进为 **`已移出（Moved Out）`**。
此时如果你试图调用第二次 `submit()`，编译器根本不会被“参数表是空的 `()`”骗过去，而是直接检查环境状态并报错拦截：

```flurry
struct Ticket {
    number: u32,
}

fn finish(ticket: Ticket) -> u32 {
    ticket.number
}

fn repeated_consumption() {
    let ticket = Ticket { number: 8 }
    let submit = || finish(ticket);

    -- ✅ 第一次调用：将环境中的 ticket 按值移交消费，输出 8
    println("{}", submit());

    -- ❌ 编译期直接拒绝。
    -- 第一次调用后，闭包环境中的 `ticket` 字段状态已变为“已移出（Moved Out）”。
    -- 第二次调用缺少所需的资源责任，严禁重复消费已移出的环境字段。
    println("{}", submit());
}
```

---

## 首次词法出现顺序与逆序精确清理（Reverse Drop Order）

如果一个闭包捕获了多个带有 `Drop` 清理责任的对象（比如 `first` 和 `second`），当闭包离开作用域（或者只消费了其中一个字段）时，编译器按什么顺序析构它们？

`docs/functions/closure-environments.md` 给出了毫不含糊的确定性规则：
1. **按首次出现的词法次序建立语义字段**：编译器扫描闭包体，对捕获去重后，按它们在闭包体中**第一次出现的先后顺序**排列语义字段；
2. **退出时仅对尚留在环境中的责任执行逆序清理**：如果某个字段在调用过程中已经被移走（如 `consume(first)`），它就不再由闭包环境清理；而仍然留在环境里的字段（如 `second`），则在闭包生命周期结束时按**逆序**精确清理。

下面对照验证官方规范中的两个精彩示例：

```
+-------------------------------------------------------------------------+
|  闭包环境的语义字段次序、部分消费与逆序 Drop 清理              |
|                                                                         |
|  外部声明：let first = Trace { id: 1 };  let second = Trace { id: 2 };  |
|                                                                         |
|  【案例 1：未调用即离开作用域】                                         |
|    let action = || {                                                    |
|        println("{}", second.inspect());  ──► 首次出现 #1: second        |
|        println("{}", first.inspect());   ──► 首次出现 #2: first         |
|    }                                                                    |
|    • 语义字段次序：[ second, first ]                                    |
|    • 离开作用域逆序清理：先 Drop `first` ("drop 1")，                   |
|                          再 Drop `second` ("drop 2")                    |
|                                                                         |
|  【案例 2：调用时消费了 first，仅剩 second 留在环境中】                 |
|    let action = || {                                                    |
|        let value = consume(first);       ──► `first` 被移交并在         |
|        println("still {}", second.inspect());  `consume` 结束时立即清理 |
|        value                                     ("drop 1")             |
|    }                                                                    |
|    • 调用 `action()` 时输出：                                           |
|      `drop 1` ──► `still 2` ──► `result 1`                              |
|    • 离开作用域清理剩余环境字段 `[ second ]`：                          |
|      仅输出 `drop 2`  （绝不重复 Drop 已经移出的 `first`  ）✅          |
+-------------------------------------------------------------------------+
```

```flurry
struct Trace {
    id: u32,

    fn inspect(*self) -> u32 {
        self.id.*
    }
}

impl Drop for Trace {
    fn drop(*self) {
        println("drop {}", self.id.*);
    }
}

fn consume(resource: Trace) -> u32 {
    resource.id
}

fn run_once() {
    let first = Trace { id: 1 }
    let second = Trace { id: 2 }
    let action = || {
        let value = consume(first);
        println("still {}", second.inspect());
        value
    }
    println("result {}", action());
}
-- 运行 run_once() 依次输出：drop 1、still 2、result 1、drop 2。
```

---

## 拦截跨边界悬垂逃逸——当闭包试图带走局部指针或已卸载的 Handler 环境

现在，让我们回到最关键的**跨边界逃逸检查（Escape Checking）**。
当一个闭包从 `do { ... }` 块或 `# { ... }` 效应安装块中作为返回值逃逸到外层时：
1. **如果闭包按值捕获了 `Copy` 数值（如 `|| local`）**，并且它请求的效应行（如 `#[next_number]`）留待外层调用点供应：那么闭包环境自己拥有那份 `u32` 副本，不依赖块内即将销毁的栈槽地址，**合法放行**。
2. **如果闭包捕获了指向块内局部对象（包括块内局部有状态 Handler 对象 `counter.ref` 或局部变量 `local.ref`）的指针**，企图跨出该局部对象的存活范围：哪怕 `%c` 环境类型把指针记录得清清楚楚，**编译器也会在逃逸边界编译期拒绝。**

来看 `docs/functions/closure-environments.md` 中的这组正反对比：

```flurry
-- ❌ 编译期直接拒绝。
-- `action` 捕获了指向块内局部变量 `local` 的指针 `source`，
-- 当 `do` 块结束时，`local` 的栈存储生命周期已经终结，闭包严禁携带悬垂指针逃逸。
test {
    let action = do {
        let local: u32 = 7;
        let source = local.ref;
        || source.*
    }
    println("{}", action());
}

-- ✅ 合法通过。输出 7
-- 环境按值拥有 `u32` 的副本，未保存指向块内 `local` 栈槽的地址。
test {
    let action = do {
        let local: u32 = 7;
        || local
    }
    println("{}", action());
}
```

同理，如果我们在一个函数里写了一个发起效应请求的闭包 `|| next_number()# + offset`，它的调用契约会诚实地保留 `#[next_number]` 效应行——只有在调用点外围安装了有效的 `next_number` Handler 时才能调用，绝不可能让你在一个已经卸载销毁的死 Handler 栈帧上发起悬空调用。

---

## 本篇小结

| 维度 | Flurry 闭包与效应环境机制 | 工程与安全优势 |
| :--- | :--- | :--- |
| **环境限定 `%c F`** | `%{name: T = value} F`（空环境自动消去） | 无需 `Fn/FnMut/FnOnce` 僵硬分类，直接在类型与验证层追踪环境绑定与身份 |
| **调用后字段状态演进** | 逐字段追踪 `未取得 -> 持有 -> 已移出 -> 已终结` | 允许指针修改后重复调用，同时在编译期精准拦截对已 Move 字段的二次调用 |
| **首次词法序与逆序清理** | 按闭包体内首次出现的词法顺序建表，退出时对未移出字段逆序 `Drop` | 即使发生部分消费（Move 走部分捕获），也能 100% 保证剩余资源不漏删、不重删 |
| **逃逸与可达性边界** | 跨出 `do` 块、函数或 Handler 范围时核验全部捕获指针与环境的存活期 | 彻底杜绝闭包携带局部变量指针或已卸载的局部 Handler 状态逃逸 |

---

### 下一篇预告

激动人心的时刻再次来临。
在第七卷的前面章节中，我们已经集齐了控制流大一统的全部核心拼图：
- `effect` / `effect enum` 与单次限定 Continuation（`catch continuation: builtin.Continuation`、`resume`）；
- `async effect`、`.await` 与结构化并发（正常退出等待完成、提前退出取消并等待 `Drop` 清理）；
- 效应 ABI、证据传递与 `.use(pool)` / `.use(runtime)` 池化执行后端；
- `atomic(vars) { ... }` 软件事务内存（STM）与“Commit-Then-Act”纯度/效应隔离模式；
- 有状态 Handler 仿真与 `%c` 闭包安全捕获。

在接下来的**第七卷双篇里程碑（Episodes 159–160）**中，我们将展开一场硬核的综合实战：
- 在上半部 **`159-milestone-part1-custom-async-executor-with-effects.md`** 中，我们将利用限定 Continuation、任务状态机与结构化清理协议，手把手构建一个支持**协作式挂起（Yield）、异步子工作等待（Await）与超时取消清理**的轻量级调度执行器内核。
- 在下半部 **`160-milestone-part2-fault-injection-on-sensor-service.md`** 中，我们将在这个调度内核之上运行起一套结合了 **STM 事务聚合与异步遥测上报**的网关服务，并用确定性故障注入 Handler 完成混沌测试（Chaos Testing）。

让我们立刻进入里程碑上半场。
