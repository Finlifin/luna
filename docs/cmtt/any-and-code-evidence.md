# Any、any 与代码接入的证据

## Intro

生成器有时需要把不同接口的代码放在同一容器里。放宽静态描述可以做到这一点，但代码的真实类别、绑定和依赖仍要保留。

取出代码准备接入时，使用处需要的是它实际能够提供的接口，不能只凭“与 Any 相容”认定任意操作都合法。

## 语法

~~~text
contextual_expr_type -> < runtime_binding* |- expr >
contextual_pattern_type -> < expr -| runtime_binding* >
splice_expr -> ~<expr> | ~[ substitution* ] <expr>
splice_pattern -> ~<expr> | ~[ substitution* ] <expr>
~~~

Any 和 any 继续使用既有 comptime 通配描述规则，不为 quoted code 引入另一套同名值。

## 正式定义

### 四种不同信息状态

| 情形 | 已知什么 | 接入还需要什么 |
|---|---|---|
| q: Any | 静态描述未说明 q 的实际种类 | 从实际值或合法观察恢复代码类别与接口 |
| q: < Ψ &#124;- Any > | 已知是表达式代码，并有上下文接口 | 取得本次结果交付所需的实际类型关系 |
| 接口部分使用 Any 或 any | 某些信息允许未知或结构匹配 | 保持真实参数关联、依赖和上下文要求 |
| 直接接入 any | 只有通配值，没有给定程序主体 | 不具备代码接入依据，拒绝 |

Any 是类型描述的通配，any 是任意 comptime 值的通配描述；二者都不是等待统一器任选答案的新变量。相容不等于类型相等，也没有经由 Any 的传递闭包。

### 静态描述与实际见证

实际代码值保留 expr、pattern 或 arm 类别、原始接口、主体、绑定来源、固定依赖和必要义务。标注 Any 不擦除这些内容，也不自动将接口泛化到所有上下文或所有类型。

接入可以使用仍含合法参数的接口，不要求所有输入提前具体化；但实际交付前必须兑现所需的阶段、替换和表示依据。不能把“以后检查”无限延迟到 runtime。

若 q 原本产生 bool，使用位置要求 i32 不能把 q 改造成 i32 代码。若 q 是 pattern，表达式位置也不能仅凭共同静态描述 Any 接受它。

### 上下文不能被放宽掉

隐藏开放代码的静态类型，不会使其成为闭代码；未知 Ψ 不等于空 Ψ。已固定的存储或外部对象身份不会因 any 而获得替换为另一个同型对象的资格。

Type 和代码值即使经过 Any，仍是其原有阶段的值，不能直接成为 runtime-only 上下文成员。上下文 rank 同样保留；隐藏一个高秩方案并不能把它变成合法的 rank-1 代码参数。

### 证据必须关联同一份值

从异构 comptime 容器取出某个元素后，可以依据这份实际元素恢复接口。容器被修改后，再读取相同下标可能取得另一份代码；旧检查不能授权新值。

代码值共享或复制只涉及编译时描述。生成程序多次消费同一 runtime 资源时，仍按真正的控制流拒绝非法重复使用。

## 常见示例

### 经过 Any 保存仍可接入具体代码

~~~flurry
pure comptime fn keep(value: Any) -> Any { value }

fn through_any(value: i32) -> i32 {
    const code: Any = keep('(value + 1i32));
    ~<code>
}

test { println("{}", through_any(41i32)); }
~~~

输出 42。本次编译时实例能够恢复 expr 类别、value 的绑定及 i32 结果。keep 没有把任意输入转换成代码。

### 目标类型不能改写实际主体

以下为独立拒绝例：

~~~flurry
fn wrong_result() -> i32 {
    const code: Any = '(true);
    ~<code>
}
~~~

实际代码产生 bool，不能因为外部描述为 Any 就按 i32 交付。

### 类别放宽不产生转换

~~~flurry
fn wrong_kind() -> u32 {
    const pattern: < u32 -| number: u32 > = '<number>;
    const code: Any = pattern;
    ~<code>
}
~~~

这是拒绝例：实际取得模式代码，当前位置需要表达式代码。应把它接到模式位置，再在成功路径使用模式提供的绑定。

### any 不是等待生成的程序

~~~flurry
fn missing_program() -> u32 {
    ~<any>
}
~~~

这是拒绝例。目标 u32 不能替生成器提供一个任意程序，也不能使 any 自动变成 0 或某个局部变量。

### 运行时条件不能决定 comptime 代码值

~~~flurry
fn late_choice(flag: bool) -> u32 {
    const code = if flag { '(1u32) } else { '(2u32) }
    ~<code>
}
~~~

这是拒绝例：flag 在 runtime 才有值。把 code 标成 Any 不改变阶段。正确做法是引用一个含有 flag 的 runtime 条件：

~~~flurry
fn runtime_choice(flag: bool) -> u32 {
    const code = '(if flag { 1u32 } else { 2u32 });
    ~<code>
}

test { println("{} {}", runtime_choice(true), runtime_choice(false)); }
~~~

输出 1 2。代码结构在编译时已确定，分支选择留给运行时。

## 与 Rust / Zig / C / C++ 的关键差异

无对等的 Any/any 与 contextual 代码接口组合。C 的 void 指针或 C++ 类型擦除对象可以隐藏运行时表示，但不等于保留并检查这里的编译时代码主体；Rust 宏片段和 Zig comptime 则使用各自的代码生成规则。[Rust 宏](https://doc.rust-lang.org/reference/macros-by-example.html)、[Zig comptime](https://ziglang.org/documentation/master/#comptime)

## 运行时实现设计引导

目标程序不因某个生成步骤使用 Any 而获得统一的 runtime Any 代码对象。接入前确定实际表示与操作；运行时只执行相应生成主体。

对运行时输入的引用继续保持对象身份、有效期和资源责任。编译时包装的删除不得抹去这些依赖。

## 编译器实现引导

接入检查依次执行类别、阶段、rank、上下文、结果与生成后资源核验。每一步必须有实际代码或保留接口的依据，不能从目标要求反向伪造主体。

内部检查结果关联代码值身份和实际接口。缓存键至少覆盖主体、接口、声明来源、固定依赖、替换和相关阶段事实；不能只用 Any 或表面源码文本作键。

代码构造保留尚待实例化义务时，记录义务来源与兑现边界。交付 runtime IR 前不允许留下未知类别、缺失上下文、未定表示或 unresolved splice。

PTC 处理类型命题，程序验证处理生成后的程序状态。Any 相容结果不能跨越这两个层次，成为任意代码接入或内存操作的许可。
