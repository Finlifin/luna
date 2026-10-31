# Lambda 与捕获

## Intro

lambda 在表达式中创建一个可调用值。它的参数由调用者提供，外部局部值则在创建时进入捕获环境。

想保留创建时的数值，就捕获值；想观察以后发生的修改，就显式捕获指向相应对象的指针或视图。

## 语法

~~~text
lambda -> |anchored_parameter*| (-> expr)? expr
anchored_parameter -> id | id : expr
refer -> expr.ref
deref -> expr.*
closure_type -> %closure_context expr
closure_context -> expr | { environment_binding* }
environment_binding -> id : expr (= expr)?
~~~

零参数 lambda 写成 `|| expr`。lambda 的参数只允许锚定参数，不能在其参数列表中声明 optional、变参或 implicit；接收 lambda 的函数可以有这些参数种类。

## 正式定义

### 创建与调用

lambda 表达式的求值建立捕获环境，不执行主体。主体在调用时执行。捕获不是把整个主体中所有表达式提前计算一遍，而是取得它将要引用的外部位置及依赖。

默认按值捕获使闭包的资源责任在创建时确定，从而简化生命周期分析：Copy 值复制，非 Copy 值移动。Clone 不自动调用；需要独立克隆时，先显式取得克隆值，再创建闭包。

| 时刻 | 发生的工作 | 尚未发生的工作 |
|---|---|---|
| 求值 lambda 表达式 | 取得捕获并建立环境 | 不执行主体的输出、循环或参数计算 |
| 调用闭包 | 绑定锚定参数，按当前状态执行主体 | 不重新从最初源变量补抓一份捕获 |
| 环境退出 | 清理仍然持有的捕获责任 | 不为了清理而先执行一次主体 |

主体既可以是一个表达式，也可以是块；返回类型可显式标注，或依据目标及主体取得。参数列表中的裸 id 是参数类型的省略，不是一个 Type 参数声明。局部参数和主体内声明遮蔽外部同名绑定时，以真正解析到的绑定为准。

### 指针与观察

捕获普通指针复制其表示并保留真实目标依赖，不复制 pointee，也不延长目标生命。通过该指针的后续读取可以看到合法修改后的内容。

直接引用外部变量的 lambda 不根据主体“只是读了一下”就自动改成借用捕获。共享观察通过显式指针或视图表达。

### 最小捕获位置

对能证明独立交付的内建字段投影，捕获所需的最小位置。判断必须保留类型依赖、对象身份、内存关系和剩余清理要求。

同一位置多次使用只捕获一次。存在父子重叠需求时，捕获计划不能产生两份同一非 Copy 责任。若某字段不能独立交付，不能仅凭文本中出现 `object.field` 就把它从整体中拆出。

用户定义的投影或任意函数调用不因拼写近似字段访问就取得同样的分离证明。更宽捕获也须合法；不满足时拒绝，而不是自动克隆或分配。

| 主体对外部位置的需求 | 捕获分析需要确认什么 |
|---|---|
| 同一字段读取多次 | 去重为同一捕获；读取本身仍逐次发生 |
| 两个互相独立的字段 | 分别交付的合法性及剩余对象的清理 |
| 字段与其整个父对象 | 合并重叠需求，不能生成两份相同责任 |
| 已取得的普通指针 | 捕获指针及目标依赖，后续访问不捕获 pointee 的值副本 |
| 通过调用才能得到的位置 | 按实际调用契约分析，不能假装是内建字段路径 |

### 捕获次序

去重后的首次捕获词法顺序形成环境的语义字段次序。环境的实际布局可以调整，已确认的依赖和可观察清理次序必须保留。清理规则见[闭包环境](closure-environments.md)。

捕获值不同或函数体不同，不按创建位置产生新的名义闭包类型。实际捕获值仍决定运行行为；类型等价不能用来互换不同环境中的对象。

## 常见示例

### 数值捕获保存创建时的值

~~~flurry
test {
    let offset: u32 = 10;
    let add_offset = |value: u32| value + offset;
    offset = 20;
    println("{} {}", add_offset(1), offset);
}
~~~

输出 `11 20`。u32 可 Copy，闭包保存的是创建时取得的副本。

### 指针捕获观察后续修改

~~~flurry
test {
    let current: u32 = 10;
    let source = current.ref;
    let read_current = || source.*;
    current = 20;
    println("{}", read_current());
}
~~~

输出 `20`。指针在创建 lambda 之前显式取得。current 仍存活，修改与读取顺序明确。

### 创建闭包不执行主体

~~~flurry
test {
    let label: u32 = 7;
    let action = || {
        println("body {}", label);
    }
    println("created");
    action();
    println("finished");
}
~~~

输出 `created`、`body 7`、`finished`。捕获 label 在创建时完成，但输出要等到调用。把整个 lambda 主体当作初始化表达式执行，会改变这一顺序。

### 创建时就移出非 Copy 捕获

~~~flurry
struct Ticket {
    number: u32,
}

fn finish(ticket: Ticket) {
    println("ticket {}", ticket.number);
}

test {
    let ticket = Ticket { number: 7 }
    let submit = || finish(ticket);
    submit();
}
~~~

ticket 的责任在创建 submit 时进入环境，不等到调用才从外部位置拿走。调用后相应捕获已消耗，不能再用同一责任调用一次 finish。

Ticket 没有声明 Copy；只有一个 u32 字段不使它自动可复制。调用能力还取决于闭包主体如何使用环境，见[调用后的状态](closure-environments.md#调用后的状态)。

~~~flurry
-- 拒绝：ticket 已在创建 submit 时移入环境。
fn invalid_capture() {
    let ticket = Ticket { number: 8 }
    let submit = || finish(ticket);
    finish(ticket);
}
~~~

### 一个字段可以独立进入闭包

~~~flurry
struct Pair {
    left: Ticket,
    right: Ticket,
}

test {
    let pair = Pair {
        left: Ticket { number: 1 },
        right: Ticket { number: 2 },
    }
    let submit_left = || finish(pair.left);
    finish(pair.right);
    submit_left();
}
~~~

在这个无额外整体依赖的聚合中，两个字段可独立交付，输出 `ticket 2`、`ticket 1`。捕获 left 后，不能把 pair 当作完整值再交付，但 right 仍可使用。

### 显式克隆后再捕获

以下是独立例子，不依赖前面的 Ticket 定义：

~~~flurry
struct Ticket {
    number: u32,
}

impl Clone for Ticket {
    fn clone(*self) -> Self {
        println("clone {}", self.number.*);
        Self { number: self.number.* }
    }
}

fn finish(ticket: Ticket) {
    println("finish {}", ticket.number);
}

test {
    let original = Ticket { number: 7 }
    let captured = original.clone();
    let action = || finish(captured);
    finish(original);
    action();
}
~~~

输出 `clone 7`、`finish 7`、`finish 7`。克隆发生在显式 clone 调用处。闭包取得 captured 的责任，original 仍属于原作用域。Clone 实现可能产生效果，这些效果不应隐藏在捕获规则中。

### 嵌套闭包在各自创建时取得环境

~~~flurry
test {
    let current: u32 = 3;
    let source = current.ref;
    let snapshot = || {
        let saved = source.*;
        || saved
    }
    let first = snapshot();
    current = 9;
    let second = snapshot();
    println("{} {}", first(), second());
}
~~~

输出 `3 9`。外层闭包保存 source，调用时读取本次内容；内层闭包在那次调用中创建，复制 saved。first 的结果不会因 current 后来改为 9 而变化，second 也不会复用第一次 saved 的局部身份。

### Lambda 的类型简写

~~~flurry
fn apply(value: u32, action: fn(u32) -> u32) -> u32 {
    action(value)
}

test {
    println("{}", apply(5, |value| value + 1));
}
~~~

参数类型由目标提供。这里没有环境，lambda 可以接入普通 fn 目标。

显式结果类型可以使接口直接可见：

~~~flurry
test {
    let accepted = |value: u32| -> bool {
        value <= 10
    }
    println("{} {}", accepted(7), accepted(12));
}
~~~

输出 `true false`。lambda 的结果类型可写出，参数仍然只使用锚定形式。需要默认参数或参数协议时，把它们放在接收闭包的函数接口上。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 的默认闭包捕获根据使用方式选择借用或移动，move 闭包强制相应的按值取得；Flurry 默认按值，需观察外部对象时显式传递指针或视图。C++ 有按值及按引用捕获形式；Flurry 不据此引入 C++ 捕获列表语法。[Rust capture modes](https://doc.rust-lang.org/reference/types/closure.html#capture-modes)、[C++ lambda capture](https://eel.is/c++draft/expr.prim.lambda.capture)

C 可显式打包上下文并传给函数；Zig 可用结构体携带状态。Flurry 的捕获规则直接参与源程序的移动与清理检查。[C11 §6.7.6.3](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf)、[Zig structs](https://ziglang.org/documentation/master/#struct)

## 运行时实现设计引导

按捕获计划取得每个环境字段。对 Copy 做合法表示复制，对其他位置移交责任；不能在构造结束后仍让源位置承担同一所有权。

若环境构造在中途失败，只清理已经完成取得的字段。闭包主体未调用时，其环境也仍需要清理。捕获指针只保留依赖，不取得 pointee 的独占销毁权。

## 编译器实现引导

1. 解析 lambda 局部参数和局部绑定，识别自由位置引用。
2. 将合法内建投影表示为根绑定加投影路径；取得每条路径的独立交付依据。
3. 合并相同位置，处理父子覆盖，保留首次捕获位置与所需依赖。
4. 检查取值方式、Copy／move、剩余对象状态和 Drop 义务。
5. 形成有语义次序的环境与独立主体，把自由引用改为环境访问。
6. 分别计算构造、调用与销毁的内存效果及结果依赖。

用投影树合并 m 条引用，总路径长度为 p 时可在 O(p) 结构遍历内完成登记，具体独立性证明另计。不得以类型相同代替位置相同，也不能用字段偏移互异替代所有资源分离条件。

验收覆盖同一捕获多次读取、非 Copy 创建后再用、独立字段、重叠父子捕获、指针目标失效和构造失败清理。
