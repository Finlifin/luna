# 处理能力与接口接入

## Intro

处理器可以带有状态，也可以由库实现。调用方选择一个具体对象，语法协议把它提供的处理入口接到计算需要的位置。

处理能力描述对象能够回答哪些请求；安装还要确定这一次计算使用哪个对象、哪层入口以及怎样退出。

## 语法

~~~text
handler_bind -> pattern handles expr
handler_application -> expr.use(expr)
effect_handle -> expr# { case_arm* }
error_handle -> expr! { error_arm* }
~~~

多个处理能力在 handles 后用 and 连接。该模式可嵌入元组等已有绑定模式，函数签名中的效应需求继续使用结果的效应限定。

## 正式定义

### Trait 与 elaboration 的分工

Trait 协议声明处理入口的要求，实现提供行为；elaboration 在安装处取得选定实例、接收者及环境，把入口接到对应处理位置。

接口选择沿用普通 implementation 的来源、约束与一致性规则。找到一个方法不等于已经消去行：安装仍须检查标签覆盖、结果交付、外层行为、恢复与资源责任。

绑定模式中的能力注解描述可使用的处理入口，不为对象补出它没有实现的能力。一次绑定也不隐式处理整个程序中的同名操作；处理计算仍通过相应安装位置接入。

### 环境与结果

处理器的接收者和捕获按普通传参与资源规则使用。连续调用可在契约允许时修改状态；同时执行的异步分支则另需满足共享或串行访问的约束。

错误入口接收错误载荷并产生处理结果。直接效应入口接收操作实参并产生操作结果 Y。可恢复入口还接收 continuation，并遵守整体结果 B 的恢复契约。不能用一个未经区分的函数指针替代这三类入口。

### 路由的完整性

effect enum 的组只是接口组织方式。单项或整组接入均须保留每个完整操作实例及所选 occurrence。分支向外请求同名操作时使用外层环境，不再次进入自己。

部分处理只消去已经覆盖的路径；剩余请求必须有外部路由。错误入口也只消去对应错误，不会因为对象同时有其他方法而抹去其他行。

## 常见示例

### 将一个有状态接口接入 effect

下面的 ProvideNumber 是例子自定义的普通 trait，具体分支显式连接它，不把该名称当作内置效应协议。

~~~flurry
trait ProvideNumber {
    fn provide(*self) -> u32;
}

struct Counter { value: u32, }

impl ProvideNumber for Counter {
    fn provide(*self) -> u32 {
        let result = self.value.*;
        self.value.* += 1u32;
        result
    }
}

effect next_number() -> u32;

fn total() -> #[next_number] u32 {
    let first = next_number()#;
    let second = next_number()#;
    first + second
}

test {
    var counter = Counter { value: 10u32 }
    let result = total()# {
        next_number() => counter.provide(),
    }
    println("{} {}", result, counter.value);
}
~~~

输出 21 12。trait 的方法访问同一个 Counter，handler 只负责把操作请求接到这个方法。

### 处理能力绑定与 .use

下面把错误入口的接口完整写出：

~~~flurry
trait HandleError
where E, R
{
    fn handle_error(*self, value: E) -> R;
}

enum LimitErr { TooLarge(u32), }
struct ClampTo { limit: u32, }

impl HandleError<LimitErr, u32> for ClampTo {
    fn handle_error(*self, value: LimitErr) -> u32 {
        value match { .TooLarge(_) => self.limit.*, }
    }
}

fn checked_count(value: u32) -> !LimitErr u32 {
    if value <= 8u32 { value } else { error LimitErr.TooLarge(value) }
}

test {
    let handler handles LimitErr = ClampTo { limit: 8u32 }
    let result = checked_count(12u32).use(handler);
    println("{}", result);
}
~~~

输出 8。正常成功结果不调用 handle_error；错误结果则经选定实例处理。错误的表示仍是错误限定结果，不转成一次可恢复 effect 请求。

### 保留外层供应

接续 next_number 和 total：

~~~flurry
fn total_with_offset() -> #[next_number] u32 {
    total()# {
        next_number() => {
            let outside = next_number()#;
            outside + 1u32
        },
    }
}

test {
    let result = total_with_offset()# { next_number() => 5u32, }
    println("{}", result);
}
~~~

输出 12。函数仍要求外层 next_number；内层分支只提供变换，不能把自身使用的外层需求一并消去。

## 与 Rust / Zig / C / C++ 的关键差异

Rust trait、C++ 虚函数或模板、Zig 的编译时接口和 C 函数表均可承载处理对象。Flurry 的额外契约在安装处：它把这些入口与错误或效应行的消去、occurrence 以及恢复关系连接起来。普通动态分派本身不定义这些控制行为。[Rust trait](https://doc.rust-lang.org/reference/items/traits.html)

## 运行时实现设计引导

安装结果保存选定入口及其实际环境。直接效应按操作传参；错误入口只在错误分支调用；捕获入口按 continuation 协议交接恢复权。

环境只创建和求值一次。若多个入口引用同一个对象，应保留共同对象身份，不因为有多个操作就复制对象。退出时按剩余责任清理，不能对每个入口重复销毁共同环境。

## 编译器实现引导

1. 从绑定注解与安装表达式取得需要的处理能力。
2. 按普通 trait resolution 选择实现，取得关联项、环境与替换。
3. 分别建立错误映射、直接操作入口、可恢复入口或安装值。
4. 检查完整行关系和未覆盖路径，保留同名 occurrence。
5. 检查实际对象的可用性、共享、移动与退出清理。
6. 将处理分支自己的错误和效应接到其外层环境。
7. 向 ABI 层交付明确的入口类别，不以通用方法调用掩盖控制契约。

在增量编译中，入口缓存必须依赖所选实现和环境接口；只有同名操作和结果类型不足以复用旧入口。
