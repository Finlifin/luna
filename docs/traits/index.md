# Trait 系统

Trait 是 Flurry 实现静态抽象和代码共享的基础机制，类似于其他语言的接口（interface）或类型类（typeclass）。

## 定义 Trait

使用 `trait` 关键字声明方法签名（无实现体）：

```flurry
trait Drawable {
    fn draw(*self, canvas: *Canvas);
}

trait Area {
    fn area(*self) -> f64;
}
```

## 实现 Trait（`impl`）

用 `impl TraitName for TypeName` 为类型提供具体实现：

```flurry
struct Circle { radius: f64 }
struct Rectangle { width: f64, height: f64 }

impl Area for Circle {
    fn area(*self) -> f64 {
        3.1415926535 * self.radius.* * self.radius.*
    }
}

impl Area for Rectangle {
    fn area(*self) -> f64 { self.width.* * self.height.* }
}
```

## Trait 约束

在函数或类型的 `where` 子句中声明类型必须实现哪些 Trait：

```flurry
fn print_area(shape: T) where T:- Area {
    println("area = {}", shape.area());
}

-- `:- Trait` 读作"实现了 Trait"
-- 多个约束：where T:- Area + Drawable
```

## 孤儿原则

`impl T for Type` 必须定义在：
- 定义 `T` 的包，或
- 定义 `Type` 的包

不允许在两者都不属于的第三方包中提供实现，以保证实现的唯一性和一致性。

## `derive` 自动派生

对于有标准实现模式的 Trait（如 `Debug`、`Clone`、`Copy`、`Eq` 等），可以用 `derive` 让编译器自动生成实现：

```flurry
struct Point { x: i32, y: i32 }
derive Debug, Clone, Copy for Point;

let p1 = Point { x: 1, y: 2 }
let p2 = p1.clone();
println("{debug}", p2);
```

## 动态多态

### `dyn Trait`（开放集合）

`dyn TraitName` 是一个胖指针（数据指针 + vtable 指针），可在运行时持有任何实现了 Trait 的类型：

```flurry
let animals: Vec<dyn Speaker> = Vec.new();
animals.push(Box.new(dog).*.dyn(Speaker));
animals.push(Box.new(cat).*.dyn(Speaker));

for a in animals.iter() {
    a.speak();   -- 通过 vtable 动态分派
}
```

优点：开放扩展，任何地方都可以实现。缺点：间接调用开销，无法内联。

### Tagged Polymorphism（封闭集合）

将固定的类型集合封装进枚举，分派基于枚举标签而非 vtable，通常可以直接调用（无间接层）：

```flurry
enum AnyGreeter {
    ^{ tagged_polymorphic: Greeter }
    english: EnglishGreeter,
    spanish: SpanishGreeter,
}

let g: *AnyGreeter = ...;
g.greet();  -- 基于 tag 分派，无 vtable
```

适合类型集合已知且稳定、性能敏感的场景。`comptime` 可以在编译时动态生成此类枚举，兼顾灵活性与性能。

## `using` 委托

在结构体中用 `using field.*` 将字段的公开成员提升到外部接口，减少转发代码（详见 [data-structures/index.md](../data-structures/index.md)）。
