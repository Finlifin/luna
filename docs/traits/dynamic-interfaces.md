# 动态接口与环境

## Intro

静态分派在编译时确定调用目标，但某些场景——如异构集合、插件系统或运行时配置——要求在不知道具体类型的情况下调用接口方法。动态接口让调用者只保存可用接口，而把具体类型隐藏起来。调用者仍需要知道怎样传入参数、接收结果，以及谁负责保存和清理对象。

隐藏类型不会消除它所依赖的存储或 allocator。因此，这些依赖要和调用入口一起交付。

## 语法

~~~text
trait_object_type -> dyn expr
dyn_cast -> expr.dyn(expr)
closure_type -> %closure_context expr
closure_context -> expr | { environment_binding* }
environment_binding -> id : expr (= expr)?
~~~

`expr` 必须交付合法的完整 trait 实例。动态封装沿用对象、指针及环境原有的资源规则；dyn 本身不意味着值已经转移，也不意味着指向对象自动获得更长生命。

## 正式定义

### 可封装条件

动态接口交付隐藏主体、所选实现证据、可调用入口及必要环境。接口允许的每次调用，参数和结果都必须有可执行的表示和取得方式。

Comptime-only 操作不直接成为 runtime 分派入口。它可以在封装前确定关联值、代码或布局；调用者不能通过 runtime vtable 执行一个任意 comptime 查询。

接口所需的关联项必须被固定、通过已有依赖关系交付，或随隐藏主体一同保持封闭。未知关联项不能被替换成 Any 来掩盖调用表示缺失。

含子句参数的成员，需要在允许的使用边界兑现所需实例和调用约定。不能只登记一个普通函数地址，就承诺支持任意未来类型实参。缺少可交付入口的操作不可在该动态接口上调用。

### 表示与资源

裸 dyn Trait 的大小固定为两个 usize。数据入口和分派入口是一种实现方式；这一大小保证不规定公开字段顺序或固定的 vtable 内容。

所选实现需要额外环境时，构造推导出 `%c dyn Trait`，环境为空才消去限定。完整限定值的承载成本由 c 及其交付方式决定，不承诺整体也只有两个 usize。需要的环境可以合法共享在外层，但不能用隐藏存储擦掉类型中实际需要的环境。

例如 `%{space: Space = space} dyn Drop` 同时描述固定表示的 dyn 主体和名为 space 的环境绑定。它仍需真实取得该环境；赋给裸 dyn 目标不能悄悄丢弃非空 c。

封装已有指针时，动态视图保留对目标的依赖，不接管外部对象的清理责任。按值封装则按主体实际的移动、复制和清理契约交付。不能用同一种“胖指针复制”解释所有 dyn 值。

跨边界后，所有可调用方法需要的环境仍须有效。失效的原始环境不能靠改选一个同类型依赖补齐。

### Self 结果与参数

返回 Self 的方法在隐藏主体内部产生同一具体类型的结果。通过动态接口调用时，把结果连同其所需实现证据和环境重新封闭到相应动态接口；不能把隐藏具体类型当作已知裸返回类型交给调用者。

结果的依赖遵循方法契约，不无条件复制原环境。例如 clone 若生成了新的依赖对象，应交付新对象对应的关系；若复用 allocator，则保留该 identity。结果存储分配也必须有合法调用约定。

需要 Self 参数的方法要求输入确实属于同一隐藏具体类型及必要依赖关系。两个值都写成 dyn Trait，并不能证明它们的 Self 相同。没有可用证据时该次调用拒绝，不以接口名相同执行未经检查的转换。

### 实现稳定性

动态调用使用封装时交付的实现。后续作用域中的导入、同名成员或更方便的适配路径，不会使运行时重新选择对象的实现来源。若编译器证明具体入口唯一，则可以消除间接调用；但语言不承诺每次动态调用必有间接开销。

## 常见示例

两个本地对象提供同一接口，动态视图只在对象存活期间使用：

~~~flurry
trait Reading {
    fn read(*self) -> u32;
}

struct Gauge {
    value: u32,
}

struct FixedReading {}

impl Reading for Gauge {
    fn read(*self) -> u32 {
        self.value.*
    }
}

impl Reading for FixedReading {
    fn read(*self) -> u32 {
        100
    }
}

fn show_dynamic(reading: dyn Reading) {
    println("{}", reading.read());
}

test {
    let gauge = Gauge { value: 27 }
    let fixed = FixedReading {}
    show_dynamic(gauge.ref.dyn(Reading));
    show_dynamic(fixed.ref.dyn(Reading));
}
~~~

输出 `27`、`100`。指针封装沿接收者适配取得主体的接口，不要求另行编写 Reading for *Gauge。show_dynamic 结束时，外面的 gauge 和 fixed 仍由原作用域管理。

错误的逃逸不能被 dyn 隐藏：

~~~flurry
-- 接续 Gauge、Reading 的声明；拒绝例。
fn dangling_reading() -> dyn Reading {
    let local = Gauge { value: 5 }
    local.ref.dyn(Reading)
}
~~~

返回结果仍依赖 local，函数退出后该对象不再有效。只有按相应资源规则真正交付对象或合法外层环境，才能让结果跨过边界。

Self 结果可以用一个无外部资源的复制接口观察：

~~~flurry
trait Duplicate {
    fn duplicate(*self) -> Self;
    fn code(*self) -> u32;
}

struct CodeValue {
    value: u32,
}

impl Duplicate for CodeValue {
    fn duplicate(*self) -> Self {
        Self { value: self.value.* }
    }

    fn code(*self) -> u32 {
        self.value.*
    }
}

test {
    let original = CodeValue { value: 9 }
    let source = original.ref.dyn(Duplicate);
    let copied = source.duplicate();
    println("{} {}", source.code(), copied.code());
}
~~~

输出 `9 9`。source 指向原对象；copied 封闭新构造的 CodeValue，并承担新结果的资源责任。Duplicate 只为展示 Self 返回而定义，不替代 Clone 的具体资源契约。

环境参数化的转交函数保留完整动态接口：

~~~flurry
fn relay_reading(reading: %c dyn Reading) -> %c dyn Reading
where c: Type.vars.Closure
{
    reading
}
~~~

c 为空时结果就是裸 dyn Reading；非空时环境随完整值交付。函数不能把返回类型无条件改为裸 dyn Reading。两个 usize 是裸主体的大小，不是任意环境的总承载大小。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 对 dyn-compatible trait 的成员有明确限制，普通 Self 返回方法不能直接动态分派；这里通过合法的结果再封闭处理这类返回，条件是表示、入口和环境可交付。[Rust dyn compatibility](https://doc.rust-lang.org/reference/items/traits.html#dyn-compatibility)

C 的数据指针加函数表、Zig 的显式接口对象是实际替代；C++ 的虚函数提供动态分派，但对象模型和返回约定不等于本篇的隐藏主体与环境契约。[C11 §6.7.6.3](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf)、[Zig](https://ziglang.org/documentation/master/)、[C++ virtual](https://eel.is/c++draft/class.virtual)

## 运行时实现设计引导

封装描述至少需支持：取得主体、调用公开入口、取得必要环境、执行相应清理。裸 dyn 的表示必须满足两个 usize 的大小保证；具体内部排列和表内容由实现选择。额外环境由完整闭包限定及实际交付承载，不挤入一份未声明的可变长裸 dyn 表示。

Self 结果入口必须支持结果存储与再次封闭。可以由调用约定传入结果承载空间，或由已允许的分配机制建立承载物；不能在没有分配许可时暗中调用全局 allocator。

只保存指针的动态视图不能析构被指向的外部对象。拥有主体的封装则必须在实际退出路径上兑现其清理责任。

## 编译器实现引导

1. 选择完整接口实例和主体实现，生成依赖替换。
2. 对动态可用的每个成员检查参数、结果、关联项和环境可表示性。
3. 将 Self 输出变为带相应证据的封闭结果；检查 Self 输入的身份要求。
4. 检查主体交付方式及资源责任；若需额外环境，推导 `%c dyn Trait` 并交付它，拒绝悬空或目标擦除环境。
5. 生成入口描述和承载方案，保留方法效果与结果依赖摘要。
6. 调用点使用同一描述检查实参与结果；确定具体入口时允许去虚化。

验收至少覆盖：无环境方法、共享外层依赖、按指针封装逃逸失败、Self 结果再封闭、不同隐藏 Self 参数不可混用、comptime-only 成员不能作 runtime 入口。查表成功不代替这些检查。
