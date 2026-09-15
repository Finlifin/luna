# newtype 与 typealias

## Intro

typealias 给已有类型另一个名字。newtype 建立需要独立区分的类型，同时保持与底层类型的直接表示关系。

当程序要区分两个同样编码的概念时，选身份隔离；当程序只想给复杂类型计算结果命名时，别名更合适。

## 语法

~~~text
typealias_def -> typealias id (<parameter*>)? = expr
newtype_def -> newtype id (<parameter*>)? = expr
as_expr -> expr.as(expr)
~~~

参数与表达式沿用全局语法。

## 正式定义

typealias 不增加类型身份。参数化 alias 的目标仍按其表达式的计算与身份规则确定。

newtype 与底层类型及其他 newtype 声明具有不同身份；保持底层本体的大小、对齐和合法值编码。这个保证不推广为任意同大小类型可以互转。

newtype 不继承底层字段使用、构造、方法或 trait 实现。构造先得到底层值再 cast；使用底层接口先 cast 回底层。独立定义的新接口与资源操作遵循自己的契约。

纯类型工厂的重复调用沿用缓存及结果复用规则，alias 不额外改变该模型。

## 常见示例

把用户编号与订单编号都保存成 `u32`，同时要求接口区分二者：

~~~flurry
typealias Count = u32;
newtype UserId = u32;
newtype OrderId = u32;

fn show_user(id: UserId) {
    println("user {}", id.as(u32));
}

fn show_order(id: OrderId) {
    println("order {}", id.as(u32));
}

test {
    let count: Count = 42;
    let user = count.as(UserId);
    let order = count.as(OrderId);

    show_user(user);
    show_order(order);
}
~~~

两个接口分别输出 `user 42`、`order 42`。数值相同，但参数要求不同。两个函数都在内部显式回到 `u32`，供底层数值接口使用；调用者不再使用已经交出的 newtype 值。

以下调用分别是静态错误：

~~~flurry
test {
    let order = 42u32.as(OrderId);
    show_user(order);
}

test {
    let count: Count = 42;
    show_user(count);
}
~~~

`Count` 没有新身份，`OrderId` 有自己的身份；二者都不能凭宽度相同就充当 `UserId`。

表示关系也不带来成员继承：

~~~flurry
struct Point {
    x: i32,
    y: i32,

    fn horizontal(self) -> i32 {
        self.x
    }
}

newtype ScreenPoint = Point;

test {
    let screen = (Point { x: 12, y: 8 }).as(ScreenPoint);
    println("x: {}", screen.as(Point).horizontal());
}
~~~

调用先显式取得 `Point` 视角，才使用该类型的方法。`screen.horizontal()` 不能由 newtype 声明本身获得调用依据。

## 与 Rust / Zig / C / C++ 的关键差异

Rust tuple struct 和 C++ 包装类可建立独立身份，但表示保证需看各自规则。Rust alias、C typedef 和 C++ alias 则是命名手段；Zig 可给类型值命名。Flurry newtype 的直接表示与接口隔离同时成立。[Rust alias](https://doc.rust-lang.org/reference/items/type-aliases.html)、[C++ alias](https://eel.is/c++draft/temp.alias)、[Rust 布局](https://doc.rust-lang.org/reference/type-layout.html)

## 运行时实现设计引导

允许复用底层存储布局，但不复用未经定义的行为入口。转换是否需要运行时指令由合法降码决定；零指令转换也必须保留静态身份变化。

## 编译器实现引导

alias 解析到目标，newtype 保存独立声明身份及底层表示边。结构和接口查询不能无条件穿透该边。

类型等式、布局查询、cast 和成员解析分别访问所需信息。验收应同时检查：alias 相等、newtype 不等、布局一致、字段不继承、trait 不自动继承以及合法双向转换。
