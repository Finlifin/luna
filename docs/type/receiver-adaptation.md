# 接收者适配与隐式转换

## Intro

调用方法时，值的现有形态不一定正好是方法需要的接收者。Flurry 沿有序的视角和解引用路径找到成员，再取得所需指针。

这些适配减少重复操作，但不会免除对象有效性、效果和资源检查。程序仍能明确写出实际访问的接口和目标。

## 语法

~~~text
take_reference -> expr.ref
dereference -> expr.*
as_expr -> expr.as(expr)
projection -> expr.id
application -> expr(argument*)
normal_form_head_application -> expr<argument*>
~~~

类型视角可出现在已有类型标注中。它与 `.as(Trait)` 的显式接口资格共用表达式形式，但选择的对象不同。

## 正式定义

### 类型视角

对于 `A.as(B).as(C).as(D)`，绑定值本身的类型仍是 A。D、C、B 是相对于 A 的 cast candidates，分别需要与 A 具有双向 cast 关系；按 D、C、B、A 的顺序考虑。

这不是 A→B→C→D 的相邻转换链。选中 D 时直接使用 A 与 D 的合法适配。newtype 的表示关系本身不继承底层成员，访问仍需要对应 cast 关系。

显式接口视角则先限定 trait 来源。指定完整 trait 实例以后，不再让其他同名接口参与竞争。

### 方法接收者

在一个类型视角内，从原接收者形态开始，再依次考虑内建指针解引用和唯一确定的 Deref 路径。每个位置内部按[成员来源](member-projection.md)与[实现选择](../traits/implementation-resolution.md)检查。

选定方法需要指针接收者时，进行一次必要的内建取址或 Reference 适配。Reference 由已知目标需求触发，不反向生成无穷的新查找主体；自定义转换存在同优先级歧义时要求显式消歧。

已有指针能直接满足方法需求时，不先复制所指对象再取副本地址。方法按值消耗接收者时仍需合法移动或复制，自动查找不制造资源权限。

### 三层顺序

1. 先按显式视角顺序选当前视角。
2. 在该视角内，按解引用深度从浅到深考虑接收者形态。
3. 同一形态中的同一接口查询，再按 implementation 结构精度选择。

不同 trait 的同名方法不能以“少取一次地址”消歧。更优先来源尚未排除时，不能提交较低优先来源。选定来源后发生参数、环境或资源错误，应报告该调用，不退到另一个来源掩盖错误。

定义端固定成员来源和适配路径的符号契约。实例化负责兑现，不因具体类型后来显露另一同名成员而重新竞争。

### 字段指针与目标引导读取

指针访问结构字段时，原始结果仍是字段指针。结构匹配中的对应字段关系也仍按指针规则处理。

初始化、实参和返回值位置已有明确目标类型时，可以插入有限次内建指针读取，使表达式满足目标；每步读取须合法，最终读出的值须具备 Copy 能力。不能以该规则自动搬出非 Copy 对象，也不搜索任意用户 cast 来满足目标。

无目标上下文时保留原投影结果。`let field = pointer.value;` 得到字段指针；目标为 `u32` 的绑定可以读取其内容。目标本身尚是未知元变量时，不凭便利性把它先固定为某个更深 pointee。

### 执行与其他适配

接收者表达式只求值一次。只执行选中路径上的转换和解引用，顺序与显式展开一致；候选搜索不执行 runtime 动作。产生的效果、依赖和临时对象清理均须保留。

Symbol 到明确 enum 目标、optional/error 成功注入及错误行嵌入按各自限定规则处理，不由本篇合成为任意隐式转换。Cast 的合法性也不单独证明结果保留原存储身份。

## 常见示例

### 多层指针与目标类型

~~~flurry
struct Counter {
    value: u32,

    fn read(*self) -> u32 {
        self.value
    }
}

test {
    let counter = Counter { value: 7 }
    let pointer = counter.ref;
    let outer = pointer.ref;

    let field = pointer.value;
    let snapshot: u32 = pointer.value;
    println("{} {} {}", field.*, snapshot, outer.read());
}
~~~

输出 `7 7 7`。field 保留 `*u32`；snapshot 的明确目标触发 Copy 读取。read 的返回目标同样允许读取字段指针。outer 是多层指针，调用沿内建解引用到达 Counter 方法需要的接收者，没有复制整个 Counter。

### 显式视角先决定偏好

~~~flurry
newtype DecimalCode = u32;
newtype ProtocolCode = u32;

impl DecimalCode {
    fn category(self) -> u32 {
        10
    }
}

impl ProtocolCode {
    fn category(self) -> u32 {
        16
    }
}

test {
    let decimal: u32.as(DecimalCode) = 7;
    let protocol: u32.as(DecimalCode).as(ProtocolCode) = 7;
    println("{} {}", decimal.category(), protocol.category());
}
~~~

输出 `10 16`。第二个绑定优先考虑 ProtocolCode；其适配相对于原始 u32 成立，不要求先转 DecimalCode。转换包含实际行为时，调用必须执行这些行为。

### 来源固定后不重新消歧

~~~flurry
trait WireCode {
    fn code(*self) -> u32;
}

trait DisplayCode {
    fn code(*self) -> u32;
}

struct Status {}

impl WireCode for Status {
    fn code(*self) -> u32 {
        7
    }
}

impl DisplayCode for Status {
    fn code(*self) -> u32 {
        107
    }
}

fn protocol_code(value: *T) -> u32
where T :- WireCode
{
    value.code()
}

test {
    let status = Status {}
    println("{}", protocol_code(status.ref));
    println("{}", status.as(DisplayCode).code());
}
~~~

输出 `7`、`107`。protocol_code 不因 Status 还有 DisplayCode 就变得歧义或改变来源。没有上述约束或显式资格的同名竞争，应按成员规则消歧。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 的方法调用也构造自动解引用和引用候选，但具体搜索次序和借用规则不等于 Flurry 的视角、路径、实现精度三层次序。[Rust method calls](https://doc.rust-lang.org/reference/expressions/method-call-expr.html)

C/C++ 的成员访问和左值读取可作字段访问的对照；Flurry 的指针字段投影先产生字段指针，明确目标再引导 Copy 读取。Zig 有自己的指针访问规则，不能据此推导本篇重载路径。[C11 §6.5.2.3](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf)、[C++ lvalue conversion](https://eel.is/c++draft/conv.lval)、[Zig pointers](https://ziglang.org/documentation/master/#Pointers)

## 运行时实现设计引导

1. 对接收者建立一次稳定求值。
2. 执行选中视角的转换，再按路径执行实际解引用。
3. 必要时取得最终接收者的地址，不移动到无关临时副本。
4. 调用方法，沿所有退出路径清理适配产生的临时值。
5. 将隐式读取的访问前提和效果与显式读取同样处理。

指针字段投影与读值是两步不同操作，优化只能在保留其契约时合并。

## 编译器实现引导

候选状态至少包括原主体、视角、当前形态、动作路径、来源及义务：

1. 在当前视角、当前形态收集合法成员，不执行候选动作。
2. 检查实现适用性；未决保存依赖，确定没有来源才推进路径。
3. 唯一 Deref 结果确定后继续，未知关联结果不能凭空生成下一主体。
4. 选定方法后检查末端 Reference 或内建取址。
5. 合并选定候选的推断结果，保留其他候选的排除依据。
6. 对明确目标位置单独执行内建 Copy 读取适配。
7. 检查动作的阶段、效果和资源状态，再降码。

重复出现相同查询状态应诊断循环。不断产生不同包装类型的路径也可能不终止，必须有展开预算；预算耗尽不是“无成员”，不能据此回退。

缓存记录来源、视角顺序、实现世界和符号类型依赖。程序状态中的对象有效性需在使用点检查，不能从旧的类型匹配缓存中取得许可。
