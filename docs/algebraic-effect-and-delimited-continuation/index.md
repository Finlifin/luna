# 代数效应与限定续延

代数效应（Algebraic Effects）是 Flurry 用于统一管理**副作用**和**非局部控制流**的核心机制。它将异步、迭代器、错误传播、状态注入、日志等传统上需要各自独立机制（`async/await`、Monad、`try/catch`）的场景，纳入同一个理论框架。

## 核心概念：请求与处理的分离

代数效应的本质是一个**请求-响应**模型：

- **发出效应（Effect Operation）**：被调方（callee）在执行到需要外部帮助的点时，不直接执行操作，而是向上发出一个带参数的"请求"。
- **处理效应（Effect Handler）**：调用方（caller）安装处理器，拦截该请求，决定如何响应，可以选择：
  - 执行操作后**恢复**（resume）被调方的执行，并传回一个值
  - 不恢复，直接从处理器返回（类似异常）
  - 多次恢复（用于协程、生成器等）

**关注点分离**：被调方只描述"需要什么"，不关心"如何做"；调用方通过安装不同的处理器来决定具体行为。这使代码高度可测试和可组合。

## 声明效应（`effect`）

```flurry
effect ask() -> String;               -- 请求输入，处理器返回 String
effect output(message: String);       -- 输出消息，默认返回 void
effect read_state() -> T where T;     -- 读取类型 T 的状态（泛型效应）
effect write_file(path: String, data: Slice<u8>) -> !IoErr void;
```

每个 `effect` 声明定义一个效应操作的签名：参数类型（发出时携带）和返回类型（处理器恢复时提供）。

## 发出效应

函数通过后缀 `#` 发出效应，并在签名中用 `#[EffectList]` 声明可能发出的效应集：

```flurry
fn user_interaction() -> #[output, ask] String {
    output("What is your name?")#;    -- 发出 output 效应
    let name = ask()#;                -- 发出 ask 效应，接收处理器的返回值
    "Hello, " + name
}
```

## 安装处理器

调用方用 `expr# { ... }` 语法拦截并处理效应：

```flurry
fn run_interaction() {
    let result = user_interaction()# {
        ask() => {
            print("> ");
            io.read_line()   -- 这个值会作为 ask() 的返回值传给被调方
        },
        output(msg) => {
            println("{}", msg);
            -- void 效应：隐式返回 ()
        },
    };
    println("Result: {}", result);
}
```

处理器分支的最后一个表达式作为效应的返回值，恢复被调方执行。

## 效应类型签名

效应在类型系统中是显式的：

```flurry
fn user_interaction() -> #[output, ask] String;
fn pure_add(a: i32, b: i32) -> #[] i32;   -- 无效应，或简写为 -> i32
```

**处理器消融规则**：处理器消解它所处理的效应。若计算的效应集为 B，处理器可处理效应集为 A，则应用处理器后剩余效应集为 B - A：

```flurry
-- run_interaction 本身不发出效应
fn run_interaction() -> #[] void {
    let result = user_interaction()# {   -- 消解了 ask 和 output
        ask()     => { ... },
        output(m) => { ... },
    };
    ...
}
```

## 限定续延（Delimited Continuation）

代数效应基于**限定续延**实现：当效应被发出时，当前计算的剩余部分（从发出点到处理器边界）被捕获为一个续延（continuation）。处理器可以调用这个续延零次、一次或多次：

```flurry
-- 协程/生成器示例
effect yield(value: T) where T;

fn generator() -> #[yield<i32>] void {
    yield(1)#;
    yield(2)#;
    yield(3)#;
}

fn collect_values() -> Vec<i32> {
    let results = Vec.new();
    generator()# {
        yield(v) => {
            results.push(v);
            -- 续延在此被隐式恢复（继续执行 generator）
        },
    };
    results  -- [1, 2, 3]
}
```

## 异步代数效应

Flurry 将异步编程建立在代数效应之上，`suspend` 效应表示异步挂起点：

```flurry
effect suspend() -> T where T;

fn async_task() -> #[suspend<Data>] void {
    let data = async_get_data();
    suspend()#;           -- 挂起，将控制权交给运行时
    process(data);
}
```

运行时（event loop、线程池等）通过安装 `suspend` 的处理器来驱动异步执行，与同步代码在语法上保持一致。

## 预制处理器（Prefabricated Handlers）

可以将效应处理逻辑直接与某个类型关联，通过 `handles` 在 `impl` 中定义：

```flurry
struct StateRuntime where T {
    current_state: T,
}

impl StateRuntime<T> where T:- Clone {
    handles write_state(new_state: T) {
        self.current_state = new_state;
    }
    handles read_state() -> T {
        self.current_state.clone()
    }
}

fn use_state(runtime: StateRuntime<i32>) -> i32 {
    do {
        let s = read_state()#;
        s + 1
    }.use(runtime)   -- 使用 runtime 的预制处理器处理效应
}
```

## 与传统机制的对比

| 场景 | 传统方式 | 代数效应 |
|------|----------|----------|
| 错误处理 | `try/catch`，单向传播 | `effect`，可恢复 |
| 状态 | 手动传递 / Monad | `read_state`/`write_state` 效应 |
| 异步 | `async/await`，专用语法 | `suspend` 效应，统一语法 |
| 生成器 | `yield` 关键字（语言内建） | `yield` 效应，用户可定义 |
| 依赖注入 | 构造函数 / 框架 | 效应 + 处理器 |
