# 限定 continuation 与单次恢复

## Intro

continuation 保存从请求点到指定处理边界之间尚未完成的计算。handler 可以立刻恢复它，也可以把恢复权转交给其他代码，在依赖仍然有效时继续执行。

每次恢复消费一次恢复权。恢复后的计算如果再次挂起，会交出一个新的 continuation。

## 语法

~~~text
catch_param -> catch parameter
algebraic_effect -> async? inline? effect id(parameter*) (-> result)? clauses?
case_arm -> pattern => (block | statement)
resume_statement -> resume expr? (while expr)?
~~~

catch 参数使用 builtin.Continuation。处理分支在对应位置使用普通参数模式；不再在分支模式中重复 catch 修饰。

## 正式定义

### 捕获许可与使用

声明中的 catch 参数许可捕获剩余计算。请求者只供应普通操作实参；continuation 由控制协议提供。

直接分支不取得该专用参数位置，按操作结果返回。捕获分支在该位置显式绑定 continuation，或用通配模式接收后丢弃。普通操作参数的个数与模式要求不因此改变。

捕获目标是所选 handler 的限定边界，不是整个线程的任意剩余执行。多个边界可以嵌套，请求的 occurrence 决定捕获到哪一层。

### 三种结果与 deep 恢复

操作结果 Y、主体结果 A、整个处理结果 B 分别检查。内部类型关系如下：

~~~text
operation : X -> Y
body      : Comp(inner, A)
resume K  : Y -> Comp(outer, B)
branch    : (X, K) -> Comp(outer, B)
~~~

这些是分析记号，不是 builtin.Continuation 的公开类型参数。默认整体保留主体结果；若改变 A 到 B，正常完成路径也必须进行对应转换。

分支在所选处理边界外执行。恢复重装被捕获计算内的 handler，包括转发时跨过的内部边界；外层操作入口由恢复点供应。固定局部值及其可达依赖不会因此被换成同名的新对象。

### 尾恢复与非尾恢复

resume 交付 Y，并结束当前分支。在捕获分支中，它消费当前恢复权并保证尾切换：分支不能再留下一个仅为等待本次恢复完成而存在的帧。

显式的非尾恢复可以等待 B，再执行分支的后续工作；此时等待中的分支仍然存活。反复采用非尾恢复可能积累这些帧，这属于程序保留的后续工作。

### 存活、逃逸与清理

continuation 不可 Copy，也不提供 Clone。可达依赖决定它能否逃逸：跨过源码中的安装位置并非一律禁止，但它需要的栈、handler 环境和外部对象必须继续有效。

池化后端使用处理协议供应的栈池实例，可以由使用者独立接入，也可以由已安装的 runtime 提供。continuation 保留对所用存储的依赖；恢复到另一个供应范围，不改变这些栈的分配来源。来源实例在相关栈完成清理并归还前保持有效，见[栈池实例](effect-abi.md#显式栈池实例)。

尚未恢复或转交的恢复权在离开清理范围时执行放弃协议。已经移入队列的恢复权由队列持有，原分支不得再恢复或放弃它。

放弃以隐藏中止通道重新进入被挂起计算，执行尚存资源的退出清理，到限定边界停止。中止不属于用户错误行，不经过用户 catch arm；不得只归还栈内存而跳过资源责任。

挂起计算不跨线程恢复。捕获区域含 C 帧时可以挂起并恢复，但不能放弃；无法静态排除这种放弃时保留运行时检查，实际放弃越过 C 帧屏障才触发 trap。

### 内建能力的检查范围

builtin.Continuation 保持无公开类型参数。resume 这样的直接语法位置可按当前操作检查 Y；逃逸后的原始显式恢复接近 intrinsic，属于 unsafe 契约。封装必须维护操作结果、整体结果、外层入口与依赖之间的对应，不能因统一的类型名而混用任意恢复载荷。

## 常见示例

### 三次挂起对应三个恢复权

以下测试片段中的 pool 由测试环境预先创建，并实现所选池化控制协议；每次安装用 .use(pool) 明确供应它。

~~~flurry
effect yield_number(value: u32, catch continuation: builtin.Continuation);

struct ProducerTrace {}

impl Drop for ProducerTrace {
    fn drop(*self) { println("cleanup producer"); }
}

fn produce() -> #[yield_number] void {
    let trace = ProducerTrace {}
    for value in 0u32..3u32 {
        yield_number(value)#;
    }
    println("finished");
}

test {
    (produce()# {
        yield_number(value, continuation) => {
            println("{}", value);
            resume;
        },
    }).use(pool);
}
~~~

依次输出 0、1、2、finished、cleanup producer。每次分支的 resume 消费当前 continuation；第二次请求不会重新使用第一次的恢复权。trace 跨越三次挂起而保持存活，主体完成后才清理。void 操作的 resume 无需结果表达式。

### 只接受第一个值

~~~flurry
test {
    (produce()# {
        yield_number(value, continuation) => {
            println("first: {}", value);
        },
    }).use(pool);
}
~~~

依次输出 first: 0、cleanup producer。分支没有恢复或转交 continuation，退出时放弃剩余计算；finished 不执行，trace 则沿放弃路径清理。这里的清理不能被简化为直接归还栈槽。

### 非尾恢复保留后续工作

下面的 resume_u32 是本例对底层能力的适配声明，不是标准库 API。其契约是消费给定恢复权、交付一个 u32，并返回整体 u32 结果；实际实现由选定控制后端提供。

~~~flurry
unsafe fn resume_u32(continuation: builtin.Continuation, value: u32) -> u32;

effect ask(catch continuation: builtin.Continuation) -> u32;

fn twice_answer() -> #[ask] u32 {
    let answer = ask()#;
    answer * 2u32
}

test {
    let result = (twice_answer()# {
        ask(continuation) => unsafe {
            let resumed = resume_u32(continuation, 6u32);
            resumed + 1u32
        },
    }).use(pool);
    println("{}", result);
}
~~~

主体取得 6，产生 12，随后处理分支产生 13。这里的 +1 必须保留，所以不能把这次恢复当成尾恢复。

### 队列封装需要维护的状态

以下是内部执行图，不是新增队列 API：

~~~text
请求挂起 → 分支取得 K1 → 将 K1 移入等待队列 → 分支不再拥有 K1
事件到达 → 从队列移出 K1 → 消费 K1 并恢复
再次挂起 → 取得 K2 → 转交 K2
正常结束 → 不再产生恢复权 → 清理并归还栈
~~~

若完成通知与取消都试图取出 K1，封装必须使其中一个取得唯一所有权，另一个观察到已转交状态。把 K1 的表示复制到两个队列不满足契约。

## 与 Rust / Zig / C / C++ 的关键差异

无对等的通用限定 handler 恢复接口。C 的 setjmp/longjmp 保存跳转环境，但不提供这里的 deep handler、单次消费及清理协议；C++ 协程提供显式挂起和恢复，其函数与帧边界也不是本篇的多 prompt 路由。[C++ 协程](https://eel.is/c++draft/dcl.fct.def.coroutine)

## 运行时实现设计引导

恢复权至少关联执行位置、栈或帧存储及其来源池实例、限定边界、内部处理上下文、清理状态，以及向本次恢复调用交回 B 的位置。

状态迁移为：挂起并持有 → 转交或运行 → 完成；放弃走单独的清理过程。恢复和放弃都必须先取得唯一执行权。已消费的句柄不得再次切入；一次新挂起必须产生新的句柄身份。

尾恢复先结束当前分支的局部责任，再切入保存的计算。非尾恢复保存分支的返回位置。正常完成和清理完成最终都要回到正确的接收方，不能使用已经退出的安装者帧。

## 编译器实现引导

1. 从声明读取捕获许可，为请求确定所选限定边界。
2. 按分支是否取得 catch 位置选择直接或捕获入口。
3. 检查 Y、A、B 的接合以及各控制目标。
4. 保存内部 handler 与恢复时需要重新供应的外层接口，保留固定依赖。
5. 以普通可达性和资源检查约束转交；原始显式操作仍履行其 unsafe 契约。
6. 为 resume 生成不保留等待帧的尾切换；为显式非尾恢复保留接收 B 的后继。
7. 建立清理入口与 C 帧屏障，避免把中止交给用户错误匹配。
8. 检查二次消费、悬垂环境、错误线程与不兼容恢复载荷。
