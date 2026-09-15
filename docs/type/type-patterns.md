# 类型模式

## Intro

类型模式观察类型已经暴露的形态，把其中信息带入当前分支。匹配不等于逆向执行工厂：一个计算返回某结构，不代表能从结构唯一恢复全部输入。

## 语法

局部产生式如下，普通模式见[模式匹配](../control-flow/pattern-matching/index.md)。

~~~text
bool_matches -> expr matches pattern
normal_form_head_application_pattern -> expr<type_pattern_argument*>
type_pattern_argument -> id: pattern
    | type_pattern_expression
    | pattern
type_pattern_binding -> id
type_pattern_expression -> 'id
~~~

参数模式中的 id 引入绑定；单引号形式引用已有表达式，不是跨阶段 quote 或模式回溯引用。

## 正式定义

类型可以作为受检值。保留范式的头可以是表达式；头观察要求受检项携带该形态，不能只依靠结果类型语义相等。

成功匹配提供绑定、结构观察与关系事实，并确定其作用域。失败分支不获得成功绑定。普通纯工厂按执行结果处理；结果相等不推出输入相等，不同工厂也可能返回同一类型。

只有分类信息不能假设长度或元素；开放行已知某标签不能否定其他标签。观察仍须逐项满足前提。

未知不等于不匹配。需要决定的位置须取得依据，允许延迟的位置保留匹配及依赖。

## 常见示例

数组的保留头允许模式同时检查元素类型并绑定长度。下面的编译时函数只接纳元素为 `u32` 的数组类型：

~~~flurry
pure comptime fn word_array_length(T: Type) -> ?usize {
    inline if T is do {
        Array<'u32, length> => length,
        _ => null,
    }
}

test {
    let accepted = word_array_length(Array<u32, 4>);
    let rejected = word_array_length(Array<f32, 4>);

    accepted match {
        length? => println("word count: {}", length),
        null => println("not a word array"),
    }

    rejected match {
        length? => println("word count: {}", length),
        null => println("not a word array"),
    }
}
~~~

输出为 `word count: 4`、`not a word array`。模式中的 `length` 引入绑定；`'u32` 引用已有的类型表达式，要求元素与它匹配。若写成 `Array<Element, length>`，两个位置都会产生新绑定，不能再把 `Element` 当作预先给定的类型要求。

也可以把目标元素类型作为外部输入：

~~~flurry
pure comptime fn array_length_of(T: Type, Expected: Type) -> ?usize {
    inline if T is do {
        Array<'Expected, length> => length,
        _ => null,
    }
}

test {
    array_length_of(Array<f32, 8>, f32) match {
        length? => println("sample count: {}", length),
        null => println("element type differs"),
    }
}
~~~

这次输出 `sample count: 8`。`Expected` 从函数参数取得，单引号让模式使用这个值，而不是引入一个同名局部绑定。

这些函数观察的是 Array 保留的形态。[类型身份](type-identity.md#常见示例)中的普通工厂 `storage_for` 会丢弃输入信息，不能套用同一思路从它返回的 `u32` 逆解输入。

## 与 Rust / Zig / C / C++ 的关键差异

C++ 模板偏特化有按类型形态选择定义的用途；Rust 普通模式处理值。C 可使用显式标签或生成代码，Zig 可反射后分支。Flurry 的保留头和普通计算结果须分别解释。[C++ 偏特化](https://eel.is/c++draft/temp.spec.partial)、[Rust 模式](https://doc.rust-lang.org/reference/patterns.html)

## 运行时实现设计引导

编译时类型匹配可以决定实例代码，不必留下运行时分支。原本依赖运行时值的条件仍按控制流契约执行。

## 编译器实现引导

1. 解析头表达式。
2. 参数模式优先识别命名项，再处理一般模式。
3. id 建立绑定，表达式引用在外层解析。
4. 根据合法头形态或结构观察递归检查。
5. 等式、长度、位模式交对应 theory。
6. 完整成功后把绑定引入分支。

结果携带状态、绑定、来源与残余。类型等式不能抹掉头观察溯源；有限展开不足时不能宣布完整匹配失败。
