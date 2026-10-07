# 移动与初始化

## Intro

移动把当前值交到新位置。源位置不再持有它，但该值拥有的外部分配可以继续存在。

移动一个字段后，其余字段仍可用。判断依据是还剩哪些已初始化值，以及接下来的操作和清理究竟要访问什么。

## 语法

移动没有额外的调用关键字；编译器在以下按值位置中，依据 Copy 能力选择复制或移动。

~~~text
value_binding -> let pattern (: expr)? = expr
    | var pattern (: expr)? = expr
field -> expr.id
return_statement -> return expr? (while expr)?
application -> expr(argument*)
~~~

## 正式定义

### 源状态与目标状态

移动要求源持有满足类型契约的值。成功交付后，目标取得值、实际依赖及可转移责任，而源的相应位置变成已移出。

已移出不是"值变成零"，也不意味着底层所有字节都被释放。即使原位置仍保留相同位模式，程序也不能继续把它作为仍持有该值的位置使用。

完整初始化、部分初始化与已移出须分别处理。读取整个聚合要求该操作需要的所有部分都可用；而访问一个仍已初始化、且与已移出部分分离的字段，可以按其自身契约继续。

### 部分移出与恢复

部分移出转移被选字段的责任，同时保留剩余字段。后续清理只处理仍持有的字段；若用户 Drop 需要已经移出的内容，则必须在清理前合法恢复，或提供能够处理该状态的已核验契约。

存在用户 Drop 本身不构成一律禁止部分移出的规则。决定接受性的，是实际 Drop 和剩余清理的前提。

向已移出位置安装新值可以恢复该位置，但不能通过重置一个"已初始化"标记来代替构造。覆盖仍持有的值时，须先兑现其旧责任。

### 移动不修复指针

指向独立分配的视图可以在 owner 描述符移动后继续有效，前提是分配及其必要性质保持。然而，指向原对象内联字段的指针仍指向原字段位置，不自动追随字节移动。

返回对象前，还要考虑函数退出时的剩余清理。返回表达式求值当时有效，不足以保证返回值交付后仍有效。

## 常见示例

### 移动 owner，保留字符分配

~~~flurry
struct TextPacket {
    storage: String,
    view: str,
}

fn make_packet() -> TextPacket {
    let storage = "hello".to_string();
    let view = storage.as_str();
    TextPacket { storage: storage, view: view }
}

fn pass_packet(packet: TextPacket) -> TextPacket {
    packet
}

test {
    let packet = pass_packet(make_packet());
    println("{}", packet.view);
}
~~~

此处 view 指向 String 的字符分配，因此两个描述符的移动不搬走字符。聚合退出时先结束 view 的平凡清理，再清理 storage。

### 返回 owner 与返回 view 的区别

~~~flurry
fn take_storage(packet: TextPacket) -> String {
    packet.storage
}

-- 拒绝：剩余 storage 的清理会使结果失效。
fn take_view(packet: TextPacket) -> str {
    packet.view
}
~~~

take_storage 转出字符资源责任；残余 view 的清理不读取字符。而 take_view 留下 owner，函数退出会结束结果所需的字符分配。

### 已移出字段可以重新初始化

~~~flurry
struct TextPair {
    first: String,
    second: String,
}

fn replace_first(pair: TextPair) -> TextPair {
    let previous = pair.first;
    println("{}", previous.as_str());
    pair.first = "replacement".to_string();
    pair
}
~~~

安装 first 时该字段已空，因此不再次清理 previous 的责任。previous 仍由自己的局部绑定清理；返回 pair 取得重新完成的聚合。

### 内联地址不能随返回自动迁移

~~~flurry
struct InlineLink {
    value: i32,
    pointer: ?*i32,
}

-- 拒绝：pointer 仍指向 local 的内联字段。
fn escape_inline() -> InlineLink {
    let local = InlineLink { value: 7, pointer: null }
    local.pointer = local.value.ref;
    local
}
~~~

若类型需要地址稳定，必须由构造、存储和迁移契约提供对应保证。普通返回优化不作为源程序合法性的依据。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 也跟踪部分移动后的字段状态，但对实现 Drop 的类型有额外移动限制；而 Flurry 以实际清理访问条件决定是否可完成交付。[Rust 部分移动](https://doc.rust-lang.org/rust-by-example/scope/move/partial_move.html)

C 的结构复制不会自动修复自引用指针。C++ 可通过自定义移动构造修复特定关系，但这不是普通指针复制的效果。Zig 同样需要在库契约中处理地址稳定性；Flurry 将这些关系纳入访问和出口检查。

## 运行时实现设计引导

聚合可以按字段保存初始化标记；当路径已静态确定时，运行时不需要这些标记。动态分支上的标记用于决定实际需要清理哪些字段，不替代对象有效性证明。

若实现采用返回位置直接构造或移动消除，必须先保证语义上的交付合法。实现不得靠一次碰巧没有复制字节的代码生成策略，使原本依赖失效局部地址的结果通过检查。

## 编译器实现引导

编译器维护位置树，每个节点至少记录已初始化、已移出或路径相关状态。移动字段时更新该字段，避免粗暴地把整个聚合的其他字段都标为不可用。

1. 检查被移动子位置和目标的交付要求。
2. 迁移该位置的依赖与责任。
3. 标记源为空，更新聚合完整性。
4. 检查剩余操作及清理是否需要该子位置。
5. 在重新初始化后恢复相应字段状态。
6. 在出口先转交结果、执行剩余清理，再检查结果依赖。

强更新仅适用于已证明对应单个具体实例的完整目标；别名不确定或循环汇总的位置不能借一次写入清空全部旧依赖。
