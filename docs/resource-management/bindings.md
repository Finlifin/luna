# 绑定与赋值

## Intro

绑定为值提供名称。let 保留直接用这个名称进行整值赋值的限制；var 允许这种赋值。两者都不替代对象的访问和清理契约。

替换一个持有资源的值，需要先得到新值，再清理旧值。若新值求值失败，赋值动作本身不会提前销毁旧值。

## 语法

~~~text
let_decl -> let pattern (: expr)? = expr (else block)?
var_decl -> var pattern (: expr)? = expr (else block)?
const_decl -> const id (: expr)? = expr
assignment -> expr = expr
~~~

模式与 `else` 规则见[模式匹配](../control-flow/pattern-matching/index.md)，const 求值见[编译时计算](../comptime/index.md)。

## 正式定义

### let、var、const

| 声明 | 约束 |
|---|---|
| let | 不允许直接以该绑定名称作整值赋值目标 |
| var | 允许直接以名称作整值赋值目标 |
| const | 建立编译时值，按编译时求值规则解释 |

let 不保证对象内容不变。这一设计有意将"绑定不可重赋值"与"对象内容只读"区分开来：let 限制的是整值替换操作，而字段写入、经指针写入乃至经合法指针替换整个本体，均由对象契约检查。因此不能从 let 单独推出只读存储或并发只读资格。

绑定左侧是模式。普通名称取得按值交付；`_` 的行为见[临时值与丢弃](temporaries.md)。遮蔽建立新绑定，不赋值给旧绑定，也不改变旧绑定自己的清理位置。

同一顺序块中的新绑定在初始化完成后才遮蔽同名旧绑定；因此初始化表达式仍在旧环境中解析。名称层面的完整规则见[作用域与遮蔽](../code-organization/scope-and-paths.md)。

### 覆盖赋值

以下规则适用于持有已知清理责任的普通值。[union 共享存储赋值](../type/user-defined-types/unions.md)遵循其独立契约，不隐式执行旧成员 Drop。

对一个合法的赋值动作，目标定位与右侧表达式遵守普通求值规则，各自只求值一次。资源处理次序如下：

1. 右侧成功交付新值，由当前计算承担其临时责任。
2. 编译器检查已经定位的目标仍可写入。
3. 若目标仍持有旧值，执行其清理；若右侧已合法移走旧值，则不重复清理。
4. 编译器检查最终安装的前提，然后写入新值并转交临时责任。

右侧失败时，清理过程只处理已完成求值且仍由本次计算持有的临时责任。该失败保证不回滚右侧已经执行的修改、输出或外部调用。

若旧值清理不能完成，编译器不能宣布新值已经安装。其后续责任按真实控制流处理；赋值不提供任意析构的事务回滚机制。

### 清理位置

绑定的清理次序由声明位置决定。覆盖赋值结束旧值责任后，新的当前值继续使用该绑定原有的清理位置，不因为刚刚赋值而移到作用域清理序列的另一端。

## 常见示例

### let 不冻结对象

~~~flurry
struct Cell {
    value: u32,
}

test {
    let cell = Cell { value: 1 }
    cell.value = 2;
    let pointer = cell.ref;
    pointer.* = Cell { value: 3 }
    println("{}", cell.value);
    -- 直接写 cell = Cell { value: 4 } 不符合 let 的赋值规则。
}
~~~

输出 `3`。两次合法写入都必须保留 Cell 的对象和字段契约。

### 覆盖不会重排退出清理

~~~flurry
struct Trace {
    id: u32,
}

impl Drop for Trace {
    fn drop(*self) {
        println("drop {}", self.id.*);
    }
}

test {
    var first = Trace { id: 1 }
    let second = Trace { id: 2 }
    first = Trace { id: 3 }
    println("body");
}
~~~

输出依次是 `drop 1`、`body`、`drop 2`、`drop 3`。first 的当前值虽然构造得较晚，但仍按 first 的声明位置清理。

### 构造失败时保留旧值

下面继续使用 Trace。

~~~flurry
enum BuildErr {
    Refused,
}

fn build_trace(id: u32, fail: bool) -> !BuildErr Trace {
    println("build {}", id);
    if fail {
        error BuildErr.Refused
    } else {
        Trace { id: id }
    }
}

fn replace_trace(target: *Trace, fail: bool) -> !BuildErr void {
    target.* = build_trace(2, fail)!;
}

test {
    var value = Trace { id: 1 }
    replace_trace(value.ref, true)! {
        .Refused => println("kept {}", value.id),
    }
}
~~~

输出 `build 2`、`kept 1`，最后 `drop 1`。如果 build_trace 自身另有修改目标的操作，那么失败后状态还必须包含那些真实修改。

### 遮蔽保留两份独立责任

~~~flurry
test {
    let value = Trace { id: 4 }
    do {
        let value = Trace { id: 5 }
        println("inner {}", value.id);
    }
    println("outer {}", value.id);
}
~~~

内层先输出并清理 `5`，外层的 `4` 仍然可用。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 的不可变绑定对直接修改所绑定位置施加限制，不能照搬来解释 Flurry let 的间接写入规则。[Rust 赋值表达式](https://doc.rust-lang.org/reference/expressions/operator-expr.html#assignment-expressions)

C 的指针常量与指向常量的指针是不同限制；Flurry let 的限制也不能替代目标访问资格。C++ 赋值运算符可由类型自定义，而 Flurry 的覆盖动作则遵循本节的新值交付与旧值清理次序。Zig 的 const/var 区分也不应直接用来推导 Flurry 对象的深层可变性。

## 运行时实现设计引导

实现可以先在临时位置构造新值，再清理目标并安装；也可以消除临时复制，但须保持失败后状态及可观察次序。

执行旧值清理时，新值仍是一个需要正确交付或清理的值。实现不能在开始 Drop 时就遗忘新值，也不能通过先覆盖目标再清理来丢失旧资源的句柄。

## 编译器实现引导

编译器将覆盖表示为"准备新值—旧责任清理—安装"三个语义动作，并保留每步的正常与非正常后继。目标表达式的求值结果只计算一次，但在后续可能改变状态的操作后，编译器仍需核验其访问前提。

编译器为绑定保存稳定的声明次序和当前初始化状态。赋值只更新后者。分支合流保留各路径上的当前责任，不能把某条分支完成的新值安装当作所有路径都已初始化。

验收包括：右侧错误、右侧自身写目标、右侧移走旧值、旧 Drop 读取环境，以及赋值后退出的固定清理顺序。
