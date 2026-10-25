# 类型身份与类型关系

## Intro

两个值在内存里长得一样，不代表它们属于同一个类型。Flurry 采用名义类型系统（nominal type system），以声明而非结构决定身份。类型身份使程序能够区分位置与速度、句柄与计数，以及碰巧具有相同字段的两个声明。

Flurry 同时允许计算类型。比较计算结果时，需要知道结果本身的身份，也需要知道哪些计算仍以可观察的范式保留。

## 语法

~~~text
typealias_def -> typealias id (<parameter*>)? = expr
newtype_def -> newtype id (<parameter*>)? = expr
normal_form_head_application -> expr<argument*>
application -> expr(argument*)
for_type -> for<parameter*> expr
~~~

尖括号应用与普通调用的完整约定见[纯性与范式](../comptime/purity-and-normal-form.md)。

## 正式定义

### 身份来源

| 类型来源 | 身份规则 |
|---|---|
| 独立 struct、enum、union 声明 | 各有声明身份；字段或变体相同不自动相等 |
| 同一参数化声明的应用 | 按同一声明及其语义参数比较；参数中的运行时依赖遵循对象 identity |
| typealias | 目标类型的另一个名字；不增加身份 |
| newtype | 独立身份；保持底层类型本体的大小、对齐及合法值编码 |
| 普通纯类型工厂调用 | 按普通编译时执行与缓存模型取得结果；复用同一调用的结果 |
| 保留的范式应用 | 保留可观察的头与实参；不展开头的函数体来化简应用 |

参数比较必须采用参数所属语义域的等式。两个运行时对象当前内容一样，不足以替换对象 identity；两个形状表达式拼写不同，也不必因此不等。

范式应用不因函数体返回某个参数，就与该参数建立类型关系。别名指向该应用时也遵守这一规则；alias 的透明性只消除别名本身。

元组也遵循这条规则。`Tuple` 的签名是 `pure comptime fn Tuple<...Es: meta.List<Type>> -> Type`：例如 `Tuple<u32, bool>` 将两个元素类型收集为一个 `meta.List<Type>` 输入，并保留 `Tuple` 的范式头。用于字段访问、解构和布局的元素结构视图不能替代这个类型身份，也不能通过展开 `Tuple` 的函数体建立类型相等关系。

括号不会因目标域是 Type 就构造类型：`(u32, bool)` 是包含两个 Type 值的 tuple，不能代替 `Tuple<u32, bool>`。单元素类型写作 `Tuple<u32>`，空 tuple 类型写作 `Tuple<>`；`(u32)` 只是分组，`(u32,)` 和 `()` 是值。tuple 值的构造与模式解构继续使用括号。

普通类型工厂按执行结果比较：不同工厂或不同输入可能得到同一个类型。因此不能仅凭普通调用的函数名不同断言结果不等，也不能从结果相等反推输入相等。普通执行的这些性质不构成展开范式应用的依据。

for<parameter*> expr 是纯编译时函数的简写。它描述参数到结果的计算，调用和类型关系仍遵循相应纯函数与范式规则；不能由这个简写引入另一套运行时多态交付机制。

### 五种不同的问题

| 关系 | 它回答什么 | 不会自动给出什么 |
|---|---|---|
| 相等 | 是否同一个语义类型 | 一段任意转换程序 |
| 匹配 | 受检项是否满足此模式，并产生哪些绑定 | 工厂的通用逆函数 |
| 相容 | 当前检查能否接受这一描述 | 相等、传递闭包或表示相同 |
| 子类型 | 是否满足已定义的方向性接纳关系 | 对所有类型构造子的协变规则 |
| 转换 | 怎样把源值交给目标要求 | 源与目标相等、原存储身份保留 |

Any 与两个不同类型分别相容，不能借助传递性把那两个类型合并。字段集合包含、错误行包含以及成员能力满足也不能相互代替。

## 常见示例

先用调用结果观察 alias 与 newtype 的区别：

~~~flurry
struct UserId {
    value: u32,
}

struct OrderId {
    value: u32,
}

typealias CustomerId = UserId;
newtype ArchivedUserId = UserId;

fn show_user(id: UserId) {
    println("user {}", id.value);
}

test {
    let customer = CustomerId { value: 7 }
    show_user(customer);

    let archived = (UserId { value: 8 }).as(ArchivedUserId);
    show_user(archived.as(UserId));
}
~~~

输出为 `user 7`、`user 8`。alias 构造的值直接满足参数要求；newtype 值先显式回到目标类型。把 `OrderId { value: 7 }` 直接传给 `show_user` 则是静态错误，即使字段完全相同。

纯工厂的结果还可以用编译时检查直接比较：

~~~flurry
pure comptime fn PairOf(T: Type) -> Type {
    struct Pair {
        first: T,
        second: T,
    }

    Pair
}

typealias FirstPair = PairOf(u32);
typealias SecondPair = PairOf(u32);
typealias FloatPair = PairOf(f32);

comptime {
    assumes FirstPair == SecondPair;
    assumes FirstPair != FloatPair;
    assumes CustomerId == UserId;
    assumes ArchivedUserId != UserId;
}

test {
    let pair: SecondPair = FirstPair { first: 4, second: 6 }
    println("pair: {}, {}", pair.first, pair.second);
}
~~~

相同纯调用复用同一结果，因而跨两个 alias 的赋值成立。`PairOf(f32)` 使用不同字段类型，不能复用该实例。

并非所有类型工厂都对输入保持单射：

~~~flurry
pure comptime fn StorageFor(T: Type) -> Type {
    u32
}

comptime {
    assumes StorageFor(i32) == StorageFor(f32);
}
~~~

两个不同输入都得到 `u32`。因此从结果相等逆推出 `i32 == f32` 会产生错误结论。对保留范式的合法观察见[类型模式](type-patterns.md)。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 的 type alias 和 C++ alias 都提供已有类型的名称，这可以直接帮助理解 Flurry 的 typealias。建立隔离接口的类型则需要另一种声明；Flurry 的 newtype 还明确保持直接表示关系。[Rust type alias](https://doc.rust-lang.org/reference/items/type-aliases.html)、[C++ alias](https://eel.is/c++draft/temp.alias)

C 的 typedef 同样不是独立包装类型；Zig 的类型计算则是理解“执行得到类型”的近邻。两者都不能用来推导 Flurry 保留范式的匹配规律；相关基础对照见[总览](index.md#与-rust--zig--c--c-的关键差异)。

## 运行时实现设计引导

名义身份用于编译时区分接口，不要求每个值携带一个类型编号。newtype 的直接表示契约允许表示复用；其调用入口和资源操作仍须独立确定。

跨接口传递一个 newtype 值时，检查的顺序是目标接口要求、允许的转换、目标交付表示。不要先擦除身份，再从比特宽度倒推某个接口可用。

## 编译器实现引导

建议把语义身份、头形状来源、表示描述分开保存。身份等式可以交给共享等式层；头形状必须保留其适用对象和来源，不应沿所有语义等式无条件传播。

比较算法：

1. 消解 alias；目标为范式应用时保留该项，目标为普通纯调用时按执行与有效缓存取得结果。
2. 对名义项先比较声明身份，再比较适用的参数。
3. 对具有正式构造规律的类型项应用其等式；只对明确自由且单射的构造子反推参数。
4. 对剩余项请求 PTC 证明，或对具有执行语义的待求值项在阶段允许时继续计算；不展开范式头的函数体来建立类型等式。
5. 无法证明相等时保留未决；只有不等证据才能断言不等。

哈希是索引工具，不是等式证明。含符号参数的项不能因为当前哈希键不同而永久判为不等。递归比较需要成对访问记录；访问过一个项只防止重复工作，不能凭此证明无条件相等。
