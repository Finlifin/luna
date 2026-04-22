# 属性系统
属性是用于为代码元素添加元数据的一种机制，可以在编译时对这些元数据进行处理和使用。用`^`前缀来给一个item、语句、参数等添加属性：
```flurry
-- 标记一个结构体为packed_struct，表示它的字段将被紧密排列在内存中，不会有填充字节。
^attrs.packed_struct
struct PackedStruct {
    a: u8,
    b: u16,
}

test {
    when {
        -- 标记热点分支
        ^attrs.likely a > b => "a is likely greater than b",
        ^attrs.unlikely a < b => "a is unlikely less than b",
        _ => "a and b are equal",
    }
}
```

`^`需要后接一个表达式，该表达式会在编译期被求值，并且其结果的类型是Object, 其键值对就是属性的名称和值。属性可以被编译器或其他工具在编译时读取和使用，以实现各种功能，如优化、代码生成、文档生成等。

属性的应用顺序，如果一个item、语句、参数等有多个属性，那么这些属性的应用顺序是从右到左的。也就是说，最后一个属性会最先被应用，而第一个属性会最后被应用。这种顺序对于某些属性的行为可能会有影响，因此需要注意。
```flurry
^{ some_attr: 1 }
^{ some_attr: 2 }
-- meta.attrs_of(example) => { some_attr: 1 }
fn example() {
    ...
}


```