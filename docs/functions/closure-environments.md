# 闭包环境、调用与清理

## Intro

环境保存闭包以后还需要的值。它可以含普通数值，也可以含承担清理责任的对象或依赖外部存储的指针。

调用可能只读取环境，也可能改变它、取走其中的值。因此，判断能否再调用，要看调用后的实际状态。

## 语法

~~~text
closure_type -> %closure_context expr
closure_context -> expr | { environment_binding* }
environment_binding -> id : expr (= expr)?
lambda -> |anchored_parameter*| (-> expr)? expr
application -> expr(argument*)
~~~

环境绑定可以写成 `%{name: T = value} F`。它描述 F 所需的绑定及其来源；环境的类型关系见[闭包限定](../type/qualified-types/closure-type.md)。

## 正式定义

### 环境与主体

%C F 保留主体 F 与上下文 C。空环境限定消去；非空环境不能仅因当前优化恰能删除存储，就被当作源语言中的无环境值。

上下文按依赖有效的声明次序比较。局部绑定可一致改名，外部对象 identity 不能跟着任意替换。环境类型等价不等于环境值相同。

闭包环境和 CMTT 的代码上下文各自约束自己的对象。运行时闭包携带可用值与责任；quoted code 的上下文描述生成代码需要的绑定，不能把后者当作提前存在的 runtime 存储。

### 调用后的状态

Once 限制一次责任至多消耗一次，不禁止消耗前重复读取。闭包是否还能调用，取决于主体操作和已经交付的契约能否在当前环境状态下再次成立。

若主体把非 Copy 捕获按值交出，该位置在调用后已移出。若只是通过有效指针观察对象，相应责任不因这次读取而消失；对象仍须在下一次访问时有效。

构造、调用和销毁分别具有内存效果与纯度。创建一个只保存指针的环境可能很简单，调用却可以修改指向对象；不能由构造行为推断整个回调只读。

| 主体行为 | 一次调用后的环境 | 后续调用的依据 |
|---|---|---|
| 读取 Copy 捕获 | 原捕获仍可用 | 相同入口条件仍成立 |
| 通过指针修改外部对象 | 指针捕获仍在，对象状态改变 | 按新状态重新检查访问与操作前提 |
| 交出非 Copy 捕获 | 相应字段已移出 | 不能再次要求同一份责任 |
| 不调用即离开作用域 | 环境保持构造后的责任 | 直接清理，不执行主体 |

Once、Clone、Copy 是值的资源能力；不能把 Once 解释为“这个闭包至多可读取一次”。也不能因为每个捕获的类型有 Clone 就在重复调用前隐式克隆环境。重复执行与复制一个可调用值是两个不同操作。

### 清理

捕获去重后按首次出现的词法次序形成语义字段，逆序清理尚留在环境中的责任。已经移出的字段不再清理。闭包未被调用，也按相同规则处理环境。

物理布局、环境共享及去虚化都不改变可观察清理次序。捕获之间的真实依赖必须保持有效；不能先结束被其他尚活捕获需要的对象。

### 逃逸

闭包返回、进入更长寿命的存储或被交给会保留它的接口时，需要交付整个环境及其依赖。按值移入自己的数值可以随闭包离开；捕获指向局部值的指针不使该局部值延寿。

返回了闭包，并不证明其捕获中的所有指针有效。访问、世代、内容版本和终结责任按[可达依赖](../resource-management/reachability.md)及契约检查。

## 常见示例

### 只读调用保留环境

~~~flurry
test {
    let offset: u32 = 4;
    let add_offset = |value: u32| value + offset;
    println("{}", add_offset(1));
    println("{}", add_offset(2));
}
~~~

输出 `5`、`6`。读取 Copy 捕获没有消耗掉环境。

### 回调通过指针修改状态并再次调用

~~~flurry
test {
    let quota: u32 = 2;
    let remaining = quota.ref;
    let take_one = || {
        if remaining.* == 0 {
            false
        } else {
            remaining.* -= 1;
            true
        }
    }
    println("{}", take_one());
    println("{}", take_one());
    println("{}", take_one());
    println("remaining {}", quota);
}
~~~

输出 `true`、`true`、`false`、`remaining 0`。闭包没有交出 remaining；每次都使用同一个仍有效的对象。条件保护了减法，前一次写入更新后一次读取的内容版本。允许再次调用不意味着可以合并三次结果。

### 消耗捕获不能再交付一次

~~~flurry
struct Ticket {
    number: u32,
}

fn finish(ticket: Ticket) -> u32 {
    ticket.number
}

test {
    let ticket = Ticket { number: 7 }
    let submit = || finish(ticket);
    println("{}", submit());
    -- 此后再次 submit() 会重复交付已移出的 ticket。
}
~~~

submit 的第一次调用交付 7，第二次调用缺少它原先需要的责任。环境状态必须参与检查，不能仅看两次调用的参数表都为空。

以下接续 Ticket 和 finish，是独立的拒绝例：

~~~flurry
fn repeated_consumption() {
    let ticket = Ticket { number: 8 }
    let submit = || finish(ticket);
    println("{}", submit());
    println("{}", submit());
}
~~~

不能用第二次调用的空实参表，推导第一次移出的环境字段已经重新初始化。

### 未调用环境的逆序清理

~~~flurry
struct Trace {
    id: u32,

    fn inspect(*self) -> u32 {
        self.id.*
    }
}

impl Drop for Trace {
    fn drop(*self) {
        println("drop {}", self.id.*);
    }
}

fn leave_closure() {
    let first = Trace { id: 1 }
    let second = Trace { id: 2 }
    let action = || {
        println("{}", second.inspect());
        println("{}", first.inspect());
    }
}

test {
    leave_closure();
}
~~~

主体没有执行。捕获的语义次序为 second、first，因此退出时输出 `drop 1`、`drop 2`。原来的局部位置已交出责任，不再重复清理。

### 调用消费一个捕获，环境清理另一个

接续 Trace 的定义：

~~~flurry
fn consume(resource: Trace) -> u32 {
    resource.id
}

fn run_once() {
    let first = Trace { id: 1 }
    let second = Trace { id: 2 }
    let action = || {
        let value = consume(first);
        println("still {}", second.inspect());
        value
    }
    println("result {}", action());
}

test {
    run_once();
}
~~~

输出 `drop 1`、`still 2`、`result 1`、`drop 2`。first 在 consume 的退出处结束责任；second 仍留在闭包环境中，随环境退出清理。不能把整个环境在调用后标为“全部已销毁”，也不能在环境析构时再次 Drop first。

### 有类型的闭包仍可能逃逸非法

~~~flurry
-- 拒绝：action 跨出了 local 的存活范围。
test {
    let action = do {
        let local: u32 = 7;
        let source = local.ref;
        || source.*
    }
    println("{}", action());
}
~~~

环境类型可以正确记录 source，仍无法让 local 在块退出后继续存活。问题是目标存储已经结束，不是函数参数表缺少信息。

相同场景可以显式保存内容副本：

~~~flurry
test {
    let action = do {
        let local: u32 = 7;
        || local
    }
    println("{}", action());
}
~~~

输出 `7`。环境拥有 u32 的副本，未保存指向 local 槽位的地址。这是两种不同的依赖关系；不是对原来悬空指针例子的优化放行。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 的闭包调用接口区分 Fn、FnMut、FnOnce；Flurry 使用主体、环境和资源契约描述可调用状态，不引入这组接口作为默认模型。Rust 闭包捕获的析构行为也不能替代这里明确的首次捕获与逆序清理规则。[Rust closure traits and drop order](https://doc.rust-lang.org/reference/types/closure.html)

C++ lambda 的捕获存储属于闭包对象；C 的手写上下文与 Zig 的状态结构体也需要管理所引用对象的有效期。Flurry 将这些环境依赖纳入语言的资源检查。[C++ lambda](https://eel.is/c++draft/expr.prim.lambda)、[Zig structs](https://ziglang.org/documentation/master/#struct)

## 运行时实现设计引导

环境实现至少保持字段来源、当前初始化状态与清理所需依赖。状态可以完全静态消去；只有控制流确实需要时才留下运行时标志，不要求所有闭包带一组动态标签。

对调用出口应用主体契约的状态变换，再生成剩余环境的清理。析构只针对实际存活字段；通过指针捕获的外部对象由原来的责任管理，除非接口明确转移了责任。

可以将单个环境字段的状态表示为：未取得、持有有效值、已移出、已终结。环境构造只把实际完成取得的字段推进到持有；调用可能读取、修改或移出；环境清理逆序处理仍持有的责任。分支汇合时保存路径条件，不能把一条路径上的持有状态覆盖到所有路径。

在捕获中共享外部存储可以减少复制，不能改变终结条件。环境地址是否稳定，依赖真实的存储与移动规则，不能由“这是一个闭包”自动取得保证。

## 编译器实现引导

1. 从捕获分析接收有语义次序的环境字段及依赖图。
2. 为构造建立取得责任后的状态，为调用建立前提和各出口状态，为销毁建立剩余责任序列。
3. 在调用点把环境 identity 与真实实参、返回依赖统一交付给程序验证层。
4. 对分支汇合保留确切可知的字段状态；未知状态不能恢复为“全部有效”。
5. 对逃逸边界验证环境及可达对象有效期；失败指明具体来源和结束位置。
6. 最后选择布局与调用约定，保留先前的清理和依赖证据。

资源安全检查在 NIR 完成；verified 的更强证明与 IIR 接口分别处理。高阶关系不能仅凭闭包类型结构自动证明，复杂要求依赖显式契约及受检 assumption。
