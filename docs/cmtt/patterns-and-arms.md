# 模式代码与 quoted arm

## Intro

表达式代码需要输入绑定，模式代码则在匹配成功时提供绑定。将两者接在一起，需要知道主体使用的是哪一次匹配取得的值。

quoted arm 保存这份关联，让一条规则的模式、guard 和成功主体作为整体被复用。

## 语法

~~~text
contextual_pattern_type -> < expr -| runtime_binding* >
quote_pattern -> '<pattern> | '[ substitution* ] <pattern>
splice_pattern -> ~<expr> | ~[ substitution* ] <expr>
case_param -> case pattern
quote_function -> quote fn id(parameter*) (-> result)? clauses? (block | (= expr))?
~~~

pattern 和分组优先级见[模式匹配](../control-flow/pattern-matching/index.md#模式优先级与括号)。quoted arm 通过 quote fn 与函数级模式表达，不增加独立的 arm 引用关键字。

## 正式定义

### 输入与成功输出

< T -| Ψ > 描述匹配 T 的模式代码。匹配成功时产生一份新的 Ψ；失败路径没有这些绑定。类型中的 Ψ 是输出接口，不是在构造 quoted pattern 时已经创建的 runtime 对象。

独立 quoted pattern 的公开接口不能暗藏额外 runtime 局部输入。需要外部运行时阈值或存储身份的规则，应在整个规则接口中交付相应上下文，并将它同时用于模式和主体。

每次 pattern splice 都产生新绑定身份。重用同一代码值不会让两场匹配共享一份运行时变量。模式导出的依赖类型也随这次新鲜化一起更新。

### 配对的定向检查

当生成器接口明确要求一份 < T -| Ψ > 与一份 < Ψ |- U > 时，先确定模式的输出接口，再在该虚拟上下文中检查表达式代码。正文中的名字指向接口授予的绑定，而不是偶然同名的外部变量。

普通元组相邻放置两份 quote 不会自动建立这种关系；关系必须来自目标接口、合法的替换或完整的 arm。模式可以导出更多绑定，但未被主体使用的资源仍须履行清理责任。

### Arm 的完整接口

内部可以把一份 arm 记为 Arm(Γ; T; Ψ; U)：在外部 Γ 下匹配 T，成功产生 Ψ，再在 Γ、Ψ 下交付 U。Arm 是分析记号，不是公开类型工厂。

模式、成功主体和控制关系作为一份代码保存。不能仅把主体取出当作闭表达式，再靠同名变量补齐缺失输入。不同 arm 可以拥有不同的 Ψ，只需满足目标要求的外部接口和结果接合。

### 规则集合

模板的 arm 接入协议提供有限规则集合及分派契约。单条允许非穷尽的规则不承担全输入覆盖，但组合后的目标仍必须满足对应 match 的覆盖要求。

open code 适合分散维护的大量入口，如重写规则或路由；一个已由生成器完整持有的有限图不需要先变成分散注册集合。规则集合来源必须明确，不能把文件枚举顺序当作重叠模式的优先级。

## 常见示例

### 模式产生绑定，正文使用它

~~~flurry
fn matching_value(
    value: u32,
    comptime pattern: < u32 -| number: u32 >,
    comptime body: < number: u32 |- u32 >,
) -> u32 {
    value match {
        ~<pattern> => ~<body>,
        _ => 0u32,
    }
}

test {
    let first = matching_value(42u32, '<(20u32..100u32) as number>, '(number + 1u32));
    let second = matching_value(12u32, '<(20u32..100u32) as number>, '(number + 1u32));
    println("{} {}", first, second);
}
~~~

输出 43 0。第二次失败没有产生 number，因此 fallback 不能接入使用 number 的 body。matching_value 本身不是 quote，其中两个独立 splice 不违反单洞规则。

### 两次接入分别产生绑定

~~~flurry
fn extract_twice(left: u32, right: u32) -> u32 {
    const pattern: < u32 -| number: u32 > = '<number>;
    const read: < number: u32 |- u32 > = '(number);
    let ~[number = first]<pattern> = left;
    let ~[number = second]<pattern> = right;
    ~[number = first]<read> + ~[number = second]<read>
}

test { println("{}", extract_twice(20u32, 22u32)); }
~~~

输出 42。first 和 second 是两份不同的绑定；同一个 pattern 值只共享代码结构。

### 元组模式导出两个绑定

~~~flurry
fn add_pair(pair: Tuple<u32, u32>) -> u32 {
    const pattern: < Tuple<u32, u32> -| left: u32, right: u32 > = '<(left, right)>;
    const body: < left: u32, right: u32 |- u32 > = '(left + right);
    let ~<pattern> = pair;
    ~<body>
}

test { println("{}", add_pair((19u32, 23u32))); }
~~~

输出 42。模式成功建立的同一对应供给 body；不能用另一场匹配的 left 和本场的 right 拼出一份未经检查的依赖上下文。

### Quote 函数的分支保存成功关联

~~~flurry
enum Command { reset, add(u32), }
quote fn command_value(command: Command) -> u32;

quote fn command_value(case .reset) -> u32 { 0u32 }
quote fn command_value(case .add(number)) -> u32 { number + 1u32 }

fn request() -> Command {
    println("request");
    Command.add(41u32)
}

test { println("{}", command_value(request())); }
~~~

输出 request、42。实参只执行一次，add 分支的 number 与其主体一起接入。reset 分支不能使用这个 number。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 宏可接收 pattern 与 expr 片段，C++、Zig 可通过模板或编译时分派生成相应分支；它们不直接提供本篇模式输出上下文与正文输入上下文的代码类型关系。C 宏则需由作者显式管理生成绑定。[Rust 宏片段](https://doc.rust-lang.org/reference/macros-by-example.html#metavariables)

## 运行时实现设计引导

接入后的模式执行普通判别与绑定。只有成功路径初始化导出位置，失败转向下一臂或既定失败出口。尝试一个模式不能提前消费后续失败路径仍需要的值。

完整 arm 的 scrutinee 按目标构造只求值一次；模式观察可以按契约共享，成功主体不能在尚未选中时执行。

## 编译器实现引导

1. 取得 pattern 或 arm 的实际类别和输入接口。
2. 为外部 Γ 建立替换，并检查模式输入 T。
3. 给每次接入分配新鲜成功上下文 Ψ，同时更新全部依赖。
4. 在成功环境检查主体；失败环境不包含 Ψ。
5. 对 arm 统一替换模式、guard、主体及控制出口。
6. 组合有限集合，检查结果、覆盖、重叠策略、效果及资源责任。
7. 保存规则来源和集合契约，作为接入与缓存的依赖。

类型相容不能替代“这份成功绑定属于这次匹配”的证据。缓存可共享规则结构，但每个接入点必须重新实例化绑定身份。
