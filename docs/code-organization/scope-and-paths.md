# 作用域、路径与遮蔽

## Intro

名字先确定一个声明，再检查这个声明能否用于当前位置。一个局部变量遮蔽了类型名，类型位置不会跳过它重新寻找同名类型。

路径让程序明确指定查找的起点。后续投影沿已经选定的容器前进。

## 语法

~~~text
path -> id
    | projection_path
    | projection_multi_path
    | projection_all_path
    | super_path
    | path_as_bind
projection_path -> path . id
projection_multi_path -> path . { path* }
projection_all_path -> path . *
super_path -> . path
path_as_bind -> path as id
label -> : id
~~~

这些 path 形式用于 use。普通表达式投影仍用 `expr.id`；`@` 仅用于宏相关语法，宏导入 `@use` 见[构造宏](../macro/construction-macro.md)。

## 正式定义

### 统一命名空间

Flurry 采用统一命名空间以避免不同名称类别之间的隐式回退查找。普通程序名称共用一个命名空间，包括类型、值、模块与 label。使用位置对解析结果提出种类与类型要求，而不是选择另一张名称表。

静态声明集合不允许以重名建立函数重载或重复字段。顺序绑定和嵌套作用域可以遮蔽名称；遮蔽不会改变被遮蔽声明的身份或已有引用。

### 查找次序

从最内层合法作用域向外逐层查找。在每一层：

1. 直接声明与显式导入优先于通配来源。
2. 两个确定命名项指向不同同名声明，形成重名错误。
3. 没有确定命名项时，汇集通配来源；相同声明身份合并，不同身份保留歧义。
4. 本层有结果或歧义后，不继续用外层同名项替换它。

内部通配来源可以遮蔽外层显式导入。访问失败或结果种类不符也不能触发另一轮“寻找可用同名项”。歧义的报告时机见[导入规则](imports-and-reexports.md)。

### 顺序绑定

let／var 的新绑定在初始化完成后进入后续环境。初始化表达式按引入新绑定之前的环境解析，因此可以读取同名旧绑定。

同一顺序块中的重复 let 建立新的绑定，不是给旧值赋值。旧绑定自己的清理责任遵守[资源管理](../resource-management/bindings.md)，不会因为失去短名而被删除。

### 路径起点

| 形式 | 查找起点 |
|---|---|
| 普通首名 | 当前合法词法环境，逐层向外 |
| 前导点 `.` | 所属静态容器的父静态容器（连续前导点逐级向上） |

运行时块和函数体不增加前导点所计的静态容器层级。满足无子句限制的类型容器参与该包含关系。越过 package 根是静态错误，umbrella 不提供更外层语言作用域。

显式定位后只查指定容器的成员，找不到时不回退祖先。投影后续各段同样只查已选容器。某个名字在容器内部可经词法父级找到，不意味着它是这个容器的成员。

成员一旦被选中，后续实例化保留该来源；类型和值的投影规则见[成员投影](../type/member-projection.md)。

## 常见示例

### 包根导入供子模块使用

~~~flurry
use std.prelude.*;

mod report {
    fn show() {
        println("ready");
    }
}

fn main() {
    report.show();
}
~~~

show 可以通过父级作用域找到 println；这不使 report.println 成为 report 自己的成员。

### 父静态容器逐级定位

~~~flurry
struct RootToken {}

mod codec {
    struct LocalToken {}

    mod decoder {
        use .LocalToken;
        use ..RootToken;

        fn local_token() -> LocalToken {
            LocalToken {}
        }

        fn root_token() -> RootToken {
            RootToken {}
        }
    }
}
~~~

第一条 use 从父容器 codec 取 LocalToken，第二条向上两级从包根取 RootToken。

### 类型容器也是相对起点

~~~flurry
struct Outer {
    struct Token {}

    struct Inner {
        use .Token;

        fn token() -> Token {
            Token {}
        }
    }
}
~~~

这里的父静态容器是 Outer，不是文件所在目录。Token 的类型名不会因为函数名 token 使用小写而冲突。

### 新绑定读取旧绑定

~~~flurry
use std.prelude.*;

test {
    let value: u32 = 20;
    let value = value + 1;
    println("{}", value);
}
~~~

输出 `21`。第二个初始化表达式中的 value 指向第一个绑定。

### 标签不会另开查找通道

以下代码静态拒绝：

~~~flurry
fn invalid_label() {
    while : retry true {
        let retry: u32 = 1;
        break retry;
    }
}
~~~

break 找到的是局部数值绑定，不能作为循环目标。解析器不能跳过它，重新选外层标签。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 将类型、值、label 等放在不同命名空间，Flurry 在这里统一查找。Zig 禁止以同名隐藏另一标识符，Flurry 允许词法遮蔽。[Rust 命名空间](https://doc.rust-lang.org/reference/names/namespaces.html)、[Zig shadowing](https://ziglang.org/documentation/master/#Shadowing)

C 对 label、tag、成员和普通标识符有不同的名字空间；C++ 的名字查找也包含依赖使用位置的规则。Flurry 不用类型或 label 位置跳过已选中的同名绑定。[C11 §6.2.3](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf)、[C++ 名称查找](https://eel.is/c++draft/basic.lookup)

## 运行时实现设计引导

遮蔽后的两个局部绑定在语义上仍是不同位置。寄存器和栈槽可以在满足存活与资源规则后复用；名字相同不是提前销毁或共享存储的依据。

label 解析为明确控制流目标。降低到分支指令之前，必须检查它确实是当前控制结构中合法的标签，不能把同名局部值解释成地址或编号。

## 编译器实现引导

使用稳定绑定身份记录解析结果。静态名字表与顺序环境各自记录声明可用位置，但不按类型／值／标签拆成彼此可回退的名字空间。

~~~text
lookup(name, environment):
    for frame in environment.innermost_first():
        explicit = frame.explicit_binding(name)
        if explicit exists:
            return validate_selected(explicit)
        candidates = frame.glob_candidates(name)
        if candidates is not empty:
            return validate_selected(deduplicate_origins(candidates))
    return Missing
~~~

显式绑定可以暂处待解状态；它仍占住本层位置，不能暂时用通配项填补。validate_selected 区分歧义、访问失败与使用种类错误，不在失败时继续外层循环。

每次进入顺序声明先解析初始化，再分配新绑定身份并更新后续环境。闭包捕获保存选中的身份，不因之后出现同名绑定而改变捕获对象。

静态路径先解析定位，再逐段查询成员。缓存键至少区分词法查找与指定容器成员查找，并携带访问者；否则同一个名字会在不同访问范围下复用错误结果。复杂度由访问的作用域深度、路径长度和候选数量决定；同名来源的全部候选须参与去重与歧义判断。
