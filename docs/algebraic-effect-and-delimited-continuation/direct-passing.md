# 安装时求值与直接传值

## Intro

一组函数常常需要同一份配置，却各自只使用其中几项。无参数的 direct passing 操作把这些需求分别列出：安装处理器时取得值，调用链传递所需的值。

操作仍能被局部覆盖。调用者不必为了覆盖一个配置项而重新构造包含所有配置的大对象。

## 语法

~~~text
direct_passing_effect -> ^attrs.effects.direct_passing effect id() -> expr
effect_variant -> attribute* async? inline? id(parameter*) (-> result)?
effect_enum -> effect enum id clauses? { effect_variant* }
effect_type -> #expr expr
~~~

无主体的声明以分号结束；effect enum 的项以逗号分隔。属性可以放在独立操作或单个 variant 上；被标记操作不能有参数，包括 catch 参数。

## 正式定义

### 安装与请求

安装一个 direct passing handler 时，在其外层环境中执行提供值的分支一次。完成后才开始被处理主体。该安装实例保存所得结果，主体及其调用链中的请求取得这个结果，不重复执行分支。

这是该属性的行为契约。即使主体未发出请求，安装时的求值仍然发生；不能因为最终没有读取结果，就消去具有可观察行为的安装过程。

分支需要的错误处理、外层效应与资源依赖，在安装处检查。分支未完成时，主体尚未开始。无参数操作没有根据请求实参选择不同分支的工作；结果类型仍由操作签名规定。该操作采用安装时的同步提供契约，不能同时要求每次请求启动 async handler；提供分支自身的外层行为仍按实际接口检查。

### 行与隐式传值

每个操作仍有独立身份。#Args 引用整个操作组，#Args.limit 只要求其中的 limit。函数只传递实际接口要求的项目，不要求构造整组的运行时记录。

这里的隐式传值由效应接口的 elaboration 产生。操作名不会自动成为函数签名中的命名参数。需要覆盖时安装一个更内层的 handler；已经声明的普通 implicit 参数另按其参数协议供应。

属性不提供隐式 Clone。一个可复制的数值可以被反复交付，指针的复制保留其可达依赖；一个只能消费一次的结果仍按实际使用检查，不能从多次请求复制 owner。

### 覆盖与恢复

每个安装实例有自己的结果值。内层覆盖不修改外层实例；离开内层后继续使用外层提供的值。

continuation 恢复时，被捕获计算内部的 handler 随计算重新接入；外层入口由恢复点供应。对外层 direct passing 操作，后续请求取得新入口已经保存的值，不重新执行原来的安装分支。

已经取得并保存在普通局部变量中的值保持原值。重接效应入口不会追溯修改此前读取的数值或指针。若该旧值仍携带外部依赖，恢复也必须满足它的存活与访问条件。

## 常见示例

### 两次请求只执行一次提供分支

~~~flurry
^attrs.effects.direct_passing
effect limit() -> u32;

fn sum_limits() -> #[limit] u32 {
    let first = limit()#;
    let second = limit()#;
    first + second
}

test {
    let total = sum_limits()# {
        limit() => {
            println("install limit");
            8u32
        },
    }
    println("{}", total);
}
~~~

输出一行 install limit，随后输出 16。将此操作改为普通 effect 后，两次请求会分别执行提供分支；属性因此不能只被当作优化提示。

### 分组接口只要求一项

~~~flurry
effect enum Settings {
    ^attrs.effects.direct_passing
    width() -> u32,
    ^attrs.effects.direct_passing
    height() -> u32,
}

fn twice_width() -> #Settings.width u32 {
    let width = Settings.width()#;
    width * 2u32
}

fn area() -> #Settings u32 {
    let width = Settings.width()#;
    let height = Settings.height()#;
    width * height
}

test {
    let doubled = twice_width()# {
        Settings.width() => 6u32,
    }
    let size = area()# {
        Settings.width() => 6u32,
        Settings.height() => 4u32,
    }
    println("{} {}", doubled, size);
}
~~~

输出 12 24。twice_width 不需要 height 的处理入口。

### 局部覆盖

接续第一个 limit 声明：

~~~flurry
fn with_override() -> #[limit] u32 {
    let before = limit()#;
    let inside = sum_limits()# {
        limit() => 3u32,
    }
    let after = limit()#;
    before + inside + after
}

test {
    let result = with_override()# {
        limit() => 10u32,
    }
    println("{}", result);
}
~~~

输出 26。before 和 after 都是 10，inside 是 6。内层安装不会把外层提供的值写成 3。

### 参数不符合该 ABI

以下声明被拒绝：

~~~flurry
^attrs.effects.direct_passing
effect lookup(key: u32) -> u32;
~~~

不同 key 的请求需要保留一次次操作，不能由安装时的一个结果替代。应使用普通 effect，或先把待传递对象取得为无参数配置，再在普通函数中查询。

## 与 Rust / Zig / C / C++ 的关键差异

这些语言没有与本篇效应属性直接对等的构造。实际替代是显式上下文参数、C++ 模板参数或闭包捕获；其中普通运行时配置可以通过指针逐层传递。Flurry 的操作行分别记录需求，handler 安装规定每个值的取得时刻。闭包捕获的具体方式仍由各语言定义，例如 [Rust 捕获规则](https://doc.rust-lang.org/reference/types/closure.html#capture-modes)。

## 运行时实现设计引导

安装过程维护未就绪、已就绪、正在退出三个状态：

1. 在外层环境中计算提供分支。
2. 将结果及其责任放入该 occurrence 的值槽，再把槽标为已就绪。
3. 执行主体；请求从本次合法入口取得值或对应传递参数。
4. 退出安装时，只清理仍由实例持有的责任。

请求不得读取未就绪槽。分支失败时按其真实退出方式清理已建立对象，不能启动主体后补算配置。多项安装的依赖和次序按安装接口确定，不由哈希表遍历顺序决定。

无挂起的路径可把值直接放进参数或寄存器。可恢复边界需要区分固定局部值与将来请求使用的外层供应槽；更新后者不得覆盖前者。

## 编译器实现引导

1. 读取操作属性，拒绝带参数或带 continuation 的 direct passing 声明。
2. 为每个完整操作实例保留结果类型与 occurrence 身份。
3. 将提供分支接入安装位置，检查其外层行为、结果交付和依赖。
4. 计算函数实际需要的隐式值接口，沿调用与开放行实例化传递。
5. 将请求降为合法的取值／交付，而不是再次调用提供分支。
6. 对消费、Copy、可达依赖和清理进行普通资源检查。
7. 在 continuation 恢复处刷新需要重绑定的外层供应，保留主体内的实例和值快照。

检查应覆盖无请求仍安装、重复请求只提供一次、局部覆盖、单项行、非复制结果被重复消费，以及恢复前后普通局部值不被改写。
