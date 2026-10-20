# Flurry 演进手记 (137)：用 template test 为生成的自动机编写全覆盖性质测试

在上一回（136）中，我们用 CMTT 把 Mini-BPF 过滤规则树和状态转移表编译成了零解释器开销的原生代码。

然而，在把这套自动机生成引擎部署到生产环境的传感器网关之前，质量保障（QA）和形式化验证团队向我们灵魂发问：
1. 当我们测试十几个由 CMTT 生成的不同协议匹配器时，每个测试都需要先初始化测试统计计数器 `counter: *u32`、挂载带 `Drop` 析构日志的追踪器 `RequestTrace`、准备测试夹具（Fixture），然后才跑测试主体——怎么在不复制粘贴任何样板代码的前提下复用这套测试上下文？
2. 更重要的是，在复用测试上下文时，怎么从类型系统层面保证：**每个测试实例都拥有自己独立初始化的上下文状态（绝不跨测试串扰），且即使测试正文提前返回，夹具的资源清理（`Drop`）也 100% 自动执行**？

今天，我们就来解锁 Flurry 函数与测试体系中的高阶复用利器——**`template fn` 与 `template test`（`docs/functions/templates.md`）**，为生成的自动机打造一套全覆盖性质测试（Property Testing）靶场。

---

## 一、 问题背景与设计思路：为什么传统的 `setup/teardown` 或宏拼接测试不够好？

在其他语言的测试框架里，为了在多个测试用例之间复用初始化和清理逻辑，通常只有两种笨办法：

```
  [ ❌ 传统方案 方案 A：面向对象的 beforeEach / afterEach 全局可变成员 ]
  • 问题背景：测试夹具的变量不得不声明成类成员变量（往往还要套 Option 或未初始化默认值），
          一旦并行跑测试极易发生状态污染。

  [ ❌ 传统方案 方案 B：用不卫生的文本宏把 setup 和 teardown 粘在测试体前后 ]
  define run_my_test(body) {
      let counter = ...;
      $body
      cleanup(); -- ❌ 致命漏洞：如果 $body 内部提前 return，这行 cleanup() 就被跳过了。
  }
```

而在 Flurry 中，**测试模板（`template test`）** 和 **函数模板（`template fn`）** 直接建立在 **CMTT 上下文模态类型（`< Ψ |- T >`）** 的坚实地基之上。

让我们用一张图看清 `template test` 与 `test ... use ...` 的协作模型：

```
+-------------------------------------------------------------------------+
|               Flurry `template test` 的上下文授予与生命周期                |
+-------------------------------------------------------------------------+
|                                                                         |
|  [ 模板定义端：template test with_automaton_harness ]                    |
|  where body: < counter: *u32 |- void >                                  |
|  {                                                                      |
|      let trace = RequestTrace { id: 1 };   <-- 1. 建立局部 Drop 守护资源 |
|      var value: u32 = 0;                   <-- 2. 每个测试实例独立初始化 |
|      let counter = value.ref;              <-- 3. 准备公开的上下文绑定   |
|      ~<body>;                              <-- 4. 卫生接入测试实例正文   |
|  }                                         <-- 5. 沿真实控制路径触发 Drop|
|         ▲                                                               |
|         │ 仅暴露 `< counter: *u32 |- void >`，内部私有变量 `trace` 不外泄  |
|  [ 测试实例端：test accepts_valid_frames use with_automaton_harness ]   |
|  {                                                                      |
|      counter.* += 1;  -- 直接使用模板授予的 counter 绑定                  |
|  }                                                                      |
+-------------------------------------------------------------------------+
```

---

## 二、 核心语法：`template fn` 与 `template test` 的两层复用能力

在 `docs/functions/templates.md` 中，模板分为由浅入深的两个层次：

### 层次 1：纯签名抽象（Signature Abstraction）
当多个函数（无论是普通 `fn`、`inline fn` 还是 `quote fn`）需要严格共享同一份参数签名、返回类型和 `where ensures` 后置契约时，可以先声明一个以分号结尾的无主体签名模板：
```flurry
template fn bounded(value: u32, limit: u32) -> result: u32
where
    ensures result <= limit;
```
然后用 `fn clamp use bounded { ... }` 或 `inline fn zero use bounded { ... }` 实例化。每个实例自动继承接口中的参数名 `value`、`limit` 与 `ensures` 契约，同时保持各自的函数类别（普通函数仍是普通函数，`quote fn` 仍保证展开）。

### 层次 2：基于 CMTT `quote lambda` 的正文上下文复用（Body Reuse）
当模板不仅要共享签名，还要先在内部准备一套局部运行期环境，再把用户正文接进来时：
- **`template fn`** 使用 `quote lambda body: < Ψ |- T >` 参数接收实例正文；
- **`template test`** 使用唯一一个锚定子句参数 `where body: < Ψ |- void >` 绑定测试正文，并在模板体内通过 `~<body>` 接入。

---

## 三、 实战：用 CMTT 组装正则自动机，并用 `template test` 批量验收

让我们把 `docs/cmtt/program-construction.md` 中的有限状态自动机（识别正则语言 `a[bc]*d`）与 `docs/functions/templates.md` 中的 `template test` 结合起来。

在这个自动机中：
- 状态 `0` 遇到 `'a'` (`97u8`) 转移到状态 `1`；
- 状态 `1` 遇到 `'b'` (`98u8`) 或 `'c'` (`99u8`) 循环自转移回状态 `1`；
- 状态 `1` 遇到 `'d'` (`100u8`) 转移到唯一接受状态 `2`。

我们用 `template test` 建立一个带计数器 `passed_cases: *u32` 的自动机验收靶场：

```flurry
struct Transition {
    source: u32,
    byte: u8,
    target: u32,
}

-- 使用 CMTT 单洞链将状态转移表编译为 < state: u32, byte: u8 |- ?u32 >
pure comptime fn build_step(
    edges: Array<Transition, n>,
) -> < state: u32, byte: u8 |- ?u32 >
where
    n: usize
{
    var code: < state: u32, byte: u8 |- ?u32 > = '(null);
    var remaining = n;
    while remaining > 0usize {
        remaining -= 1usize;
        const edge = edges[remaining];
        const previous = code;
        code = '(if state == edge.source and byte == edge.byte {
            edge.target
        } else {
            ~<previous>
        });
    }
    code
}

fn step(state: u32, byte: u8) -> ?u32 {
    const edges: Array<Transition, 4> = [
        Transition { source: 0u32, byte: 97u8, target: 1u32 },
        Transition { source: 1u32, byte: 98u8, target: 1u32 },
        Transition { source: 1u32, byte: 99u8, target: 1u32 },
        Transition { source: 1u32, byte: 100u8, target: 2u32 },
    ];
    const dispatch = build_step(edges);
    ~[state = state, byte = byte]<dispatch>
}

fn matches_input(input: Array<u8, n>) -> bool
where
    n: usize
{
    var position = 0usize;
    var state = 0u32;
    while position < n {
        let next? = step(state, input[position]) else { return false; }
        state = next;
        position += 1usize;
    }
    state == 2u32
}

-- 定义测试模板：为每个测试实例独立准备计数器上下文 < counter: *u32 |- void >
template test with_automaton_counter
where
    body: < counter: *u32 |- void >
{
    var value: u32 = 0;
    let counter = value.ref;
    ~<body>;
    assumes value > 0u32;
}

-- 测试实例 1：验证正例性质（匹配 a[bc]*d 的输入必须全部接受）
test automaton_accepts_valid_inputs use with_automaton_counter {
    -- "ad" 与 "abcd" 均应被接受
    assumes matches_input([97u8, 100u8]);
    counter.* += 1u32;

    assumes matches_input([97u8, 98u8, 99u8, 100u8]);
    counter.* += 1u32;
}

-- 测试实例 2：验证拒绝例性质（未到达接受状态或多余后缀必须拒绝）
-- 注意：它的 counter 初值依然是独立的 0，绝不承接前一个测试的状态。
test automaton_rejects_invalid_inputs use with_automaton_counter {
    -- "abc"（未以 d 结尾，停在状态 1）与 "add"（状态 2 无出边）均应被拒绝
    assumes not matches_input([97u8, 98u8, 99u8]);
    counter.* += 1u32;

    assumes not matches_input([97u8, 100u8, 100u8]);
    counter.* += 1u32;
}
```

---

## 四、 进阶：退出清理责任（`Drop`）与上下文隔离边界

在使用 `template fn` 和 `template test` 时，有两条极其重要的语义规则（`docs/functions/templates.md`）：

### 4.1 规则一：清理责任靠 `Drop` 资源协议，不靠“写在 `~<body>` 后面的普通语句”

很多写过宏的同学会误以为：“我在模板里把一句 `println("cleanup")` 写在 `~<body>;` 后面，那不管 `body` 怎么退出，这句 `println` 都会像 `finally` 一样执行吧？”
**错。** 模板里的普通后续语句只是正常控制流的一部分；如果 `body` 提前 `return` 或传播了空值 `request?`，后面的普通语句自然不会执行。
真正需要在**所有退出路径（包括提前返回和失败分支）**都保证执行的清理工作，必须交给实现了 **`Drop`** 的局部守护对象（如 `RequestTrace`）。

### 4.2 规则二：正文只能访问 `< Ψ |- T >` 公开声明的绑定，禁止访问模板私有变量

在 `checked_request` 模板中，模板内部声明了一个私有局部变量 `trace = RequestTrace { id: 1 }`，而向正文公开的接口是 `quote lambda body: < number: u32 |- u32 >`。
这意味着：正文只获得了 `number` 的访问权，**模板内部未列入 `< ... |- ... >` 接口的局部变量 `trace` 对正文是完全不可见的**。

下面我们同时验证 `RequestTrace` 的清理行为与上下文隔离边界：

```flurry
struct RequestTrace {
    id: u32,
}

impl Drop for RequestTrace {
    fn drop(*self) {
        -- 无论模板实例从成功路径还是 request? 提前失败路径退出，Drop 都保证执行。
        let _ = self.id.*;
    }
}

template fn checked_request(
    request: ?u32,
    quote lambda body: < number: u32 |- u32 >,
) -> ?u32 {
    let trace = RequestTrace { id: 1 }
    let number = request?;
    ~<body>
}

fn halve_request use checked_request {
    number / 2u32
}

fn clamp_request use checked_request {
    if number > 10u32 {
        10u32
    } else {
        number
    }
}

test {
    assumes halve_request(8u32) == 4u32;
    -- 当传入 null 时，let number = request? 直接提前返回 null，
    -- 正文不执行，但局部对象 trace 依然沿退出路径安全完成 Drop 清理。
    assumes clamp_request(null) == null;
}
```

如果有人试图在 `use checked_request` 的正文里访问模板私有的局部变量 `trace`：

```flurry
-- ❌ 编译期直接拒绝：trace 是模板 checked_request 的内部私有变量，
-- 并未包含在正文的公开上下文接口 < number: u32 |- u32 > 中。
fn inspect_template_local use checked_request {
    trace.id
}
```

编译器会毫不含糊地报错拒绝：基于 CMTT 的模板复用绝不是不卫生的文本拼接，未在 `< Ψ |- T >` 接口中明确授予的绑定，任何实例正文都无权染指。

---

## 五、 本篇小结

| 模板构造 | 语法形式 | 核心机制与安全契约 |
| :--- | :--- | :--- |
| **纯签名模板** | `template fn id(...) -> R;` | 多个 `fn` / `inline fn` / `quote fn` 实例共享参数、返回值与契约，保留各自函数类别 |
| **带正文上下文模板** | `template fn id(..., quote lambda body: < Ψ \|- T >)` | 模板先准备局部环境并授予绑定 $\Psi$，实例在定义时绑定正文，调用时无需再传 `body` |
| **测试模板** | `template test id where body: < Ψ \|- void > { ... }` | 每个 `test name use id` 独立运行初始化，不跨测试污染状态 |
| **退出清理保障** | 局部 `Drop` 守护对象（如 `RequestTrace`） | 提前返回或 `?` 失败路径由资源协议负责清理，不把 `~<body>` 后的普通语句当成 `finally` |
| **上下文最小暴露** | 仅暴露 `< Ψ \|- T >` 声明的绑定 | 模板私有局部变量对实例正文严格隐藏，彻底杜绝名字意外捕获 |

---

## 六、 下一篇预告

在上面的 `checked_request` 和 `traced_test` 中，我们已经看到了 CMTT 生成代码与 `Drop` 资源清理的初步配合。

现在，让我们把难度推向极限——如果**被 Quote 的代码片段本身直接操作了第三卷（041–065）里的仿射资源（`Once` 唯一所有权值）或带有可达性限定（`&r T`）的局部指针**呢？
试想以下两个惊心动魄的场景：
1. 你有一份开放代码 `const consume_buf: < buf: PacketBuffer |- void > = '(drop_buffer(buf));`，它消费了上下文里的唯一资源 `buf`（不具有 `Copy` 能力）。如果某个生成器不小心把这份代码在同一个函数里 **Splice 接入了两次**，会发生可怕的双重释放（Double-Free）吗？
2. 如果你复制了一份代码值 `const alias_code = consume_buf;`，“复制编译期代码值”会复制运行期的 `PacketBuffer` 资源吗？

下一回 **《Flurry 演进手记 (138)：当 CMTT 遇到资源管理——保证生成代码中的仿射值与指针绝不泄漏》**，我们将揭秘 CMTT 是如何与第三卷的所有权、移动检查和可达性系统天衣无缝地联动的。
