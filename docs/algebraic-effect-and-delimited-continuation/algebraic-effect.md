# 操作声明与直接处理

## Intro

效应操作给计算留出一个由外部回答的问题。函数可以请求一个数值、提交一条消息，或要求外部完成一次操作，而把具体处理方式交给安装在调用环境中的 handler。

直接处理的分支交回操作结果，主体便从请求之后继续。分支可以请求外层效应，所以嵌套处理也能逐层组合行为。

## 语法

~~~text
algebraic_effect -> async? inline? effect id(parameter*) (-> result)? clauses?
effect_enum -> effect enum id clauses? { effect_variant* }
effect_variant -> attribute* async? inline? id(parameter*) (-> result)?
effect_type -> #expr expr
perform_effect -> expr#
effect_handle -> expr# { case_arm* }
case_arm -> pattern => (block | statement)
resume_statement -> resume expr? (while expr)?
~~~

声明没有函数主体，以分号结束；省略结果时为 void。effect enum 的项采用操作签名，并非普通 enum 的载荷声明。

## 正式定义

### 操作、组与允许行

每个独立操作实例形成标签。effect enum 组织一组操作，E.v 表示一个操作，E 表示整组需求；编译器仍分别检查每个操作的实参与结果。

参数化操作按完整实例区分。相同声明在不同参数下的标签是否相同，依其类型与参数关系判定，不能只比较源码名字。

允许行说明主体可能请求哪些入口。主体可以不请求，也可以多次请求；顺序组合在共同允许环境下检查，不按执行次数复制行中的标签。

### 直接分支的返回

设操作接收 X 并返回 Y。直接分支必须按 Y 交付结果；分支不取得 continuation。分支的普通结果返回请求点，resume 则提前交付操作结果并结束该分支。

分支使用其安装位置的外层效应环境，避免同名请求重新进入自己。主体恢复执行后仍处在原处理器内。因此第一次请求结束不会移除后续请求需要的入口。

显式的 return、break 和 continue 保留各自合法的词法目标。若目标要求退出正在被处理的主体，需要执行控制后端的退出与清理协议，不能把这些动作重写为一次普通操作返回。

### 错误值属于操作结果契约

操作可以把 !E T 作为 Y。此时 handler 交付的错误是一次正常的操作结果；主体是否传播或消去错误，由自己的 ! 或 match 决定。

若 Y 是 u32，分支不能把 error E 作为 u32 返回。效应处理器的安装不会让一个错误值绕过结果类型检查，也不会将错误标签加入效应行。

### 部分处理与转发

分支没有覆盖某个请求，或 guard 失败时，需要存在合法外层入口。转发仍携带主体恢复后重新进入当前 handler 的关系。没有外层供应就拒绝未覆盖请求，不能把“没有匹配分支”当成隐式成功。

### Inline

inline effect 表达内联意向。编译器可以联合展开调用方、被调用计算和 handler，使请求变为局部计算；无法完成时采用兼容的普通 evidence 路径。inline 不改变捕获许可、async 子工作归属或 direct passing 的安装时求值行为。

例如，内层只处理 `true`，其余请求交给外层。一次转发不会卸载内层；第三次 `ask(true)#` 仍由它回答。

```flurry
inline effect ask(value: bool) -> bool;

inline fn produce() -> #ask bool {
    let a = ask(true)#;
    let b = ask(false)#;
    let c = ask(true)#;
    not a and b and not c
}

inline fn filtered() -> #ask bool {
    produce()# { ask(true) => false, }
}

fn entry() -> bool {
    filtered()# { ask(value) => not value, }
}
```

结果为 `true`。当前可选展开器将这类部分 direct handler 降为有序匹配和最后的外层转发分支。候选失败先撤销绑定，转发交付原载荷；不会重新运行请求实参，也不会把拥有型载荷自动变为 Copy。对安装者循环的 `break` / `continue` 仍按原词法目标清理被退出的生产者和中间处理层。


当前可选优化入口已支持封闭的 direct inline effect：先展开普通 inline callee，再将请求替换为卫生重命名的局部 pattern match。每次请求保留一次参数求值，分支可读写安装位置的外层绑定；显式 resume 通过请求私有的结果边界提前交付，只结束当前分支，后续请求仍可继续；return、break 和 continue 另行保持原函数或词法循环目标，不被 resume 的结果边界捕获；handler 自身的循环与其中已经展开的 inline/quote 调用同样保留局部退出和清理边界；不含效应的闭包保留独立调用边界及捕获责任；转换后重新检查资源并发布析构计划。命令行通过 `--interpret entry --inline-functions --copy-protocol Copy --drop-protocol Drop` 显式启用，协议名按实际源定义填写。

可独立封闭的嵌套安装先展开内部，再处理外部。例如内部安装虽然允许使用词法环境中的 `poll`，实际只请求自己处理的 `read`，两层都可展开：

```flurry
inline effect read() -> bool;
inline effect poll() -> bool;

inline fn nested() -> #poll bool {
    let inside = read()# { read() => true, };
    inside and (poll()#)
}

fn entry() -> bool {
    nested()# { poll() => true, }
}
```

内部的完整效应行、槽位及操作身份仍要核对。若它实际请求外层操作，或需要未展开 callee 的动态效应路由，当前优化保留相应安装。已独立成功的内层展开可以保留；外层失败只回滚本次外层尝试产生的代码与元数据，不撤销有效的内层结果。

callee 也可只使用调用方行中的一部分操作，或以不同顺序声明它们：

```flurry
inline fn only_poll() -> #poll bool { poll()# }
inline fn wider() -> #[read, poll] bool { only_poll() }

fn mapped_entry() -> bool {
    wider()# { read() => false, poll() => true, }
}
```

这里 callee 的槽 0 对应调用方的 `poll` 槽，而不是 `read`。展开逐层组合 EffectRun 槽映射，并核对操作声明及类型实参，之后才选择 handler 模板；子集映射、顺序调整及 `resume` 不改变普通实参的求值次数和清理次序。错误槽位或不匹配的操作身份使该安装尝试回滚，不发布部分代码。

当前转换不处理外层效应行、部分转发、direct passing/capturing/async、带效应的闭包／嵌套效应，也不跨未展开的 effectful callee。遇到这些情况或候选预算不足，整段安装保留普通路径；已经尝试生成的局部身份与来源记录回滚。这是实现范围，不改变上述语言语义。


## 常见示例

### 用同一主体连接两种处理方式

~~~flurry
effect read_number() -> u32;

fn twice_input() -> #[read_number] u32 {
    let number = read_number()#;
    number * 2u32
}

test {
    let first = twice_input()# { read_number() => 7u32, }
    let second = twice_input()# { read_number() => 11u32, }
    println("{} {}", first, second);
}
~~~

输出 14 22。主体无需知道数值由常量、设备或测试驱动提供。

### 分组的操作各有签名

~~~flurry
effect enum Report {
    count() -> u32,
    message(text: str),
}

fn report_twice() -> #Report u32 {
    Report.message("begin")#;
    let count = Report.count()#;
    Report.message("end")#;
    count * 2u32
}

test {
    let result = report_twice()# {
        Report.count() => 4u32,
        Report.message(text) => println("{}", text),
    }
    println("{}", result);
}
~~~

依次输出 begin、end、8。message 返回 void，count 返回 u32；整个组没有统一的载荷或结果类型。

### 同名请求向外传递

~~~flurry
fn with_halved_input() -> #[read_number] u32 {
    twice_input()# {
        read_number() => {
            let original = read_number()#;
            original / 2u32
        },
    }
}

test {
    let result = with_halved_input()# { read_number() => 12u32, }
    println("{}", result);
}
~~~

输出 12。内层分支从外层得到 12，再向主体交付 6；它不会递归调用自身。

### 提前交付操作结果

~~~flurry
effect scale(value: u32) -> u32;

fn use_scale() -> #[scale] u32 {
    let first = scale(0u32)#;
    let second = scale(4u32)#;
    first + second
}

test {
    let result = use_scale()# {
        scale(value) => {
            if value == 0u32 { resume 1u32; }
            value * 2u32
        },
    }
    println("{}", result);
}
~~~

输出 9。resume 只结束处理当前请求的分支，主体随后仍会发出第二次请求。

### 操作返回一个错误限定值

~~~flurry
enum InputErr { Missing, }
effect try_read() -> !InputErr u32;

fn read_or_zero() -> #[try_read] u32 {
    let input = try_read()#;
    input! { .Missing => 0u32, }
}

test {
    let result = read_or_zero()# {
        try_read() => error InputErr.Missing,
    }
    println("{}", result);
}
~~~

输出 0。效应已成功交付一个 !InputErr u32；随后由主体处理其中的错误。若将 try_read 的结果改为 u32，原 handler 的错误分支便不满足接口，程序被拒绝。

## 与 Rust / Zig / C / C++ 的关键差异

无对等的通用操作行与 handler 安装。C 可显式传递函数指针和环境，Rust、C++ 可传闭包，Zig 可传函数或编译时接口；这些方式都能实现请求／响应，但不会自动提供 scoped occurrence 的路由关系。具体闭包环境也不能直接替代效应行。[Rust 闭包](https://doc.rust-lang.org/reference/types/closure.html)

## 运行时实现设计引导

直接入口可以表示为处理函数及其环境。调用时先取得普通实参，再调用选定入口；分支完成后把 Y 放入请求结果位置。

进入分支时切换逻辑处理上下文，返回主体时恢复原上下文；逻辑边界不要求每次都切换物理栈。分支请求外层可捕获操作时，整个调用链仍需遵守可挂起调用的契约。

部分处理先判断模式与 guard；不覆盖时转交外层入口。失败匹配不能消费后续入口仍需要的参数，也不能丢失重新安装内部 handler 的关系。

handler 的 guard 和参数中的顺序计算模式遵循普通候选观察规则，可以调用具名标量函数，也可通过工厂、局部计算模式或 match 取得已知 Copy 代码再调用。它们的读取足迹进入安装函数的内存摘要，局部赋值、临时资源及调用清理边界属于当前 handler 的运行环境。guard 失败后，候选计算已经完成的清理不重放；请求转发到外层，恢复主体后仍使用原内层 handler。清理发布器单独遍历这些模式计算，不把它们当作函数前置条件。

对封闭的 direct inline effect，联合展开把这些候选变成每个请求自己的局部 match。参数中的计算模式及 guard 内部绑定都需卫生重命名；生产函数和回调的求值次数、候选顺序、临时资源与请求参数的清理顺序保持一致。普通分支结果与 `resume` 都遵守这一要求。展开本身不提供不拒绝依据：工厂或它返回的目标可能拒绝时，展开后仍须通过同样的失败路径资源检查。

## 编译器实现引导

1. 将操作组展开为独立操作身份，保留组投影用于查找与诊断。
2. 按普通参数规则检查请求实参，并取得操作结果 Y。
3. 在共同允许环境中检查多次请求，保持同名 occurrence 的次序。
4. 检查分支的模式、结果及外层行为；错误结果只按错误类型规则接合。
5. 为未覆盖路径建立转发证据，为控制退出建立清理后继。
6. 根据 inline 意向尝试联合展开；无法展开时维持原 ABI 和行为。
7. 检查目标中不留缺失入口、错误 occurrence 或未满足的结果交付。

复杂度应分别记录路由、参数处理、分支匹配和实际处理工作。evidence 免去动态搜索，并不意味着任意分支都没有执行成本。
