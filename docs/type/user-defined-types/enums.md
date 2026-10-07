# enum

## Intro

enum 描述一个封闭的变体集合。一个值属于其中某个变体，而变体可以没有载荷，也可以带位置载荷或命名字段。

判别变体既可以依靠 tag，也可以依靠明确的编码约束。这种双重判别机制使 enum 既能表达高层逻辑分支（如消息类型），又能直接映射底层编码格式（如指令字节），因此不要假定每个 enum 都额外保存一个独立 tag 字段。

## 语法

~~~text
enum_def -> enum id clauses? { (configuration | enum_variant | definition | statement)* }
enum_variant -> id
    | id(expr*)
    | id { struct_field* }
    | id: pattern
    | id = expr
    | id.{ enum_variant* }
~~~

id: pattern 与 id = expr 是不同形式。圆括号只表达位置载荷；命名字段用大括号。width、base_type、discrete_tag 等配置使用[属性系统](../../comptime/attribute.md)的 configuration 形式。

## 正式定义

### 变体与分组

enum 的变体集合封闭。普通变体可以用 tag 区分；指定 pattern 或 expr 的变体按相应约束区分。分组支持整组匹配，其含义是覆盖该组叶变体，不由此额外推出组名有独立类型身份。

表达式变体只接纳该表示机制支持的结果域；Symbol 属于可用域。不能由“这里是 expr”推出所有 comptime 值都能编码为变体。

### 模式表示

pattern 变体的模式作用于 base type。对于 enum 表示中的位模式，u8、u16、u32、u64、usize、u128 与无符号 ArbitrarySizedInteger 可作为相应主语；普通整数的位模式匹配仍按[BitVec](../../control-flow/pattern-matching/bitvec.md)要求显式取位视图。

discrete_tag 使判别布局由所写模式承担，变体区分依赖模式约束的互斥性。不能再附加一个并未声明的 tag 来掩盖编码冲突。

width 约束整个 enum 表示，包括载荷。设置 base type 时，以 base type 的表示约束为准。不能把 width 当作只限制判别字段。

### 覆盖和合法域

互斥性检查判断两个变体是否能同时接纳一个合法编码。覆盖检查判断合法域中是否有编码不属于任何变体。两者是不同问题；掩码不冲突不等于带 guard 的完整模式一定相交。

编码规则没有授予任意混合布局、跨成员读取或自动构造载荷的许可。具体值构造必须满足所选变体自己的契约。

## 常见示例

用一个小型绘图消息协议展示无载荷、位置载荷与命名载荷。处理函数必须把每个变体解释成相应动作：

~~~flurry
enum Message {
    quit,
    repeat(u32),
    move_to { x: i32, y: i32 },
}

fn dispatch(message: Message) {
    message match {
        .quit => println("quit"),
        .repeat(count) => println("repeat {} times", count),
        .move_to { x: x, y: y } => println("move to ({}, {})", x, y),
    }
}

test {
    dispatch(Message.quit);
    dispatch(Message.repeat(3));
    dispatch(Message.move_to { y: 20, x: 10 });
}
~~~

输出是 `quit`、`repeat 3 times` 和 `move to (10, 20)`。`repeat` 用位置参数构造，也用位置模式取出载荷；`move_to` 的字段按名字对应，调用中交换输入书写顺序不交换坐标。

调用目标也可以为 Symbol 提供变体上下文：

~~~flurry
test {
    dispatch(.quit);
}
~~~

这里 `dispatch` 的参数已经要求 `Message`。这不意味着脱离目标上下文的任意 Symbol 都具有某个 enum 类型。

上例从逻辑变体组织程序。若身份已经编码在现有比特中，可以使用模式变体；[ADD/SUB 编码教学](fixed-width-enum-example.md)给出完整声明、原始指令字匹配和寄存器载荷输出。

## 与 Rust / Zig / C / C++ 的关键差异

Rust enum 可携带载荷；C/C++ enum 主要命名整数值，带载荷数据需要其他结构。Zig 可使用 tagged union。Flurry 的模式变体可以直接以编码约束判别，没有一一对应的 Rust enum 声明形式；真实替代是显式掩码、解析和载荷结构。[Rust enum](https://doc.rust-lang.org/reference/items/enumerations.html)、[C++ enum](https://eel.is/c++draft/dcl.enum)

## 运行时实现设计引导

为每个叶变体生成判别条件和载荷取得计划。普通 tag、模式和表达式编码分别遵循自己的表示契约，不强制使用同一物理结构。

分组匹配可以在编码确实连续且语义等价时降低为范围判断；不能为了优化而重排作者规定的模式编码。构造与判别必须使用同一编码域和合法值约束。

## 编译器实现引导

1. 编译器建立封闭变体树与叶索引。
2. 区分普通载荷、模式、表达式和分组语法。
3. 确定 base type 与整体宽度约束。
4. 检查模式/表达式定义域及载荷契约。
5. 在合法编码域内检查所需互斥与覆盖。
6. 生成构造、判别和载荷访问计划。
7. 验证分组优化与原始叶集合等价。

纯掩码可使用 BV 快路径；带 guard 或复杂模式按 PTC 能力保留残余。一般匹配自动机和跨元素解析使用其专门契约，不能由一次定宽 SAT 检查代替。
