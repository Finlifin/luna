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

### 从载荷补全类型，保留值索引

```flurry
enum Choice where T, enabled: bool {
    none,
    some(T),
}

fn keep(value: Choice<T, true>) -> Choice<T, true> where T {
    value
}

fn run() -> bool {
    let stored = keep(.some(true));
    let .some(value) = stored else { return false; };
    value
}
```

`keep` 的参数上下文已经固定 enabled 为 true；`.some(true)` 的载荷再提供 T 为 bool 的信息。因此 stored 的类型是 `Choice<bool, true>`，不需要另一个实参先补全类型。推断不会把值索引当作 Type 参数，也不能把 `Choice<bool, false>` 当作同一个类型。

位置载荷的展开和命名载荷的默认值沿用同一规则：保留上下文中已知的输入，再从载荷类型取得剩余证据，最后检查完整参数域和变体字段。值索引也可以由载荷的类型身份确定：

```flurry
struct Flag where enabled: bool {}
enum Wrapped where flag: bool { Item(Flag<flag>) }
fn read(value: Wrapped<flag>) -> bool where flag: bool { flag }
fn run() -> bool {
    read(.Item(Flag<true>{})) and not read(.Item(Flag<false>{}))
}
```

这里推导依据是 `Flag<true>` 与 `Flag<false>` 的类型身份，不读取 runtime 载荷值。计算得到的参数包展开也能提供这类证据。没有载荷证据、没有完整上下文的值索引不能猜定；同一索引从多个字段得到冲突值时拒绝构造。默认值只能使用已确定的输入。

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

### 显式编码值与模式共用表示

固定宽度的表示也可以直接写一个编译时值。下面的空包编码为零，数据包使用高四位作为判别条件：

~~~flurry
typealias Byte = .u8'builtin;
typealias Nibble = .BitVec'builtin<4>;

enum Packet {
    .base_type Byte,
    .discrete_tag true,
    empty = 0,
    data: 0xA (payload: Nibble),
}

fn encode(packet: Packet) -> Byte {
    (packet'bitvec).as(Byte)
}

fn example() -> bool {
    encode(Packet.empty) == 0
        and encode(Packet.data { payload: 5 }) == 0xa5
}
~~~

表达式编码按 base type 检查，`empty = 256` 不属于 Byte 的域；`empty = 0xa5` 则与 data 的模式重叠，两者均拒绝。没有载荷的表达式变体仍通过成员名构造、通过成员模式匹配，并不授予任意整数到 Packet 的转换。同一编码出现在不同 enum 中，也不改变各自的名义类型身份。

当前固定掩码实现支持这种混合，以及只给整体 width 时的 BitVec 表达式编码；配置和表达式均通过 comptime invocation 取得。一般表示协议仍未接通。

Symbol 可以作为独立的编码域：

~~~flurry
comptime fn ready_code() -> Symbol { .wire_ready }

enum Message {
    .base_type Symbol,
    .discrete_tag true,
    ready = ready_code(),
    waiting: .wire_waiting,
    closed = .wire_closed,
}

fn is_ready(message: Message) -> bool {
    message match {
        .ready => true,
        _ => false,
    }
}
~~~

`Message.ready` 的编码是 `.wire_ready`，但它的类型仍是 Message；编码相同的另一 enum 不因此成为同一 nominal 类型。编码之间必须互斥，重复时诊断给出冲突变体与 Symbol。有限个变体也不覆盖所有 Symbol，未列出的符号不属于这个 enum 的合法编码域。

当前实现要求显式 Symbol base type 和 `discrete_tag true`，各叶变体提供表达式或单个常量 Symbol 模式；支持分组及已可求值的 comptime 表达式。Symbol 表示没有隐含整数位宽，不能同时指定 width，也不能借此接纳载荷。表示表随 NIR/IIR 保留并在执行入口验证，底层编码／判别接口共用此表。LLVM host 路径已支持这类 enum 的构造、传递和成员模式匹配，私有存储使用只读符号文本及其长度，不增加整数判别值；这不是对 FFI 表示的承诺。将原始 Symbol 受检转换为该 enum 仍待实现；runtime Symbol 的独立存储不能替代 enum 的合法编码接纳检查。

### 分组构造与整组匹配

`'tag_name` 取得当前叶变体的声明名称，返回 `str`。例如 `Message.ready'tag_name` 是 `"ready"`，不会返回它的 Symbol 编码 `"wire_ready"`。分组变体保留路径：

~~~flurry
enum Status where T {
    network.{ready(T)},
    closed,
}

fn describe(value: Status<bool>) -> str {
    value'tag_name
}

test {
    assumes describe(Status<bool>.network.ready(true)) == "network.ready";
    assumes describe(Status<bool>.closed) == "closed";
}
~~~

操作数只求值一次。读取已有局部或字段的标签不转移它的载荷，取名后仍可使用原值；已移动的值不能再取名。临时 enum 的载荷仍按资源规则清理，返回的字符串不依赖原对象存活。这一视图支持 runtime 与 comptime invocation，所得字符串可作为 normal-form 值参数或 staged 参数。具有闭合、可冻结载荷的 enum 值本身也可作为普通函数的 comptime 参数，见[阶段参数的 nominal 值](../../functions/comptime_fn.md)。

分组组织同一 enum 的叶变体，不引入新的载荷类型：

~~~flurry
typealias Byte = .u8'builtin;

enum Event {
    network.{
        connected,
        received(Byte),
        nested.{stalled},
    },
    stopped,
}

fn is_network(value: Event) -> bool {
    value match {
        .network => true,
        .stopped => false,
    }
}

fn example() -> bool {
    is_network(Event.network.connected)
        and is_network(.network.received(3))
        and is_network(.network.nested.stalled)
        and not is_network(Event.stopped)
}
~~~

`.network` 模式覆盖该组全部后代叶变体，包括 `nested.stalled`。它不绑定载荷，也不消除载荷原有的资源清理责任。需要取出载荷时，使用 `.network.received(byte)` 等具体叶模式。分组名本身不是值，不能构造 `Event.network`，也不能把它作为独立类型。

整组匹配按完整路径计算成员集合：`a` 不包含邻组 `ab`；空组不匹配任何值。嵌套命名载荷仍使用叶变体自身的字段默认值，泛型叶变体仍属于外层 enum 的同一实例。编码表示的互斥检查跨越分组边界，不能用分组掩盖两个叶变体的重叠。

当前实现将整组模式展开为叶模式的析取，保留分组路径及稳定叶索引；尚未实施连续编码组的范围判断优化。

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
