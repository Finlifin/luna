# 接收者适配与隐式转换

## Intro

一些成员访问可以从值自动取得指针，或沿明确给出的类型视角找到成员。读懂这些调用，需要知道适配后的接收者是什么，以及最终访问哪个接口。

这里集中说明类型相关的隐式转换。取址、解引用或转换是否获得操作许可，仍需满足相应程序契约；一个类型关系不能独自证明对象仍然有效。

## 语法

~~~text
take_reference -> expr.ref
dereference -> expr.*
as_expr -> expr.as(expr)
projection -> expr.id
application -> expr(argument*)
~~~

类型视角沿用 as 表达式，可出现在已有类型标注中。expr 与绑定产生式同全局语法。

## 正式定义

### 值和指针

值的成员需要指针接收者时，可以通过取址适配。指针访问结构字段时，指针关系传播到字段：结果是字段指针，而不只是字段内容。对指针进行相应结构匹配时，字段绑定同样具有指针关系。

Deref 和 Reference 的重载能力通过相应显式操作使用，不自动加入一条无限的成员查找链。

### 类型视角

对于类型标注 A.as(B).as(C).as(D)，绑定值本身的类型仍是 A。B、C、D 是相对于 A 的 cast candidates，分别需要与 A 具有双向 cast 关系。

查找按 D、C、B、A 的顺序考虑视角，选定来源后执行相应转换与访问。不能把它解释成只要求 A↔B、B↔C、C↔D 的相邻转换链。

newtype 的直接表示关系不提供底层字段或方法继承。使用底层接口须经与底层类型的 cast 接合。

### 其他适配

有明确 enum 目标时，Symbol 可以参与对应变体适配。optional 成功注入、error 成功注入和错误行嵌入按[各自限定](qualified-types/optional-type.md)处理，不能合成一个任意跨类型转换通则。

一般 cast 的合法性不单独证明结果保留原存储身份；别名、可达性与资源条件由相应操作契约约束。

## 常见示例

为范围类型定义一个指针接收者方法，再从值调用它：

~~~flurry
struct Range {
    from: usize,
    to: usize,

    fn length(*self) -> usize {
        self.to.* - self.from.*
    }
}

test {
    let range = Range { from: 3, to: 10 }
    let pointer = range.ref;
    let start: *usize = pointer.from;

    println("start: {}", start.*);
    println("length: {}", range.length());
    println("explicit length: {}", range.ref.length());
}
~~~

输出为 `start: 3`、`length: 7`、`explicit length: 7`。`pointer.from` 取得字段指针，读取内容还要 `.*`。`range.length()` 则为已选定的指针接收者取得 `range.ref`。

类型视角可以用两个自定义接口观察查找顺序。两个 newtype 都与原始 `u32` 具有直接双向 cast 关系：

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

    println("decimal category: {}", decimal.category());
    println("protocol category: {}", protocol.category());
}
~~~

输出为 `decimal category: 10`、`protocol category: 16`。第二个绑定先考虑末尾的 `ProtocolCode`。`DecimalCode` 与 `ProtocolCode` 都是相对于原始 `u32` 的候选，不要求程序先把一个 newtype 转成另一个。

该例的方法返回不同常量，只用于显露成员选择。若转换包含实际行为，调用还必须执行所选转换，不能把视角仅当作忽略转换的名称查找提示。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 方法查找包含自动解引用与借用候选；这不能用来推导 Flurry 自动调用 Deref/Reference。C 的指针字段访问取得字段左值，C++ 还支持运算符重载；Flurry 的字段指针传播须明确辨认。Zig 可使用显式指针操作。[Rust 方法查找](https://doc.rust-lang.org/reference/expressions/method-call-expr.html)

## 运行时实现设计引导

降码保留接收者动作序列与实际结果类型。字段指针应由合法基址及字段位置产生，不能误降成读取字段值。

视角转换需要执行其对应操作；相同表示可能允许消除指令，但只有在保持转换契约、调用来源和对象关系时才可消除。

## 编译器实现引导

将“查找来源”和“适配动作”分开表示：

1. 记录原主体类型及有序视角。
2. 检查每个候选相对于原类型的 cast 前提。
3. 按既定来源规则选择成员。
4. 为所选接收者生成取址、解引用、cast 或字段投影动作。
5. 保留动作产生的类型与程序前提。
6. 在实例化和降码时兑现动作。

禁止从本篇类型规则推导未定义的安全许可差别、存储身份保证或任意回退策略。诊断应能显示实际需要的接收者以及已选择来源。
