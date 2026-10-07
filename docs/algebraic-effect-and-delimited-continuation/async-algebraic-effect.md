# 异步效应与结构化并发

## Intro

异步效应把 handler 的处理工作交给一个子 task。请求点取得 Future 后继续执行；需要结果时才等待，因此多个请求可以先发出，再按程序需要取得结果。

runtime 实例由使用者选择。它负责安排工作和完成通知，底层挂起与恢复通过代数效应的控制协议完成。

## 语法

~~~text
algebraic_effect -> async? inline? effect id(parameter*) (-> result)? clauses?
async_pattern -> async pattern
effect_handle -> expr# { case_arm* }
async_await -> expr.await
handler_application -> expr.use(expr)
~~~

async 声明与 async 分支相对应。await 是协议接入的后缀形式；本篇使用 .use(runtime) 显式选择运行时实例，由该实例封装并供应栈池。

## 正式定义

### 请求启动工作

声明的操作结果为 Y 时，异步请求交付该次子工作的 Future，子工作完成后产生 Y。Future 的具体实现由协议提供；不因采用这个名称就要求固定数据布局。

请求时已经登记子工作。等待不是启动工作的唯一时刻；丢弃尚未等待的 Future 也不会自动取消工作。所属结构化范围仍负责其完成或取消。

async 分支在子 task 的执行栈上运行。同步分支在当前工作中执行，不另创建异步子 task；显式 continuation 捕获需要的物理栈切换由控制后端另行管理。

### 等待

await 先检查结果是否就绪。就绪时按结果所有权契约交付；未就绪时，通过 runtime 安装的可恢复效应交出当前计算。完成事件使这份恢复权变为可运行，随后由调度器消费它。

Future 被等待多次是否合法由结果交付协议决定，不能默认复制一个只能消费一次的 Y。Future 也不提供复制其等待 continuation 的许可。

### 结构化范围与资源

含 async 分支的安装范围拥有它派生的子工作。正常退出先等待未完成工作；提前退出先发起取消，并等待相关清理完成，再释放子工作依赖的环境。

一个 runtime 可以服务多个安装范围。runtime 对象仍存活，不能代替每个范围对自己的子工作履行等待和清理责任。

runtime 可以在内部封装 stack pool，安装 runtime 时一并供应所需存储；使用者只需指定 runtime 实例。runtime 的调度身份与栈池的存储身份仍分别保留。所有子工作、挂起栈和清理结束前，仍被使用的池及其存储必须有效；等待完成不等于可以提前释放尚被 continuation 持有的栈。

handler 的捕获可能被多个子工作访问。共享、复制、所有权转交或串行执行必须满足实际对象契约；不能因为它们来自同一个 handler，就复制 Once 资源或允许冲突写入。

### 调度边界

调度采用协作方式，只在类型化的效应挂起点交出当前计算。已经挂起的计算在线程内恢复；在其他线程执行新的工作也必须先满足普通资源转交要求。

完成、等待登记与取消必须协调同一份恢复权。取消尚在运行的工作只设置相应请求，并在允许的控制点执行退出；取消已经挂起的工作可以直接交给放弃协议。两者都不能跳过资源清理。

## 常见示例

### 先发出两个请求，再等待

下列使用端片段中的 runtime 是使用者预先创建的运行时实例，内部封装栈池并供应执行存储。运行时库负责实现 task、Future、await 与挂起协议；调用方通过 .use(runtime) 接入这些服务。

~~~flurry
async effect double(value: u32) -> u32;

let total = (do {
    let first = double(20u32)#;
    let second = double(1u32)#;
    println("submitted");
    let left = first.await;
    let right = second.await;
    left + right
}# {
    async double(value) => value * 2u32,
}).use(runtime);

println("{}", total);
~~~

结果为 42。两个请求都在第一次等待前发出；子工作具体何时运行由所选 runtime 决定。源码没有承诺它们在哪个工作线程并行，也没有承诺先发出的工作先完成。

### 异步结果携带错误

~~~flurry
enum DivideErr { Zero, }
async effect divide(numerator: u32, denominator: u32) -> !DivideErr u32;

let value = (do {
    let pending = divide(12u32, 0u32)#;
    let completed = pending.await;
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
~~~

输出 0。await 取得一个 !DivideErr u32，之后才发生普通错误消去。错误没有成为另一种 continuation 信号。

### 未等待的工作仍属于当前范围

~~~flurry
async effect report(value: u32);

(do {
    let pending = report(7u32)#;
    println("body end");
}# {
    async report(value) => println("report: {}", value),
}).use(runtime);

println("scope end");
~~~

pending 的丢弃不取消 report。scope end 在子工作完成和范围清理之后输出；body end 与子工作输出的先后不作额外承诺。

### 同步计算的异步测试处理器

第一个例子的 double 分支可以直接计算乘法作为 mock。它仍写 async，保留子工作归属和 Future 交付契约。删去 async 不表示“自动构造已完成 Future”，而是入口类别不符合该声明。

## 与 Rust / Zig / C / C++ 的关键差异

Rust async 表达式产生 Future，执行由轮询推进；本篇 async effect 在请求时登记子工作，等待协议另行决定如何取得结果。C 与 C++ 可以使用回调、线程或协程库完成类似工作，但这些替代手段不自动提供本篇 handler 安装范围的任务归属。[Rust async blocks](https://doc.rust-lang.org/reference/expressions/block-expr.html#async-blocks)、[C++ 协程](https://eel.is/c++draft/dcl.fct.def.coroutine)

## 运行时实现设计引导

结果槽至少区分未完成、已完成、已取消和结果已交付。等待登记与结果发布必须具有原子交接点；单线程运行时也要处理登记过程中同步完成的重入情况。

~~~text
await(result):
    如果已经完成：按交付契约取出结果
    否则：建立唯一等待权；原子地登记等待或观察到完成
    确认已经登记且需要挂起后：把 continuation 转交等待状态

complete(result, value):
    取得结果槽的发布权
    保存 value，并把唯一等待权交给就绪队列

cancel(work):
    与完成争用同一完成／恢复权
    若取得挂起 continuation：执行放弃和清理
    最终向所属范围报告已经停止
~~~

不得出现“先检查未完成，随后无保护登记”的空隙；否则完成可能发生在中间并丢失唤醒。也不能把已入队的 continuation 再交给取消路径恢复一次。

任务归属、handler 上下文和 continuation 栈链分别保存。子工作继承合法的入口环境，不通过读取父任务随时变化的当前栈取得 handler。栈槽归还其实际来源 pool，不能仅凭当前 runtime 或恢复点的供应决定归还对象。

## 编译器实现引导

1. 区分操作的完成结果 Y 与请求交付的 Future 接口。
2. 检查 async 声明和分支类别，按协议接入选定 runtime。
3. 将分支环境的捕获和参数交付纳入资源检查。
4. 将 await 接到就绪快路径和类型化挂起路径，保留对应允许行。
5. 为安装范围建立正常等待、提前取消与最终清理的出口。
6. 保留错误结果的独立表示与传播，不把取消包进用户错误行。
7. 检查挂起计算的线程约束、依赖存活以及不重复交付恢复权。

编译器不能因为 inline 意向就消去可观察的工作启动、完成关系或安装范围；内联只在保持这些行为时成立。
