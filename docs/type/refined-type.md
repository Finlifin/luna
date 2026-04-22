为了在极端情况下简化一些问题，flurry引入了鸭子类型来在类型系统中表达一些模糊的定义。
```flurry
-- 鸭子类型不可做返回值
-- 慎用鸭子类型，因为只有当foo单态化时，编译器才能完整检查foo函数的正确性
fn foo(x: Any) {
    -- 模式匹配x的类型
    inline if x'type is do {
        Vec<i32> => ...
        -- Type类型的模式匹配, id规则不同普通类型，为了方便起见，id会被解析为对应的值，其可能是一个类型，如果需要绑定出来，需要使用`' id`的形式
        -- 解构出Array类型的参数N
        Array<i32, 'N> => ...
        _ => ...
    }
}
```

鸭子类型的意义可以通过println的例子来理解。观察println的类型`fn(comptime format: str, ...args: Parse(format))`,
我们可以知道，println是依赖类型，通过编译时计算系统，编译器执行Parse(format)来计算出println的参数类型（运行时函数的varadic参数会被收集为一个元组，所以依然视作单个参数）。考虑`println("{}", 1)`这样的format, 显然我们期望println的参数类型是`for<T:- Display> Tuple<T>`，然而，flurry放弃了higher-ranked types的支持，所以我们无法直接表达这个类型，取而代之的，我们可以使用鸭子类型来表达这个类型。
但是`Parse("{}")`如果只是单纯地返回一个`Tuple<Any>`，我们依然会损伤很多类型信息，这就是refined type的意义了。refined type允许我们在鸭子类型的基础上添加一些约束来表达更多的类型信息。`Parse("{}")`实际上会被求值为`Tuple<Any refines { Self:- Display }>`
```flurry
struct Foo {
    age: isize,
    name: String
}

test {
    -- Parse("{any}, {}, {x}") => Tuple<Any, Any refines { Self:- Display }, Any refines { Self:- Display and Type.is_integer(Self) }>
    println("{any}, {}, {x}", Foo { age: 18, name: "luna".to_string() }, false, 16);
}

```
