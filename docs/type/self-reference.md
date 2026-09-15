# Self、Itself 与 itself

## Intro

Self 指向词法所属类型；Itself 和 itself 指向当前要求检查的类型与值。阅读嵌套要求时，先找名字在哪里绑定，比把它们都理解成“自身”更可靠。

## 语法

~~~text
self_param -> self
self_ref_param -> *self
itself_param_type -> itself
itself_ref_param_type -> *itself
refinement_type -> refines { expr } expr
~~~

星号接收者表达指针关系；其余表达式同全局语法。

## 正式定义

| 名字 | 指称 |
|---|---|
| Self | 词法所属类型，或 trait/implementation 实现主体 |
| Itself | 最内层明确受检对象的类型；类型要求检查 T 时指 T |
| itself | 当前受检值；实际存在值对象时才有普通值绑定 |

检查类型 T 时，Itself 不因 T 是 Type 值而指向 Type。只有类型要求时，不生成假想运行时实例。

成员签名里的 itself 和 *itself 是接收者关系槽，说明接收者与受检对象的值关系和指针关系，不是把任意 comptime 值拿来构造指针类型。

嵌套要求重新绑定局部受检对象，Self 保持词法含义。没有相应绑定的位置使用这些名字是静态错误。

## 常见示例

词法 Self 可以用于构造同类结果；方法参数中的精化则可以检查另一个类型：

~~~flurry
struct Counter {
    value: u32,

    fn next(self) -> Self {
        Self { value: self.value + 1 }
    }

    fn accepts_tuple(*self, value: refines {
        Type.predicates.is_tuple(Itself)
    } Any) -> u32 {
        self.value.*
    }
}

test {
    let counter = Counter { value: 4 }
    let next = counter.next();

    println("counter: {}", next.value);
    println("tuple accepted: {}", next.accepts_tuple((10u32, true)));
}
~~~

两次都输出数值 `5`。`next` 中的 `Self` 是 `Counter`，因此能用命名字段构造结果。`accepts_tuple` 的要求检查元组参数，局部 `Itself` 是该参数的类型；它不会变成 `Counter`，也不会升一层变成 `Type`。

下面的调用应因不满足精化要求而拒绝：

~~~flurry
test {
    let counter = Counter { value: 5 }
    counter.accepts_tuple(10u32);
}
~~~

整数参数不满足 tuple 分类。这个检查仅要求分类，没有从 `Itself` 虚构一个元组实例或假定它含有几个元素。

普通值 `itself` 只在存在受检值的要求中绑定；成员签名中的 `itself`、`*itself` 则表示接收者关系槽。不要把上面类型要求里的 `Itself` 机械改成小写后当作可读取的对象。

## 与 Rust / Zig / C / C++ 的关键差异

Rust Self 可类比词法实现主体；C++ this 是对象指针，不能类比 Itself 的类型绑定。C 和 Zig 可使用显式接收者参数。[Rust Self](https://doc.rust-lang.org/reference/paths.html#self-1)、[C++ this](https://eel.is/c++draft/expr.prim.this)

## 运行时实现设计引导

类型指称静态消去。itself 仅在存在实际值绑定时对应运行时值；签名接收者槽由选定操作解释，不能生成假想对象。

## 编译器实现引导

分别维护词法主体栈和局部受检对象栈。解析 Itself 先区分类型要求与值要求，避免直接取表达式的 type 而多升一层。

进入嵌套要求压入局部绑定，退出恢复；证明与缓存使用稳定绑定身份。未绑定名字在此拒绝，不交推断器猜测。
