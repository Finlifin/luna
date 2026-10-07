# 语法协议与能力入口

## Intro

语法糖和运算符重载如果缺乏统一的接口约束，会导致行为不可预测。Flurry 选择通过 trait 协议将语法操作（如运算符、字面量后缀、错误处理）与用户实现连接，从而使每种语法形式都有明确的类型检查和契约保证。

有些操作由语法直接提出：使用字面量后缀、处理一个错误，或对对象执行解引用。Trait 为这些操作提供可查找、可检查的实现入口。

语法决定需要哪种能力和怎样组织控制流；实现提供具体行为。找到一个方法，不会自动证明整段控制流合法。

## 语法

~~~text
literal_suffix -> (integer | float | char | str) id
handler_application -> expr.use(expr)
deref -> expr.*
refer -> expr.ref
normal_form_head_application -> expr<argument*>
~~~

各操作完整语法仍由对应主题定义。本篇只描述 trait 接口的连接，不用一个通用“调用重载”替代各操作的求值规则。

## 正式定义

### 共同查询

语法操作先确定接口身份、主体、已知实参与待推断参数；再使用普通 implementation 选择和替换规则取得入口。关联项可以提供后续类型信息，但不能凭普通实现声明制造可信的类型相等、行消去或内存有效性证明。

普通成员简写也遵循该路径：`student.serialize<.json>()` 由投影、范式应用、调用组成。成员来源确定为参数化 Serialize 时，`.json` 应用到该接口的开放参数，再调用普通 serialize 方法。方法自身若另有参数化入口，按实际成员契约处理；来源无法唯一确定时使用显式接口视角，不添加序列化专属语法节点。

### 字面量转换

字面量主体 S、后缀 symbol 和结果 R 组成 `LiteralInto<suffix, R> for S` 查询。R 可以由上下文提供，也可以是本次推断的新元变量；候选之间的试探替换必须隔离。

相同后缀允许对应不同结果类型，因为完整 trait 实例不同。目标上下文不能确定唯一合法结果时，报告该次使用歧义。后缀本身不检查 Pixel 范围或某个字符串格式，这些是具体转换方法的契约。

### 处理能力

Capability 系统由 trait 协议配合 elaboration 实现。Trait 描述处理入口的要求，具体实现提供处理行为；elaboration 将选定的入口与环境接入处理位置。错误与效应的处理仍须满足各自的行、控制流与资源契约。

错误处理入口接收已确定错误类型 E，交付处理位置要求的结果 R。handler 的真实副作用、错误和依赖仍进入整体计算。处理一个错误不意味着其他错误自动消失；处理过程新产生的错误同样要进入剩余行。

效应处理还必须保留完整 operation 实例、occurrence 层次与恢复约束。操作结果、被处理主体结果和整体 handler 结果分别检查。Trait resolution 只提供入口及环境，不能取代行分解、恢复包装和清理检查。

### 协议分工

| 操作 | Trait 连接点 | 由操作主题规定的部分 |
|---|---|---|
| 算术、比较、连接、布尔组合 | 对应操作能力的实现与替换 | 实参求值、短路、结果契约 |
| `.*`、`.ref` | Deref、Reference | 访问合法性、目标身份与适配次序 |
| 下标与空间应用 | Index、SpaceApply | 参数形式、优先关系与失败传播 |
| Pattern view | 所需 phase、关联项和入口 | 绑定、依赖次序、失败与覆盖 |
| 变参收集、隐式传参 | 明确的协议及参数来源 | 求值次数、临时对象、来源描述 |
| 字面量后缀 | LiteralInto | 字面量范围及转换函数契约 |
| Splice | 对应实例及阶段 | 跨阶段输入、proxy 与结果 |
| Error/effect handler | 分支能力及环境 | 行消去、作用域、恢复与清理 |

不存在“协议找不到就随意尝试另一语法含义”的通用回退规则。候选缺失、要求未决与已选方法运行失败必须分别处理。

## 常见示例

### 自定义像素后缀

~~~flurry
struct Pixel {
    value: u32,

    fn new(value: u32) -> Self {
        Self { value: value }
    }
}

impl LiteralInto<.px, Pixel> for Integer {
    fn literal_into(comptime value: Integer) -> Pixel {
        Pixel.new(value.as(u32))
    }
}

test {
    let width: Pixel = 32px;
    println("{}", width.value);
}
~~~

输出 `32`。上下文提供 R=Pixel；后缀为 `.px`，字面量主体为 Integer。转换方法是 runtime phase 的普通函数，其中 value 用于 comptime 实例化。Pixel 构造仍按方法实际阶段执行。

### 通过接口恢复错误

以下示例把处理入口定义完整，并用已有错误分支语法连接它：

~~~flurry
trait HandleError
where E, R
{
    fn handle_error(*self, value: E) -> R;
}

enum LimitErr {
    TooLarge(u32),
}

struct ClampTo {
    limit: u32,
}

impl HandleError<LimitErr, u32> for ClampTo {
    fn handle_error(*self, value: LimitErr) -> u32 {
        value match {
            .TooLarge(_) => self.limit,
        }
    }
}

fn checked_count(count: u32) -> !LimitErr u32 {
    if count <= 8 {
        count
    } else {
        error LimitErr.TooLarge(count)
    }
}

test {
    let handler handles LimitErr = ClampTo { limit: 8 }
    let result = checked_count(12).use(handler);
    println("count: {}", result);
}
~~~

输出 `count: 8`。返回目标 u32 允许读取 `self.limit` 的字段指针；它并不让该方法能够返回任意 R。

这里的处理能力由 `HandleError<LimitErr, u32> for ClampTo` 提供。对 LimitErr 的处理接入 handle_error 方法，checked_count 正常返回时直接交付成功值。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 的运算符接口和派生机制、C++ 的运算符重载都提供语法到用户实现的连接，但具体求值和约束仍由各语言的操作规则规定。Zig 可显式调用 comptime 生成的操作，C 则主要以函数和宏表达替代。不能从“使用 trait”推出这些协议拥有相同控制流。[Rust operators](https://doc.rust-lang.org/reference/expressions/operator-expr.html)、[C++ operator overloading](https://eel.is/c++draft/over.oper)、[Zig](https://ziglang.org/documentation/master/)、[C11](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf)

## 运行时实现设计引导

保留操作本身的执行顺序和分支。以错误处理为例，成功值不调用 handler，匹配的错误调用对应入口，未处理错误按剩余行传播；不能先调用全部候选再挑选结果。

协议入口产生的效果和资源变化必须计入整体摘要。被处理的标签消失，不会抹去 handler 对内存的读写。

## 编译器实现引导

1. 从语法操作取得协议查询及已知类型，不预先固定未知结果元变量。
2. 进行普通接口选择、phase 检查与依赖替换。
3. 根据协议关联项检查剩余输入，维护正确的绑定作用域。
4. 在操作专属规则中检查控制流、行关系、效果和资源义务。
5. 仅为选定入口生成调用，保留原操作与实现来源用于诊断。

处理能力的 elaboration 将协议查询得到的入口和环境连接到对应处理分支：成功路径交付正常结果，已处理分支调用选定入口，其余分支依行关系转发。效应分支还须按恢复契约连接操作结果与恢复路径；仅找到 trait 实现不足以完成这一步。

字面量后缀的结果推断、handler 的结果类型和 pattern 的绑定类型是不同输入输出关系，不能仅因共用 resolution 就共用未经定义的类型合并规则。
