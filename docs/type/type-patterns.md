# 类型模式

## Intro

类型模式在编译时观察类型已经暴露的形态，把其中信息带入当前分支。匹配不等于逆向执行工厂：一个计算返回某结构，不代表能从结构唯一恢复全部输入。

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

这些函数观察的是 Array 保留的形态。[类型身份](type-identity.md#常见示例)中的普通工厂 `StorageFor` 会丢弃输入信息，不能套用同一思路从它返回的 `u32` 逆解输入。

成功的常量 Type 模式也为同一保留头中的后续依赖参数提供类型依据：

```flurry
typealias Type = .Type'builtin;
typealias Bool = bool;
pure comptime fn Box<T, value: T> -> Type { T }

comptime fn Contents(value: Type) -> Bool {
    value match {
        Box<('Bool as Kind), contents> => Kind == Bool and contents,
        _ => false,
    }
}
```

匹配第一个参数成功后，第二个参数按 Bool 检查。`as` 绑定和额外 guard 保留这一依据；guard 失败仍退出该候选。`('Bool or 'Bool)` 的各分支提供同一类型，也可以继续检查后面的值。不同类型的 `or` 不提供唯一后续域；`(_ if Bool == Bool)` 没有限定受检 Type 的身份，更不能充当依据。normal-form 头保持不透明，不能把 `Identity<Bool>` 与 Bool 合并来满足这一条件。

串联模式保留左侧已经建立的依据，例如 `('Bool and condition is true)`。它也能先绑定受检 Type，再直接观察同一绑定：

```flurry
comptime fn ContentsOr(value: Type, fallback: T) -> T where T {
    value match {
        Box<(Kind and Kind is 'T), contents> => contents,
        _ => fallback,
    }
}
```

这里后半段观察的仍是原先的 Kind，匹配成功才允许将 contents 按 T 处理。若改成 `Kind and Bool is 'Bool`，后半段只检查另一个表达式，不为 Kind 提供任何依据。中间包含改写绑定的计算或 guard 时，当前实现不会擅自追踪这种等同关系。串联的后半段失败仍使整个候选失败，后续候选重新建立自己的依据。

若受检表达式已经保留了可确定的 normal-form 头和参数，模式也能直接利用这些参数形成后续字段的域：

```flurry
comptime fn KnownContents() -> bool {
    Box<bool, true> match {
        Box<Kind, contents> => Kind == bool and contents,
        _ => false,
    }
}
```

这里 contents 按 bool 检查，无需先写一个 bool 常量模式。嵌套头、let 解构和串联模式的新受检表达式同样适用；命名参数按声明对应关系取得上下文，不依赖书写顺序。此观察有预算限制，保留原表达式的执行计划，不调用工厂或展开 normal-form 头。`Identity<Box<bool, true>>` 因而不能提供 Box 的参数。

类型元组模式会把已知元素类型继续传给嵌套模式。例如：

```flurry
pure comptime fn Wrap<T> -> Type { T }

comptime fn NestedContents() -> bool {
    Wrap<(Box<bool, true>, bool)> match {
        Wrap<(Box<Kind, contents>, _)> => Kind == bool and contents,
        _ => false,
    }
}
```

Wrap 的 Type 参数为元组提供类型上下文；此处元组表达式构造的是类型。嵌套的 Box 据此获得 bool 参数。绑定 Tuple 协议时，这一操作只读取协议定义的元素视图，仍保留 Tuple 的头；其它工厂的返回体不充当元组视图。元组元素数量不匹配时，不把部分元素冒充成功候选的上下文。

前面的完成值参数也可以决定后面的 nominal 参数类型。例如，布尔参数决定 Config 的实例身份：

```flurry
typealias Type = .Type'builtin;
struct Config where enabled: bool { flag: bool }
pure comptime fn Packet<enabled: bool, value: Config<enabled>> -> Type { bool }

comptime fn inspect(input: Type) -> bool {
    input match {
        Packet<true, item> => item.flag,
        _ => false,
    }
}
```

成功匹配 true 后，item 按 `Config<true>` 检查；匹配失败不会把这条依据带给其他候选。整数等能确定唯一保留值的常量模式、整体别名和一致的 `or` 分支也可提供依据；不同值的 `or` 不能推导唯一后续域。已知受检值可提供实际参数，所以直接匹配已完成的 Packet 时，前面的普通绑定也能为后续参数提供类型。浮点相等不保证唯一位表示，不能仅凭浮点常量模式推导保留值身份。

构造这些 normal-form 应用时，位置输入、晚到的具名输入和默认输入使用保留 Type／值 sort 的依赖替换。值生产函数仍按输入执行顺序求值一次；替换不会调用 normal-form 头的函数体。参数值与对应 nominal 实例不一致时仍拒绝。

当前实现只使用已检查模式或可确定受检值提供的候选内证据，不向全局求解环境添加假设。对于任意 Type 输入，一般 `Box<T, contents>` 中动态绑定的 T 尚不能据此形成 contents 的域；未覆盖的依赖模式仍给出明确诊断。

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


### 参数模式中的候选绑定

后面的参数模式可以观察前面参数建立的绑定，例如：

```flurry
pure comptime fn Pair<Left, Right> -> Type { Left }

comptime fn Same(value: Type) -> bool {
    value match {
        Pair<A, (B if A == B)> => true,
        _ => false,
    }
}
```

这里比较的是保留在 normal form 中的两个参数，不调用 Pair 的函数体。值参数和嵌套 normal-form 模式也遵循这一规则。guard 失败时，当前候选的绑定不会泄漏到后续候选。

当前实现按源码顺序建立名字的词法可见性，按参数声明顺序执行匹配。命名参数可以重排，但 guard 引用的绑定必须在这两个顺序中都已建立；引用尚未匹配的参数会被拒绝。重排命名参数本身不改变匹配顺序。
