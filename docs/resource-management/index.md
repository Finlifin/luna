# 资源管理

## Intro

把一个值交给另一个位置，也把它持有的清理责任交过去。需要继续使用数据时，可以复制值、显式克隆，或通过指针访问仍然存在的对象。

资源管理要保证两件事：责任不会被重复兑现；每次实际访问所依赖的存储仍然有效。一个指针可以复制多次，却不会因此多出几份释放目标的责任。

## 语法

以下入口分别在后续章节定义，`expr`、`pattern` 同全局语法。

~~~text
binding -> let pattern (: expr)? = expr (else block)?
    | var pattern (: expr)? = expr (else block)?
reference -> expr.ref
dereference -> expr.*
reachability_type -> &expr expr
resource_clause -> requires expr
    | ensures expr
    | outcomes expr
~~~

## 正式定义

### 阅读顺序

| 章节 | 核心问题 |
|---|---|
| [Once、Clone、Copy](once-clone-copy.md) | 怎样消耗、克隆或复制一个值？ |
| [绑定与赋值](bindings.md) | let/var 限制什么，覆盖失败时留下什么？ |
| [移动与初始化](moves-and-initialization.md) | 移出一个字段后，聚合还剩下哪些可用部分？ |
| [自动清理](drop-elaboration.md) | 谁清理、何时清理、按什么顺序清理？ |
| [临时值与丢弃](temporaries.md) | 没有名称的 owner 活到哪里？ |
| [指针与访问](pointers.md) | 取址、字段投影和别名怎样使用？ |
| [可达依赖与有效性](reachability.md) | 视图依赖什么，何时失效？ |
| [内存视图](memory-views.md) | 怎样区分描述符、分配、字段和区间？ |
| [内存效果](memory-effects.md) | 操作会读、写、终结和保留什么？ |
| [契约、推导与验证](contracts-and-verification.md) | 哪些条件默认推导，哪些需要显式证明或运行检查？ |
| [运行时纯度](runtime-purity.md) | 哪些计算可以复用、删除、移动或并行执行？ |

### 值、位置与责任

**位置**容纳一个当前值。**值**可以拥有存储，也可以仅保存指针。**清理责任**决定哪些动作必须在值被消耗或离开其清理范围时执行。这三个概念源自仿射类型理论：每个值至多被使用一次，从而在编译期就能跟踪资源的唯一归属，避免重复释放和悬垂引用。

移动 String 的描述符，可以转交字符分配而保留其地址；而取描述符本身的地址得到的指针，则仍指向原来的描述符位置。把这两种地址混同，会同时误判合法移动和非法逃逸。

按值交付非 Copy 值后，源位置不再持有它。已有的合法观察不消耗另一份值；观察是否可执行，由目标的存活、初始化、类型和操作契约决定。

### 责任与访问分别约束程序

一个对象可以承担缓冲区的清理责任，而缓冲区中的指针仍依赖外部对象。因此，拥有容器不会自动拥有其中指针的目标。

反过来，持有一个视图也不会延长来源的生命。程序必须让来源覆盖视图的实际使用，包括隐藏在 Drop 中的读取。

## 常见示例

### 先观察，再交付

~~~flurry
struct Ticket {
    code: u32,

    fn inspect(*self) -> u32 {
        self.code.*
    }
}

fn finish(ticket: Ticket) {
    println("finished {}", ticket.code);
}

test {
    let ticket = Ticket { code: 7 }
    println("{}", ticket.inspect());
    println("{}", ticket.inspect());
    finish(ticket);
}
~~~

两次 inspect 读取同一个仍有效的 Ticket。finish 按值取得它，此后调用方不能再次读取或移出 ticket。

### 中间视图结束，目标仍然存在

~~~flurry
test {
    let owner = "hello".to_string();
    let view = do {
        let intermediate = owner.as_str();
        intermediate
    }
    println("{}", view);
}
~~~

view 指向字符缓冲区，不依赖 intermediate 的局部槽位。owner 在输出后才清理，因而字符仍可读取。

## 与 Rust / Zig / C / C++ 的关键差异

| 语言 | 对应物与区别 |
|---|---|
| C | 指针和显式释放是直接对应物；Flurry 将按值交付、剩余清理和访问前提纳入语言检查 |
| C++ | RAII 是自动清理的对应物；Flurry 的仿射交付还使源位置失去该值的可用状态 |
| Rust | Copy/Clone 和所有权提供相近的使用方式；Flurry 的指针访问依据程序契约，不把所有指针分成共享引用与独占引用 |
| Zig | `defer` 是作用域清理的实际手段；Flurry 从剩余资源责任自动产生清理 |

具体区别见各节；Zig 的作用域清理机制见[语言文档](https://ziglang.org/documentation/master/#defer)。

## 运行时实现设计引导

普通移动可由寄存器传递、内存复制或目的位置构造实现。所选实现必须保持源不可再交付、目标承担剩余责任、指针目标关系不被暗中改写三个不变量。

编译器无需给所有值附加运行时区域表。静态已证明的生命周期、分离性和初始化事实可以消除对应的运行时检查；动态检查只为实际需要维护的条件保留元数据。但是，表示消除不得删除可观察的清理或检查失败。

## 编译器实现引导

资源检查以统一的程序状态贯穿各章：

1. 辨认值所在位置及其已初始化部分。
2. 为交付记录 Copy 或 move，以及结果携带的依赖和责任。
3. 在所有实际退出边加入语义清理动作。
4. 按顺序检查访问、状态变化和清理前提。
5. 在函数边界交付效果上界、结果关系和各出口保证。
6. 用已成立的保证约束优化与降码。

safe 的全部检查位于 normalized IR；verified 检查位于 instantiated IR。类型、布局和实现选择为程序检查提供输入；而逐语句的内存状态、效果与证明义务不交给 PTC。
