# union

## Intro

union 为多个成员提供共享存储。它适合对接 C 数据布局，但不会像 enum 一样由类型自动提供每次读取所需的变体判别。

选择某成员构造一个 union，与随后解释共享存储，是不同操作。

## 语法

~~~text
union_def -> union id clauses? { (configuration | union_variant | definition | statement)* }
union_variant -> id: expr
union_construction -> expr.id(expr)
~~~

成员用逗号分隔；其他定义与配置沿用全局语法。

## 正式定义

union 具有声明身份，成员需要相应固定大小表示，布局遵循 C ABI。成员类型的大小尚未求出与其表示依赖值信息，应按[布局](../reflection-and-layout.md)分别处理。

通过指定成员构造值。构造以外的 union 实例操作要求 unsafe。该规则针对 union，不由此推导所有指针操作的许可规则。

union 赋值不执行旧成员的 drop。它没有自动替程序确定旧存储当前应按哪个成员释放的通则。进入 unsafe 也不能凭空授予任意位串作为任意成员值的合法性。

union 实例不作为一般 comptime 值执行。类型描述和成员元数据是否能被某个查询观察，仍须逐个满足那个查询的定义域，不能把实例限制扩展成所有类型信息都不存在。

## 常见示例

下面的值槽允许存放一个整数或一个浮点数。构造时明确选择成员；重新写入共享存储放在 unsafe 操作内：

~~~flurry
union Number {
    integer: i32,
    real: f32,
}

unsafe fn replace_with_real(slot: *Number, value: f32) {
    slot.* = Number.real(value);
}

test {
    var slot = Number.integer(12);

    unsafe {
        replace_with_real(slot.ref, 1.5);
    }

    println("storage size: {}", Type.size_of(Number));
}
~~~

初始构造写入整数 `12`，随后赋值以浮点数 `1.5` 的表示替换共享存储。程序没有读取另一成员来猜测它的值，也没有要求赋值自动清理旧成员。大小查询观察 `Number` 类型在目标 C ABI 下的布局。

若应用需要在稍后判断该值是什么，带载荷 enum 可以直接保存这项区分：

~~~flurry
enum TaggedNumber {
    integer(i32),
    real(f32),
}

fn show_number(number: TaggedNumber) {
    number match {
        .integer(value) => println("integer: {}", value),
        .real(value) => println("real: {}", value),
    }
}

test {
    show_number(TaggedNumber.integer(12));
    show_number(TaggedNumber.real(1.5));
}
~~~

第二段的匹配依据来自 enum 的变体身份。不能把这段匹配所需的判别信息视为第一段 union 声明自动提供的内容。

## 与 Rust / Zig / C / C++ 的关键差异

C union 是 ABI 对接的对应物；C++ union 还涉及成员生命周期规则。Rust union 的字段和访问限制有其独立规定；Zig 区分普通、tagged 与 extern union。Flurry 的赋值不执行旧成员 drop，不能借这些语言的规则补出活跃成员与清理语义。[Rust union](https://doc.rust-lang.org/reference/items/unions.html)、[C++ union](https://eel.is/c++draft/class.union)

## 运行时实现设计引导

按目标 C ABI 分配满足成员大小与对齐要求的共享存储，不无条件套用未经目标核对的最大值公式。构造向选择的成员区域写入相应值。

赋值降码不能隐式加入旧成员析构。其他访问和清理必须来自明确的操作及资源契约；共享存储本身不携带可依赖的自动活跃成员状态。

## 编译器实现引导

1. 建立名义身份及成员表。
2. 检查成员的固定大小表示前提。
3. 请求目标 C ABI 的 union 布局。
4. 构造解析到具体成员并检查输入。
5. 对构造外实例操作执行 unsafe 要求。
6. 将赋值与资源清理分开，防止错误插入旧成员 drop。

输入未知时保留布局义务；非法按值递归布局拒绝。验收应覆盖构造成员选择、ABI 对齐、缺失固定大小依据和赋值降码不自动析构。
