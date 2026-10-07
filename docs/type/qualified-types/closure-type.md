# 闭包环境限定

## Intro

一个值有时需要额外环境才能使用。闭包限定把这种环境放进类型描述，使值跨过原来的作用域边界时，仍能说明依赖从哪里取得。这一机制受到 Contextual Modal Type Theory (CMTT) 的启发：通过在类型中显式记录上下文依赖，使类型系统能精确追踪值对外部环境的需求，而不是将环境信息隐藏在实现细节中。

函数闭包是一个常见例子；同样的机制也可以限定依赖运行时对象的非函数值。

## 语法

~~~text
closure_type -> %closure_context expr
closure_context -> expr | { environment_binding* }
environment_binding -> id : expr (= expr)?
fn_type -> pure? comptime? inline? (unsafe | spec | verified)? fn(parameter_type*) (-> expr)?
~~~

%C T 中 C 表示环境上下文，T 表示主体类型。环境绑定可写成 `%{name: U = value} T`，说明绑定的类型及来源；开放环境参数使用 `C: Type.vars.Closure`。例如 `%{space: Space = space} dyn Drop` 保留动态主体需要的环境。

## 正式定义

### 主体与上下文

环境限定可用于一般主体类型。一个归一主体最多有一个 Closure 上下文；需要多个环境时先显式合成上下文，不把重复限定当作行并集。

空环境限定等于主体类型。对函数，这意味着没有所需环境的闭包与相应函数类型接合；非函数主体也适用空环境消去。普通 fn 目标不隐式容纳非空捕获；需要环境时在完整接口中保留 `%C`。

闭包默认按值捕获，Copy 复制、非 Copy 移动；独立字段、调用后状态和清理规则见[Lambda](../../functions/lambdas.md)及[闭包环境](../../functions/closure-environments.md)。闭包类型不作为用户 impl 的主体，泛型实现也不能绕过这一资格限制；调用及其他语言能力按相应内建规则取得。

上下文记录绑定及依赖关系；它不是 error/effect 行。类型等价检查保持依赖有效的声明次序，不自动引入任意环境排列的等式。

### 等价

一致改名局部环境绑定不改变类型。外部 runtime identity 不是可以随局部名字一起交换的变量。

函数体或普通捕获值不同，不单独制造不同闭包类型；若其参与类型依赖，则必须比较相应依赖。闭包不按每个构造点产生独立名义身份。

环境类型等价与实际布局分开。符合语义的布局可以共享或消除冗余环境，但捕获不会自动延长被依赖对象的生命。

### 阶段边界

闭包环境与代码引用中的 contextual 类型分别约束自己的对象。不能从闭包的改名等价推出任意 staging 替换规则，也不能把代码描述当作已经执行的 runtime 值。

## 常见示例

在函数内创建两个需要偏移量的函数值，然后分别调用：

~~~flurry
fn show_offsets(first_offset: u32, second_offset: u32) {
    let first = |value: u32| -> u32 value + first_offset;
    let second = |value: u32| -> u32 value + second_offset;

    println("first: {}", first(5));
    println("second: {}", second(5));
}

test {
    show_offsets(10, 20);
}
~~~

输出为 `first: 15`、`second: 25`。两个主体都接收一个 `u32` 并返回一个 `u32`，但运行时操作还分别需要偏移量。环境类型说明这些值怎样取得；环境中的 `10` 与 `20` 仍是两个不同的实际值。

换一种函数体也不必产生新的名义闭包身份：

~~~flurry
fn compare_operations(offset: u32) {
    let add_offset = |value: u32| -> u32 value + offset;
    let keep_larger = |value: u32| -> u32 {
        if value < offset {
            offset
        } else {
            value
        }
    }

    println("sum: {}", add_offset(5));
    println("larger: {}", keep_larger(5));
}

test {
    compare_operations(10);
}
~~~

输出为 `sum: 15`、`larger: 10`。类型比较关注主体签名及所需环境的关系，不以函数体文本作为名义身份。相同类型也不会让两段函数体变成同一个行为。

这里让 lambda 推导自己的环境，读者无需手写环境描述。对于非函数值，[节点索引示例](../runtime-dependent-type.md#常见示例)展示存储 identity 怎样约束一个整数索引的使用；闭包限定的环境规则同样适用于这类依赖。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 与 C++ 的闭包表达式产生各自独特类型；Flurry 不按构造点区分，而比较主体与环境关系。C 使用函数指针加上下文，Zig 可以显式携带环境；这些表示手段可作实现类比。[Rust closure](https://doc.rust-lang.org/reference/types/closure.html)、[C++ lambda](https://eel.is/c++draft/expr.prim.lambda)

## 运行时实现设计引导

实现先确定主体操作需要的环境，再选择承载位置。函数可使用隐藏环境参数；非函数操作同样可以从外层取得依赖，不必改变主体本体。

跨边界时不能丢掉必需环境。若环境从外部对象取得，需保留对应 identity 与资源有效性；不能只保存一个同类型但无关的对象。

## 编译器实现引导

类型比较使用绑定对应关系，而非源码名称：

1. 比较主体种类及相关类型结构。
2. 按声明次序对应上下文绑定。
3. 在已有对应下比较每项类型与依赖。
4. 对局部名字做一致替换；对外部 identity 使用其自身等式。
5. 比较替换后的主体依赖。
6. 独立生成保持该契约的环境布局。

可使用绑定索引或显式替换避免捕获；类型等价无需执行函数体。上下文中含未知参数时保留残余，不能因为环境字段数相同便合并。

CMTT 提供显式上下文与替换的理论参考；本篇的环境范围、次序和交付约束是 Flurry 的具体契约。[Contextual Modal Type Theory](https://software.imdea.org/~aleks/papers/cmtt/cmtt.pdf)
