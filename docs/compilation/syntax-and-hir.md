# 源码准备、解析与 HIR

## Intro

解析首先要确定代码的范围。if 后面的花括号属于条件表达式，还是属于 if 的主体，会决定得到怎样的程序。

进入 HIR 时，这些范围已经明确；取类型、取位视图等操作也各自具有明确种类。后续检查不需要反复从字符串拼写猜测它们的含义。

## 语法

```text
attribute -> ^expr
multi_element -> base_element
    | ^expr multi_element
parenthesized_expr -> (expr)
parenthesized_pattern -> (pattern)
extended_application -> expr { (property | expr)* }
extended_application_pattern -> pattern { (property_pattern | pattern)* }
take_view -> expr ' id
```

multi_element 和 base_element 是这里说明列表解析的元记号：base_element 取当前语法列表允许的元素。它们不是新的源语言构造；原始 token 序列也不因带有重复记号就自动成为这样的列表。

## 正式定义

### 输入准备

源码集合、包构建实例和需要的构造宏先由工程依赖确定。生成文本必须明确纳入源码集合，才能参与模块、名称与类型检查。脚本的局部绑定不自动进入消费者作用域。

静态容器及导入关系按[代码组织](../code-organization/index.md)解释。两个路径指向同一声明时保留同一来源；同名而不同来源的声明不能因为扫描次序而被合并。

### 列表元素上的属性

按 multi 规则组织的语法列表，其每个元素前都可以带嵌套的属性前缀。前缀层数没有固定的语法上限；每层修饰紧随的元素，而非整个列表。嵌套列表分别应用相同规则。

这不等于任意表达式的每个内部操作数位置都新增了属性前缀。元素允许什么主体、使用什么分隔符，仍由该位置的语法决定。

多个属性的应用次序与键覆盖遵守[属性系统](../comptime/attribute.md)：从内到外应用。语法上接收属性，不等于每个属性键在任何目标上都有作用；具体功能仍检查其目标和取值。

### 表达式与模式的外层边界

表达式和模式的解析受所在构造的边界约束。外层构造需要后续花括号作为主体时，当前层的后缀扩展应用不能抢走这个花括号。

此限制也要通过 matches 等构造传给其中的模式。否则表达式停止消费后缀，模式却仍可能把 if 的主体读成自己的扩展应用。

括号和其他已封闭的内部语法范围允许在内部使用完整的相应表达式或模式。外层限制不等于禁止所有后代子树中的扩展应用。括号划定模式范围，逗号则继续区分元组；(pattern) 与单元素元组不是同一形式。

### Take view

take view 的操作种类由 view 名确定，其使用条件由对应机制规定。取类型不等于读取 runtime 内容，取某种视图也不自动取得额外的访问许可。

操作种类在进入 HIR 时明确下来，不意味着此时所有操作都已求值或所有类型都已确定。Intrinsic 的输出指定仍遵循[直接 as 规则](../pipeline-projection/intrinsics.md)，不能被普通类型转换的处理覆盖。

## 常见示例

### 每个实参元素可以有自己的属性链

```flurry
use std.prelude.*;

fn keep_flag(value: bool) -> bool {
    value
}

test {
    let result = keep_flag(
        ^{ some_attr: 1 }
        ^{ some_attr: 2 }
        true,
    );
    println("{}", result);
}
```

some_attr 是教学元数据键，不规定额外执行行为。两层属性都属于 true 这个实参元素；外层值 1 覆盖内层值 2。程序仍输出 true。属性不会附着到整个实参列表，也不会使该输入被计算两次。

### 条件的花括号属于 if

```flurry
struct Flag {
    enabled: bool,
}

fn classify(enabled: bool) -> u32 {
    if enabled {
        1
    } else {
        0
    }
}

fn from_object() -> u32 {
    if (Flag { enabled: true }).enabled {
        1
    } else {
        0
    }
}
```

classify 中的第一个花括号开始 if 主体，不能将 enabled 与它合成扩展调用。from_object 的对象构造位于括号范围内，括号外的花括号才开始 if 主体。

### 模式同样需要边界

```flurry
enum Message {
    data { value: u32 },
    empty,
}

fn is_empty(message: Message) -> bool {
    if message matches .empty {
        true
    } else {
        false
    }
}

fn has_data(message: Message) -> bool {
    if (message matches .data { value: _ }) {
        true
    } else {
        false
    }
}
```

is_empty 中的主体不属于 .empty 模式。has_data 中的内层花括号属于模式，外层花括号属于 if；分组使两个范围明确。

### Take view 的操作可以组合

```flurry
pure comptime fn ValueType(quote value: Any) -> Type {
    value'type
}
```

这里观察输入的类型描述，不读取 runtime 数据。编译器必须保存取类型这一操作的身份，而不能把 type 当作任意同名成员查找。

## 与 Rust / Zig / C / C++ 的关键差异

Rust HIR 也在 AST 之后承担语义分析使用的表示；GCC GENERIC 为前端提供与源语法不同的公共结构。这说明 parser 的节点形状不必与后续 IR 一一对应，但它们不规定 Flurry 的 take view 映射。[Rust HIR](https://rustc-dev-guide.rust-lang.org/hir.html)、[GCC GENERIC](https://gcc.gnu.org/onlinedocs/gccint/GENERIC.html)

属性的实际对应物包括 Rust 属性、C++ 属性及 C 的编译器扩展；Zig 通过类型构造和 comptime 设施表达多种相关要求。它们与 Flurry 的 Object 属性链的差异见[属性系统](../comptime/attribute.md#与-rust--zig--c--c-的关键差异)。

## 运行时实现设计引导

分组和属性附着本身不生成一次额外的 runtime 调用。运行时应执行解析后确定的控制流，并接收属性使用者已经兑现的布局或调用约定。

HIR 专用节点也不要求每种 view 在机器码中对应一条指令。取类型可以只参与编译时计算；位视图等操作则依据自己的契约生成代码。

## 编译器实现引导

### 列表解析与上下文

```text
parse_multi_element(context):
    prefixes = read_attribute_prefix_chain(context)
    element = parse_base_element(context)
    check_element_separator(element)
    return wrap_from_inner_to_outer(prefixes, element)

parse_condition_before_block():
    parse_expression(stop_before_outer_block = true)
    propagate that boundary to embedded unclosed pattern positions
    parse the outer block
```

属性表达式与目标元素须有明确边界；不能让读取属性值的逻辑把后续标识符吞成无关后缀。进入括号、调用参数等独立范围时建立其自身上下文，离开时恢复外层限制。

前缀链可迭代收集，避免把源码中的链长直接变成宿主调用栈深度。仍需报告资源耗尽，不能截断链后继续成功解析。

### 专用 HIR 节点

每一种已定义的 take view 操作均有对应的 HIR node kind。以下名称只是内部表示示意：

| AST 形态 | HIR 形态 |
|---|---|
| value'type | TypeOf(lower(value)) |
| value'bitvec | BitVecView(lower(value)) |
| value'bitvec'type | TypeOf(BitVecView(lower(value))) |
| tag'intrinsic | IntrinsicView(lower(tag)) |

Lowering 按操作表分派，携带操作数和来源，不将已识别的 view 留作通用字符串投影。该规则覆盖每一种 view，而不是只为常用的 type、bitvec 提供例外。

相邻调用和 as 可以在后续接入中组合为带显式输出类型的 intrinsic 操作。拆成专用节点不应丢失这种邻接关系，也不能提前把结果固定为 void 后再做普通转换。

### 名称、身份与来源

先建立足够的声明身份与静态容器关系，再按位置完成名称解析。需要类型信息才能完成的成员解析保留真实依赖，不能选取首个同名候选作为最终结果。

HIR 保存可追溯的源码来源和生成来源。内部节点编号只在其所属表示与会话内有效；缓存需要另有稳定身份和依赖检查。对于既定 token 流，前缀及普通结构遍历按输入规模计费；宏执行、回退尝试和后续语义查询分别统计。
