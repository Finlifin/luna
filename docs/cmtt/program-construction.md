# 在编译时组装有限程序

## Intro

生成器可以先计算程序需要的结构，再构造实际执行这些结构的代码。单洞约束每一次引用的接入形态，并不限制生成器能构造多少层代码。

自动机是一个实用例子：编译时组织有限状态与转移，运行时根据输入反复经过这些状态。

## 语法

~~~text
quote_expr -> '(expr) | '[ substitution* ] (expr)
splice_expr -> ~<expr> | ~[ substitution* ] <expr>
contextual_expr_type -> < runtime_binding* |- expr >
~~~

生成器使用普通 comptime 函数、循环、数组与代码值，不需要为每个生成分支注册一个 open code 规则。

## 正式定义

### 单洞链可以构造多步程序

一份引用可以把先前构造的完整代码接入一个洞，再添加本层自己的控制或计算。循环重复这一操作，最终代码可包含多层语句或分支，每份引用仍至多一个洞。

每次接入保留先前代码的检查结果和接口；代码不是未经检查的 AST 文本。若某种组合确实需要两份独立接口同时参与，必须由具有明确输入输出契约的组合操作承担，不能在同一个 quote 中写两个 splice 后依赖优化器删除其中一个。

### 生成图与执行图

自动机的状态数和边数在编译时有限。运行时图中存在回边，不要求生成器沿该边无限递归展开；生成器交付有限分派体和一个 runtime 循环即可。

代码大小取决于生成结构，不随运行时输入长度增长。具体布局可以使用条件链、决策树、表或其他已支持代码构造；算法和性能选择由生成器决定。

### 捕获是额外的结果接口

若匹配需要返回捕获，生成器还须确定捕获槽、成功交付及失败路径。捕获视图的可达性必须关联真实输入，编译时知道槽数或名字不使捕获数据提前存在。

quoted pattern 或 arm 可以把成功结果接到使用者的绑定；失败不能泄露未初始化捕获。正则方言、重复捕获的选择策略和返回类型属于相应库契约，不由 CMTT 统一指定。

## 常见示例

### 重复生成三条输出语句

~~~flurry
pure comptime fn repeated_message(count: usize) -> < |- void > {
    var code: < |- void > = '(());
    for index in 0usize..count {
        const previous = code;
        code = '(do {
            ~<previous>;
            println("tick");
        });
    }
    code
}

fn three_messages() { ~<repeated_message(3usize)>; }

test { three_messages(); }
~~~

输出三行 tick。循环在 comptime 构造三层代码，目标中没有用于决定生成次数的 count。每层只接入 previous 一次。

### 从转移描述构造分派代码

下面用 u32 状态和单字节输入。build_step 返回开放代码，其 runtime 接口只有 state 与 byte；转移描述本身在 comptime 已取得。

~~~flurry
struct Transition {
    source: u32,
    byte: u8,
    target: u32,
}

pure comptime fn build_step(edges: Array<Transition, n>)
    -> < state: u32, byte: u8 |- ?u32 >
where n: usize
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
~~~

生成器从末尾向前包裹分支，所以原列表中靠前的转移先尝试。每层 if 的 else 只有一个 splice，成功支直接交付该层已知 target。外层只需要一次显式映射；不需要为每条转移重复映射 state 和 byte。

对这四条边，生成程序的行为等价于：

~~~flurry
fn step_expanded(state: u32, byte: u8) -> ?u32 {
    if state == 0u32 and byte == 97u8 { 1u32 }
    else if state == 1u32 and byte == 98u8 { 1u32 }
    else if state == 1u32 and byte == 99u8 { 1u32 }
    else if state == 1u32 and byte == 100u8 { 2u32 }
    else { null }
}
~~~

### 有回边的匹配程序

接续 step，状态 1 上的 b、c 转移回到自身；状态 2 是唯一接受状态。它识别 ASCII 子集的 a[bc]*d，并要求消费整个输入。

~~~flurry
fn matches_input(input: Array<u8, n>) -> bool
where n: usize
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

test {
    println("{}", matches_input([97u8, 100u8]));
    println("{}", matches_input([97u8, 98u8, 99u8, 100u8]));
    println("{}", matches_input([97u8, 98u8, 99u8]));
    println("{}", matches_input([97u8, 100u8, 100u8]));
}
~~~

依次输出 true、true、false、false，对应 ad、abcd、abc、add。运行时可以在状态 1 循环任意多次，生成器只处理四条有限边。

这里完整展示的是从转移描述到代码的组装，并未定义一套正则解析 API。更大的正则库可以先计算同类有限结构，再选择合适的分派布局和捕获接口。

## 与 Rust / Zig / C / C++ 的关键差异

这些语言可以通过构建期生成器、宏、模板或 comptime 产生自动机。Flurry 在本例中把开放程序保存为有上下文类型的值，并逐层保留替换关系。以 C 为目标的 re2c 也展示了自动机代码生成及捕获处理的实际用途；它不规定 Flurry 必须采用相同正则或捕获策略。[re2c 文档](https://re2c.org/manual/manual_c.html)

## 运行时实现设计引导

上述生成结果每个输入字节至多检查 E 条边，长度 L 的输入耗时 O(L×E)，状态与游标占 O(1) 额外空间。这里 E=4。生成器可以改用表或其他分派结构降低单步查找，但应保持重叠边的选择契约。

失败立即返回 false；成功要求消费完输入并处于接受状态。捕获版本另维护其槽与失败回滚，不得把布尔识别器的常数状态空间结论推广到任意捕获算法。

## 编译器实现引导

1. 执行有限转移集合的 comptime 构造，保留每次代码值的实际接口。
2. 每次包装 quote 独立检查单洞及结果汇合；previous 必须指向本次取得的代码值。
3. 将 comptime edge 的字段合法物化为目标常量，不能把宿主对象地址带入目标。
4. 保持共同 state、byte 接口，将最终替换一致作用于整个生成结构。
5. 检查生成循环中的状态、下标及失败出口，并核对资源和效果。
6. 交付有限目标代码；缓存依赖包括转移集合、顺序、生成规则与接口。

条件链的目标大小为 O(E)。使用不可变共享节点时，构造可以逐层记录链接；若每次都深复制先前主体，构造成本可能退化到 O(E²)。最终展开和验证也须计入预算，不能只计算生成器循环次数。
