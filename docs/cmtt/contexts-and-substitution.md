# 上下文、绑定身份与替换

## Intro

开放代码需要的输入是一组绑定。替换把这些接口绑定接到当前位置已有的对象，并同时更新主体和依赖类型。

默认对应让模板正文直接使用模板提供的名字；显式替换则用于改名、重定向或消歧。

## 语法

~~~text
contextual_expr_type -> < runtime_binding* |- expr >
contextual_pattern_type -> < expr -| runtime_binding* >
quote_expr -> '(expr) | '[ substitution* ] (expr)
quote_pattern -> '<pattern> | '[ substitution* ] <pattern>
splice_expr -> ~<expr> | ~[ substitution* ] <expr>
splice_pattern -> ~<expr> | ~[ substitution* ] <expr>
substitution -> id = id
runtime_binding -> id : expr
~~~

## 正式定义

### 有顺序的上下文

Ψ 是按依赖次序排列的 runtime 绑定，不是错误行或效应行。后项的类型可以依赖前项允许携带的 runtime identity；这种依赖不意味着生成器能够读取对象的运行时内容。

一致的局部改名不改变接口含义。但两个外部对象同型或同名，不意味着它们是同一个对象。替换必须保持接口声明的依赖，不能只逐项比较类型形状。

### 映射方向

替换表的左侧指代码接口名，右侧指该次构造或接入使用的名字：

| 位置 | 例子 | 含义 |
|---|---|---|
| expr quote | '[c = a](a + b) | 将源码绑定 a 暴露为接口 c |
| expr splice | ~[c = w]<code> | 用当前 w 供应接口 c |
| pattern quote | '[out = inner]<inner> | 将模式导出 inner 暴露为 out |
| pattern splice | ~[out = found]<pattern> | 匹配成功时引入 found |

映射同时发生，交换两个名字不按文本顺序覆盖。右侧仅为标识符，不接收任意有副作用的表达式；需要先计算实参时，先建立普通 runtime 绑定。

pattern splice 引入新绑定，不对同名旧变量赋值。表达式的两个接口可以映射到同一对象，但随后必须按真实别名、访问和消费关系检查；pattern 的两个独立导出不能无依据合并。

### 默认对应

默认接入按已经建立的绑定关联进行：引用保留的原绑定、模板授予的接口、pattern 成功上下文，以及 implicit quote 参数提供的对应。

名字在相应接口作用域内用于解析。一旦身份确定，后来出现的同名变量不会自动替换它；相同类型也不提供这种替换依据。对应缺失或有歧义时必须补充显式映射，不能任选一个同型变量。

显式表可以覆盖可替换的接口项，其他项继续使用合法默认对应。固定外部依赖不能凭一个改名伪装成自由端口。

### 控制与资源

替换同时作用于上下文、主体、结果类型和相关控制接口。return、break、continue 的目标不能因接入处恰好有一个同名边界而被重解释。

复制代码值不复制其运行时资源。接入后的别名、移动、读写及清理检查仍以实际绑定为准。默认接入的方便程度不降低这些要求。

## 常见示例

### 引用保留原绑定

~~~flurry
fn read_original(value: i32) -> i32 {
    const code = '(value + 1i32);
    let value = 100i32;
    ~<code>
}

test { println("{}", read_original(41i32)); }
~~~

输出 42。后来的 value 不改变 code 已经记录的形参身份。

### 构造时暴露端口，接入时重定向

~~~flurry
fn redirected(a: i32, b: i32, replacement: i32) -> i32 {
    const code = '[port = a](a + b);
    ~[port = replacement]<code>
}

test { println("{}", redirected(1i32, 2i32, 40i32)); }
~~~

输出 42。显式开放的 port 改用 replacement；b 仍使用引用保存的原绑定。

### 同时交换接口

~~~flurry
fn swapped(left: i32, right: i32) -> i32 {
    const code: < left: i32, right: i32 |- i32 > = '(left - right);
    ~[left = right, right = left]<code>
}

test { println("{}", swapped(3i32, 10i32)); }
~~~

输出 7。两个映射同时解释，不会先覆盖 left 再把同一个值赋给 right。

### 模板提供默认绑定

~~~flurry
template fn with_adjusted(
    value: i32,
    quote lambda body: < adjusted: i32 |- i32 >,
) -> i32 {
    let adjusted = value + 1i32;
    ~<body>
}

fn doubled use with_adjusted { adjusted * 2i32 }

test { println("{}", doubled(20i32)); }
~~~

输出 42。模板明确授予 adjusted，因此正文和 splice 之间已有合法对应，无需重复写同名映射。

### 缺失上下文被拒绝

~~~flurry
fn missing() -> i32 {
    const code: < needed: i32 |- i32 > = '(needed);
    ~<code>
}
~~~

接口中的虚拟 needed 不会自己成为 runtime 值。接入处没有供应，程序被拒绝。

## 与 Rust / Zig / C / C++ 的关键差异

C 预处理宏按 token 替换，Rust 声明宏有其卫生规则；它们都不直接提供本篇带类型的 runtime 上下文替换接口。C++ 模板和 Zig comptime 可以显式传递生成所需信息，但不能据此省略 Flurry 对对象身份的检查。[Rust 宏卫生](https://doc.rust-lang.org/reference/macros-by-example.html#hygiene)

## 运行时实现设计引导

替换在编译时完成，目标程序操作实际供应的对象。普通形参可以建立新鲜绑定，但指针目标和依赖对象不能随形参改名而变成新对象。

代码接入后若需要清理，仍按实际路径执行。某个接口项未被正文使用，不足以证明与之关联的资源责任可以消失。

## 编译器实现引导

1. 以稳定身份区分接口端口、局部绑定、固定外部依赖与全局声明。
2. 从默认关系和显式表建立一次同时替换，诊断重复或歧义目标。
3. 沿上下文依赖次序检查各项，不对多个未知片段做无依据的自由排列。
4. 为接入代码的内部局部变量分配新鲜身份；固定声明保持原来源。
5. 将同一替换作用于主体、结果、依赖及控制接口。
6. 以生成后的实际访问关系重新检查资源和效果。

上下文替换的程序状态义务由程序验证与资源规则处理。PTC 只处理其中属于类型命题的部分，不能充当任意绑定、内存有效性或控制流的求解器。
