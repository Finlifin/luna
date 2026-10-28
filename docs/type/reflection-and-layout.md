# 类型反射与布局

## Intro

反射让编译时程序观察类型。知道一个结构有哪些字段，与知道每个字段在内存中的偏移，是不同层次的信息。

反射因此不要求把所有查询一起算完。一个结构观察可以已经合法，而布局仍在等待参数或目标配置。

## 语法

~~~text
application -> expr(argument*)
take_view -> expr'id
configuration -> .id expr
~~~

类型查询使用普通调用，例如 Type.size_of(T)、Type.repr_of(T)；值的类型使用 value'type。属性语法及其归属见[属性系统](../comptime/attribute.md)。以下内部算法名称不是反射库 API。

## 正式定义

### 观察层次

| 观察 | 所需信息 | 不能顺带推出 |
|---|---|---|
| 类型分类 | 相应分类描述 | 具体元素或字段 |
| 字段、变体、签名 | 合法且足够的结构信息 | 字段可见性、实际访问许可 |
| 大小、对齐、偏移 | 相应合法表示、参数与目标信息 | 行为 trait、任意位串合法 |
| 参数化定义的信息 | 定义对象允许的观察 | 所有已应用类型查询均可使用 |
| 已应用类型的信息 | 该类型允许的结构及表示 | 普通工厂的全部原始输入 |

尚未应用的参数化定义不自动成为已应用类型值。可观察定义信息，不等于能查询某个具体实例的布局。

### 稳定性与构造

交付后的类型字段、变体及影响表示的属性稳定。结构变化形成另一个结果，不原地改变已生成程序所依赖的类型。

类型构造的 build 是事务性的：成功交付完整结果，失败不交付可依赖的半成品。这条规则不承诺任意副作用回滚，也不将任意实现注册变成全局事务。

### 大小与递归

指针位宽与 usize 相同；取得指针本体布局不需要先取得 pointee 的完整布局。直接或间接按值包含自身、导致无法形成有限合法表示的布局环应拒绝。

is_not_sized 描述值相关大小，不描述编译器此刻是否已经算出大小。大小固定但参数未知、大小依赖值、表示不合法是不同情况。

## 常见示例

类型查询可以直接成为编译时常量；运行时仅使用查询结果：

~~~flurry
struct SamplePair {
    first: u32,
    second: u32,
}

const PAIR_SIZE = Type.size_of(SamplePair);
const POINTER_SIZE = Type.size_of(*SamplePair);
const WORD_SIZE = Type.size_of(usize);

comptime {
    assumes POINTER_SIZE == WORD_SIZE;
}

test {
    let pair = SamplePair { first: 12, second: 18 }
    println("values: {}, {}", pair.first, pair.second);
    println("pair size: {}", PAIR_SIZE);
    println("pointer size: {}", POINTER_SIZE);
}
~~~

数值输出为 `12`、`18`；大小输出按编译目标的布局确定。指针大小等于 `usize` 大小的检查不要求取得某个运行时指针的内容，也不从字段声明顺序推导偏移。

程序员也可以封装一个只观察输入类型的函数。quote 参数保留输入描述，而非在编译时读取其运行时内容：

~~~flurry
pure comptime fn storage_bytes(quote value: Any) -> usize {
    Type.size_of(value'type)
}

fn describe_storage(value: u32) {
    const VALUE_SIZE = storage_bytes(value);
    println("value: {}, size: {}", value, VALUE_SIZE);
}

test {
    describe_storage(27);
}
~~~

`VALUE_SIZE` 只依赖 `value` 的类型和目标，数值 `27` 则在运行时用于输出。

递归类型还能区分结构依赖与布局依赖：

~~~flurry
struct Link {
    value: u32,
    next: ?*Link,
}

const LINK_POINTER_SIZE = Type.size_of(*Link);

comptime {
    assumes LINK_POINTER_SIZE == Type.size_of(usize);
}
~~~

指针的布局不沿 `next` 递归计算整个链表。相反，下面的按值环无法形成有限表示：

~~~flurry
struct InvalidLink {
    next: InvalidLink,
}

const INVALID_SIZE = Type.size_of(InvalidLink);
~~~

这是有意展示的非法布局，不是“编译器尚未算出大小”，也不是值相关大小。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 的布局规则区分默认表示和显式表示，C++ 有 sizeof 与类型性质，C 有布局观察，Zig 有类型反射。Flurry 同样要求查询自己的表示契约；不能由 C 背景推定普通 struct 按声明顺序布局。[Rust 布局](https://doc.rust-lang.org/reference/type-layout.html)、[C++ 类型性质](https://eel.is/c++draft/meta.unary.prop)

## 运行时实现设计引导

布局结果至少包括大小/对齐取得方式、字段位置、合法编码条件及必要环境。固定大小项可成为常量；值相关布局需要合法运行时输入，不能偷读编译时不可见的内存。

类型构造失败不能让后续代码获得部分字段布局。布局优化完成后应重新验证对象访问与调用约定所依赖的不变量。

## 编译器实现引导

建立结构查询图和表示查询图，允许共享身份但分别记录依赖。

1. 分类或结构查询只请求必要描述。
2. 每个部分观察先验证前提。
3. 表示查询构造按值依赖边，指针边不展开 pointee 布局。
4. 检查非法环，保留合法符号大小。
5. 类型 builder 在私有构造状态验证结构及属性。
6. 完整成功后发布不可变结果，再允许其他查询依赖它。

图环检查 O(V+E)；具体类型执行与算术另计。发布前的引用必须区分构造句柄与可观察类型结果，不能用缓存命中绕过事务边界。

## 当前源码结构体反射入口

`meta.struct_field_types(T)` 返回 `meta.List<Type>`，
`meta.struct_field_names(T)` 返回 `meta.List<Symbol>`。二者都是普通
comptime 函数，输入必须是已应用、具有合法结构信息的结构体类型。
列表按字段声明顺序排列，空结构体得到空列表；泛型字段使用该次类型
应用的实参替换。字段类型保留其 normal-form 头，不展开普通类型工厂。

```flurry
struct Pair where T {
    first: T,
    ready: bool,
}

comptime fn FirstFieldType() -> Type {
    meta.struct_field_types(Pair<u32>)[0]
}

typealias First = FirstFieldType();

fn example() -> First {
    42
}

comptime fn check_names() -> bool {
    let names = meta.struct_field_names(Pair<u32>);
    names.length() == 2 and names[0] == .first and names[1] == .ready
}
```

参数表达式按普通调用规则求值一次。非法类型、未知结构、未满足的形成
条件和预算耗尽都是 invocation 失败，不用空列表掩盖；只有合法的空结构体
返回空列表。查询不计算布局，也不授予字段访问、Copy 或可见性权限。
反射函数不能在 runtime 执行；在 comptime 生成 quoted code 也不使该
代码中的反射操作自动合法。

这两个入口提供源码可用的结构观察。定义 builder 的事务发布、活跃
invocation 在新声明世界中继续执行，以及更完整的定义描述接口仍是独立
实现工作，不能把字段列表本身视为已生成的新类型。
