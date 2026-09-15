# struct

## Intro

struct 把几个有名字的值组成一个值。字段名称决定构造输入对应到哪里，声明顺序决定初始化先后；这两件事都不直接规定物理布局。

独立的 struct 声明具有独立类型身份，即使它们声明了相同字段。

## 语法

~~~text
struct_def -> struct id clauses? { (configuration | struct_field | definition | statement)* }
struct_field -> id: expr (= expr)?
struct_construction -> expr { property* }
property -> id: expr
~~~

实际字段和构造项用逗号分隔。以右大括号结束的定义或语句不再补分号。属性与子句沿用全局语法。

## 正式定义

### 构造完整性

所有构造项完全指名；不使用位置项或字段名称简写。未知字段与重复指定同一字段都是静态错误。

每个字段必须有唯一初始化来源：显式输入优先，否则使用声明默认表达式；两者都没有则拒绝，不自动填零或调用默认 trait。

按字段声明顺序处理初始化。每个字段先选择实际来源，再求值并按字段类型检查/适配。显式输入覆盖默认值时，默认表达式不执行。

默认表达式在每次实际需要的构造中求值一次，不因定义中只有一份文本就跨实例共享结果。纯编译时缓存仍遵循已有执行规则。

### 表示与使用

普通 struct 不由初始化顺序承诺物理字段顺序。表示由类型、目标与明确配置决定。字段结构信息与布局信息可分别查询。

构造过程中的资源清理与部分对象访问受初始化和资源契约约束；字段默认语法本身不授予提前使用 self 的权利。

## 常见示例

下面用一个会打印字段名的普通函数，让初始化顺序可以直接观察：

~~~flurry
fn supplied_width() -> u32 {
    println("width");
    640
}

fn default_height() -> u32 {
    println("default height");
    480
}

fn supplied_height() -> u32 {
    println("explicit height");
    720
}

struct Window {
    width: u32,
    height: u32 = default_height(),

    fn area(self) -> u32 {
        self.width * self.height
    }
}

test {
    let first = Window { width: supplied_width() }
    let second = Window {
        height: supplied_height(),
        width: supplied_width(),
    }
    let third = Window { width: 320 }

    println("areas: {}, {}, {}", first.area(), second.area(), third.area());
}
~~~

输出依次为：

~~~text
width
default height
width
explicit height
default height
areas: 307200, 460800, 153600
~~~

构造 `second` 时，虽然输入先写 `height`，仍先执行声明在前的 `width`。它的默认高度函数不执行。构造 `third` 又执行一次默认表达式，说明默认值不是在类型定义时只求一次。

以下是分别应被拒绝的构造；它们不是上面可运行示例的一部分：

~~~flurry
let missing = Window {}
let duplicate = Window { width: 1, width: 2 }
let unknown = Window { width: 1, depth: 2 }
~~~

三者分别缺少必填字段、重复提供同一字段、提供不存在的字段。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 允许字段简写，Flurry 要求完整指名。C++ 聚合初始化有自己的顺序及 designated initializer 限制；不能把这些语法限制搬入 Flurry。C 的省略初始化规则也不能作为 Flurry 补零依据；Zig 有声明默认字段值。[Rust struct 表达式](https://doc.rust-lang.org/reference/expressions/struct-expr.html)、[C++ 聚合初始化](https://eel.is/c++draft/dcl.init.aggr)

## 运行时实现设计引导

建立初始化计划，按声明索引而非输入文本顺序执行。每一步记录字段、输入表达式、转换及已初始化状态，供资源契约使用。

物理布局可与初始化计划分离。优化不能重排可观察副作用，也不能执行被覆盖的默认表达式。

## 编译器实现引导

~~~text
plan_constructor(type, supplied):
    index := declared_fields_by_name(type)
    reject_unknown_and_duplicate_names(supplied, index)
    plan := []
    for field in declaration_order(type):
        source := explicit_input(field) or declared_default(field)
        if source absent: reject_missing_field(field)
        plan.append(check_and_adapt(source, field.type))
    return plan
~~~

这是内部伪代码，or 表示选择初始化来源，不是先求值两个候选。

字段索引构建 O(n)，散列表输入对应均摊 O(n+m)，求值和类型检查另计。完整性检查不应因某个输入尚未求值就跳过重复标签诊断。阶段延迟保留具体字段义务，不能交付缺字段实例。
