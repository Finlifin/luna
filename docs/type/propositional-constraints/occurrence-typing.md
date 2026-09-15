# 出现位置与分支事实

## Intro

同一个值在不同分支中可以有不同的已知性质。成功匹配之后，程序可以使用该模式提供的信息；离开分支时，这些信息不能无条件保留。

类型汇合与事实汇合需要分别处理：结果类型可以汇合为 Any，而每条操作仍需要它所在路径上的依据。

## 语法

~~~text
bool_matches -> expr matches pattern
if_statement -> if expr block (else (block | if_statement))?
~~~

这是布尔条件相关片段；完整 if、模式条件与 inline 形式见[控制流](../../control-flow/basic.md)及[inline](../../comptime/inlining-comptime-code.md)。

## 正式定义

可提取的条件在对应路径提供正或负事实。模式成功还提供绑定；失败不生成成功绑定。任意 comptime bool 不保证可提取类型事实。

分支事实带有受检对象、绑定及作用域。退出分支后，只能保留所有可达前驱都支持的结论。一个分支建立字段存在，另一个分支没有该依据，不能在汇合后无条件使用字段。

这不把任意可变运行时状态自动纳入精化。对象变化后事实是否仍有效，需要对应资源或验证契约支持。

## 常见示例

数组类型匹配在成功分支引入长度，并让该分支知道元素类型满足要求：

~~~flurry
pure comptime fn word_array_length(T: Type) -> ?usize {
    inline if T is do {
        Array<'u32, length> => length,
        _ => null,
    }
}

test {
    word_array_length(Array<u32, 3>) match {
        length? => println("length: {}", length),
        null => println("not a word array"),
    }

    word_array_length(Array<f32, 3>) match {
        length? => println("length: {}", length),
        null => println("not a word array"),
    }
}
~~~

输出 `length: 3`、`not a word array`。`length` 仅在成功臂存在；它不是因为函数中出现过一次匹配就成为整个函数的绑定。

下面使用数值条件观察前提怎样随分支变化：

~~~flurry
pure comptime fn checked_positive(value: Integer) -> Integer
where requires 0 < value {
    value
}

pure comptime fn positive_or_one(value: Integer) -> Integer {
    inline if value > 0 {
        checked_positive(value)
    } else {
        checked_positive(1)
    }
}

test {
    const FIRST: usize = positive_or_one(7).as(usize);
    const SECOND: usize = positive_or_one(-2).as(usize);
    println("positive values: {}, {}", FIRST, SECOND);
}
~~~

输出为 `positive values: 7, 1`。第一支用条件提供的 `value > 0` 证明调用前提，第二支用具体数值 `1` 证明。两支返回的值都为正，但不能在汇合后据此说原输入 value 为正：输入 `-2` 仍然是允许输入。

这也是绑定与事实需要一起保存作用域的原因。把第一支的正数事实泄漏给第二支，会错误接受本应缺少前提的调用。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 模式提供分支绑定，C/C++ 条件分支与 Zig 编译时分支也分别改变控制路径；Flurry 在准入条件上还提取 PTC 事实。普通布尔判断不会自动获得任意运行时精化能力。[Rust 模式](https://doc.rust-lang.org/reference/patterns.html)

## 运行时实现设计引导

静态路径事实可以消去或帮助优化，但必须对应实际控制流。不能以某个编译时未执行分支的局部假设优化另一运行时分支。

## 编译器实现引导

进入分支建立 activation guard，记录新事实及绑定；退出恢复父环境。汇合通过各前驱的证明支持保留共同结论，而不是单纯取事实文本交集：不同证明也可能支持同一命题。

已知不可达路径只在有合法不可达依据时排除。学习子句保留 guard，缓存包含对象身份；回退需要恢复全部相关 theory 状态。

验收包括正负分支、嵌套绑定、不可提取条件、不同证明支持共同结论和局部事实不得泄漏。
