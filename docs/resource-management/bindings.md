# 变量绑定

Flurry 中，将值与名称关联的操作称为**绑定（binding）**。

## `let` / `var` 绑定

`let` 与 `var` 声明绑定；左侧是**模式**，因此可以解构。可反驳模式可在末尾接 `else`（类比 Rust `let … else`，详见 [模式匹配](../control-flow/pattern-matching/index.md)）。

```flurry
let message = "Hello, Flurry!";
let count = 0;
let pi: f64 = 3.14159;
let (x, y) = position;
var { name, age } = student;

let head? = maybe_head else {
    return;
}
```

- **类型推断**：可以省略类型注解，编译器从初始化表达式推断。
- **类型注解**：用 `: Type` 显式指定；在函数签名、结构体字段等位置通常必须显式标注。
- **`var`**：可重新赋值的绑定（与 `let` 的可变性分工以内存/绑定专章为准；模式与 `else` 规则两者相同）。

## `const` 绑定

`const` 声明一个编译时常量：

```flurry
const MAX_USERS: usize = 1000;
const APP_NAME: str = "Flurry Core";
```

- 初始化表达式必须在编译时可完全求值。
- 值不可在运行时改变。
- 通常需要显式类型注解。

`const` 是 Flurry `comptime` 能力的基础体现：常量可以参与编译时计算，用作泛型参数等。

## 作用域与遮蔽

绑定遵循词法作用域，在花括号 `{}` 限定的块内有效。内层可以用 `let` 遮蔽（shadow）外层的同名绑定：

```flurry
let x = 10;
{
    let x = 5;       -- 遮蔽外层的 x
    println("{}", x); -- 输出 5
}
println("{}", x);    -- 输出 10，内层遮蔽已结束
```

遮蔽允许对值进行类型转换或重新绑定，而无需使用不同的名称。

