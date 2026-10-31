# 临时值与丢弃

## Intro

调用可以在没有命名 owner 的情况下产生临时值。调用链可以使用它，但把视图保存下来不会自动把临时 owner 一起保存。

跨语句使用一个视图时，先给 owner 建立明确绑定，从而直接表达它需要活到哪里。

## 语法

~~~text
let_decl -> let pattern (: expr)? = expr (else block)?
wildcard_pattern -> _
reference -> expr.ref
application -> expr(argument*)
~~~

match 与 if-is 沿用[模式匹配](../control-flow/pattern-matching/index.md)。

## 正式定义

### 完整表达式

未交付到 owner 绑定或返回值的临时责任，在所属完整表达式结束时清理。整个初始化右侧、整个赋值动作和整个表达式语句都保留其实际求值期间所需的临时值。

一条调用链须完成本次访问，才结束仍由该表达式持有的临时责任。保存引用或视图不会自动延长来源 owner。

普通 bool 条件求值产生的临时值，在进入选定分支正文前结束。作为 match 主体保留的临时值支撑实际匹配及所选臂，直到整个 match 表达式结束；if-is 的匹配主体相应支撑所选分支。

当子表达式因错误或其他控制转移离开时，程序清理已经完成构造且未转交的责任。结果必须在这些清理之后仍然有效。

### 丢弃已有位置与丢弃新值

`_` 不建立按值绑定。用于已有位置时，`_` 不因这个模式额外移出已有值。右侧本来要发生的调用、效果或资源交付仍然执行。

新产生而无人接收的拥有值必须清理；程序在完整表达式结束时兑现仍属于它的责任。因此不能把 `let _ = make_value()` 解释成"无条件删掉构造调用"。

### 模式生成的代码

pattern view 的 elaboration 是编译期代码生成。运行时的访问、临时资源、控制流和清理按实际生成的程序检查。`_` 也不能删除为后项匹配提供依据的生成步骤。

## 常见示例

先定义一个能观察清理时机的类型：

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

fn make_trace(id: u32) -> Trace {
    Trace { id: id }
}
~~~

### 在同一调用链读取

~~~flurry
test {
    let number = make_trace(1).inspect();
    println("number {}", number);
}
~~~

先输出 `drop 1`，再输出 `number 1`。取得的 u32 不依赖临时 Trace 的存储。

### 保存指针不延长 owner

~~~flurry
-- 拒绝：初始化语句结束后临时 Trace 已清理。
fn expired_temporary() {
    let pointer = make_trace(2).ref;
    println("{}", pointer.inspect());
}

fn named_owner() {
    let owner = make_trace(3);
    let pointer = owner.ref;
    println("number {}", pointer.inspect());
}
~~~

named_owner 输出 `number 3`，再输出 `drop 3`。

### 两种通配丢弃

~~~flurry
fn ignore_existing() {
    let value = make_trace(4);
    let _ = value;
    println("number {}", value.inspect());
}

fn ignore_created() {
    let _ = make_trace(5);
    println("after");
}
~~~

前者输出 `number 4`、`drop 4`；后者输出 `drop 5`、`after`。

### match 主体覆盖选定臂

~~~flurry
test {
    make_trace(6) match {
        { id: 6 } => println("number 6"),
        _ => (),
    }
}
~~~

匹配主体保持到臂内读取完成，输出 `number 6` 后清理它。这个范围不允许把指向主体的结果交付到 match 之外继续访问。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 与 C++ 都有特定引用绑定延长临时对象生命的规则；而 Flurry 保存视图不自动延长来源。[Rust 临时生命期延长](https://doc.rust-lang.org/reference/destructors.html#temporary-lifetime-extension)、[C++ 临时对象](https://eel.is/c++draft/class.temporary)

C 中指向已结束自动对象的指针同样不能继续解引用；Zig 可以通过显式局部值及清理安排管理来源。Flurry 将临时责任及其依赖纳入统一的静态检查。

## 运行时实现设计引导

临时值可以位于寄存器、栈或最终目的位置。无论采用何种存储，实现都须保持所属完整表达式和转交点。

一旦临时责任交给绑定、聚合字段或结果，原临时记录不再执行清理。错误路径只处理已经建立的剩余责任，不能清理尚未成功构造的对象。

## 编译器实现引导

归一化时，编译器给临时值保留源码语义范围。一个初始化右侧被拆成多条 IR 指令，不使每条指令都成为新的临时清理边界。

对 `_`，编译器区分右侧的位置观察、调用求值与新 owner 结果；只省略该模式本身不需要的绑定。后续死代码删除仍须检查[纯度](runtime-purity.md)、资源责任和匹配依赖。

对匹配主体，编译器记录实际控制范围，把其必要的保活边传到生成的分支与清理；无需给 elaboration 另加运行时所有权协议。
