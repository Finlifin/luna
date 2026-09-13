# 数据结构

## 结构体（`struct`）

结构体将多个不同类型的值组合成一个有名的复合类型：

```flurry
struct Point {
    x: f64,
    y: f64,
}

struct UserProfile {
    username: String,
    email: String,
    is_active: bool,
    login_count: u64,
}
```

**实例化**：使用 `TypeName { .field value, ... }` 语法：

```flurry
let origin = Point { .x 0.0, .y 0.0 }
let user = UserProfile {
    .username "alice".to_string(),
    .email "alice@example.com".to_string(),
    .is_active true,
    .login_count 5,
}
```

**字段访问**：`.` 操作符：

```flurry
println("{}, {}", origin.x, origin.y);
```

**方法**：直接在 `struct { }` 内部定义，或通过独立的 `impl` 块添加：

```flurry
struct Rectangle {
    width: u32,
    height: u32,

    pub fn area(*self) -> u32 { self.width * self.height }
    pub fn square(size: u32) -> Self { Rectangle { .width size, .height size } }
}

let rect = Rectangle { .width 10, .height 5 }
println("{}", rect.area()); -- 50
```

**`using` 委托**：将内部字段的公开成员提升到外部结构体接口，减少转发样板：

```flurry
struct Car {
    engine: Engine,
    transmission: Transmission,
    using engine.*;           -- 将 Engine 的公开方法提升到 Car
    using transmission.*;     -- 将 Transmission 的公开方法提升到 Car
}

my_car.start();      -- 实际调用 my_car.engine.start()
my_car.shift_up();   -- 实际调用 my_car.transmission.shift_up()
```

---

## 枚举（`enum`）

枚举定义一个类型可以是若干**变体**之一：

```flurry
-- 简单状态枚举
enum Status { ok, pending, error }

-- 带关联数据的枚举（代数数据类型）
enum Shape {
    circle(radius: f64),
    rectangle(width: f64, height: f64),
    point,
}
```

**实例化**：

```flurry
let s = Status.pending;
let r = Shape.rectangle(10.0, 5.5);
```

**层级化枚举**：用 `.` 分隔层级，避免命名空间污染：

```flurry
enum AstNode {
    expr.{
        literal(LiteralKind),
        binary_op(Op, *AstNode, *AstNode),
    },
    stmt.{
        let_binding(String, *AstNode),
        return_stmt(?*AstNode),
    },
}

let node = AstNode.expr.literal(LiteralKind.integer(10));
```

**方法**：枚举同样可以定义关联函数：

```flurry
enum IpAddr {
    v4(u8, u8, u8, u8),
    v6(String),

    pub fn loopback() -> Self { IpAddr.v4(127, 0, 0, 1) }
}
```

枚举最常与模式匹配配合使用，见 [control-flow/pattern-matching/](../control-flow/pattern-matching/)。

---

## 数组（`Array<T, N>`）

数组大小固定，是类型的一部分（编译时确定）：

```flurry
let primes: Array<i32, 4> = [2, 3, 5, 7];
let zeros: Array<i8, 100> = Array.fill(0);

println("{}", primes[2]); -- 5
println("{}", primes.len()); -- 4
```

---

## 切片（`Slice<T>`）

切片是对连续内存序列的非拥有视图，大小在运行时确定：

```flurry
let array = [1, 2, 3, 4, 5];
let full:    Slice<i32> = array[..];
let partial: Slice<i32> = array[1..3]; -- 包含 [2, 3]
let tail:    Slice<i32> = array[2..];  -- 包含 [3, 4, 5]

fn sum(nums: Slice<i32>) -> i32 {
    let total = 0;
    for n in nums { total += n; }
    total
}
```

切片适合作为接受任意长度序列的函数参数，避免复制或转移所有权。

---

## 元组

元组是固定数量、异构类型值的有序组合：

```flurry
let pair: (i32, str) = (42, "hello");
let (x, y) = pair;   -- 解构
let first = pair.0;  -- 按索引访问
```

空元组 `()` 即 `void` 类型的唯一值。

---

## 联合体（`union`）

联合体的所有字段共享同一块内存，同一时刻只有一个字段有效。联合体访问通常需要在 `unsafe` 上下文中，或通过有标签的枚举来安全地封装：

```flurry
union IntOrFloat {
    i: i32,
    f: f32,
}
```

---

## 模块作为类型（`mod`）

`mod` 可以像命名空间一样组织函数和常量，同时也可以作为类型使用，包含字段和方法：

```flurry
mod MathUtils {
    pub const PI: f64 = 3.14159265358979;
    pub fn square(x: f64) -> f64 { x * x }
}

println("{}", MathUtils.PI);
println("{}", MathUtils.square(3.0));
```
