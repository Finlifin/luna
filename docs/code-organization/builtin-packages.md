# 内置包与名称入口

## Intro

内置包为程序提供基础类型、编译时工具和工程环境。使用它们的成员仍遵循普通名称解析。

Prelude 是一组便于导入的名称。知道它从哪里进入作用域，就能判断短名是否可用，以及与本地名字冲突时如何处理。

## 语法

~~~text
use_statement -> visibility? use path
projection_all_path -> path . *
path_as_bind -> path as id
attribute -> ^expr
~~~

读取 build.mode 使用普通投影，`matches` 使用已有模式匹配表达式，不增加工程配置专用运算符。

## 正式定义

### 六个包

| 包 | 职责 |
|---|---|
| builtin | 最基础的语言内容 |
| meta | comptime 基础设施与编译器暴露的 comptime 接口 |
| std | 跨平台系统编程基础库 |
| build | 当前工程、构建信息及工程配置接口 |
| attrs | 语言提供的属性构造接口 |
| verification | 程序验证所需的对象和操作，例如 Set、Map |

str 归 builtin，String 归 std，内存模块使用 mem 命名。Flurry 不提供 Option/Result 库枚举；可选类型和错误结果遵循各自的语言语义。

所属包不替代接口自己的阶段与效果要求。meta 的操作仍需满足相应 comptime 契约；verification 中的程序状态规格也不因此成为 PTC 的类型命题。

### Prelude

builtin、meta、build、attrs、verification 各自的 prelude 默认导入，作为包根的普通通配来源参与查找；每个提供者自身不隐式导入自己的 prelude。它们不具有高于本地声明的特权，也不自动成为当前包的公开接口。同一原始声明经多个 prelude 导入时按身份去重。

std.prelude 不默认导入。需要时在包根写 use std.prelude.*；子模块通过普通父级查找使用这些绑定。若两个 prelude 路径提供相同原始声明，按身份去重；真正不同的同名来源仍按通配歧义处理。

哪些名称由某个 prelude 提供，取决于该 prelude 的成员。不能仅凭一个名字常见，就跳过导入规则。

math 属于 builtin，通过 builtin.prelude 导出 math 模块入口；常用路径为
`math.sqrt(...)`，也可以显式使用 `builtin.math.sqrt(...)`。它不是第七个
内置包。math 同时提供 Eq、Ord、Order 等比较协议；这些协议经其他入口
再导出时保留原始声明身份。math 中的数值 intrinsic 函数是 LLVM intrinsic
的泛型包装，仍按具体操作的类型与目标契约检查，不因默认可见而改变数值语义。

### 属性名称

`^expr` 中的 expr 与其他表达式采用相同的名称解析。属性位置不额外优先搜索 attrs；可以使用完整路径，也可以先用普通 use 取得别名。

名称选定后，再按[属性系统](../comptime/attribute.md)检查编译时求值、Object 结果及目标契约。一个同名局部值不能因为当前位置需要属性就被忽略。

### Build 上下文

build 描述定义所属源码的具体构建实例。依赖包自己的定义读取自己的构建输入，不在被调用、内联或作为模板使用后改为读取消费者的一份全局可变上下文。

build 中的内容默认指 host pipeline。投影声明不会使 build.target 隐式改成设备目标；需要其他管线的配置时，可以通过 build.package_metadata 查询相应包的配置属性，按该包约定的类型读取。见[投影声明与类型接口](../pipeline-projection/declarations-and-interfaces.md)。

需要消费者提供某个选择时，可以显式传参。读取的配置应作为语义依赖保存；构建配置变化需要重新检查依赖它的结果，但不自动为每个声明创造新的名义类型身份。

宿主执行环境与目标产物环境分别记录。构建工具在某台机器上执行，不代表所有目标属性都应取这台机器的值。

因此，host pipeline 的目标仍可能与运行编译器的机器不同。默认配置的所属管线与编译器的执行位置不能混为一项。

## 常见示例

### 显式导入标准库 prelude

~~~flurry
use std.prelude.*;

mod greeting {
    fn show() {
        println("hello");
    }
}

fn main() {
    greeting.show();
}
~~~

输出 `hello`。println 的可用性来自包根导入及父级查找。

### 属性接口可以取普通别名

~~~flurry
use attrs.derivation.default as default_strategy;

trait Code {
    fn code(*self) -> u32;
}

struct Marker {}

^default_strategy
derive fn constant_code for Code {
    fn code(*self) -> u32 {
        7
    }
}

derive Code for Marker;
~~~

default_strategy 指向已有的属性构造接口。选择 deriver 的规则仍由[派生](../traits/derivation.md)定义，use 只缩短名字。

### 按本次构建选择常量

~~~flurry
const TRACE_ENABLED: bool = build.mode matches .debug;

fn trace_enabled() -> bool {
    TRACE_ENABLED
}
~~~

TRACE_ENABLED 在定义所属构建实例中确定。调用者没有通过调用 trace_enabled 改变这份配置。

### 显式接收调用者的选择

~~~flurry
fn tracing_allowed(comptime requested: bool) -> bool {
    requested
}

fn local_choice() -> bool {
    tracing_allowed(build.mode matches .debug)
}
~~~

这里 requested 是明确交付的 comptime 输入。将 tracing_allowed 放在依赖包中，也不会把实参来源变成隐式的全局状态。

### 读取目标库自己的配置

~~~flurry
use cuda;

comptime {
    const features = build.package_metadata("cuda").features;
    const arch: ?cuda.config.TargetArch = features.get(.target_arch, cuda.config.TargetArch);
}
~~~

这个查询读取 CUDA 包提供的 target_arch 配置，保留其可选性。缺失时的处理由该库规定，不自动回退到 host pipeline 的架构。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 有由语言版本及 crate 设置决定的 prelude；Flurry 区分五个默认内置包 prelude 与显式的 std.prelude，导入后采用普通通配竞争规则。[Rust prelude](https://doc.rust-lang.org/reference/names/preludes.html)

Zig 可导入 std 与构建相关的 builtin 信息；Flurry 把基础语言内容、编译时接口和构建信息分别放在 builtin、meta、build 的职责下。[Zig import](https://ziglang.org/documentation/master/#import)

C/C++ 的标准头文件和 C++ 库模块提供库声明，预处理定义可承载构建选择。Flurry 的 build 提供当前构建实例的信息，配置计算使用 comptime 表达式。[C 头文件](https://gcc.gnu.org/onlinedocs/cpp/Header-Files.html)、[C++ 条件包含](https://eel.is/c++draft/cpp.cond)

## 运行时实现设计引导

已求出的构建常量按对应类型的跨阶段规则进入运行时代码。不能为每次读取生成对构建工具的动态查询。

若不同配置选择不同代码，生成结果应与本次编译输入一致；优化不会把某次构建的常量复用到配置不同的另一实例。公开函数返回该常量时，按普通函数与返回值规则处理。

## 编译器实现引导

建立包根时，按已登记的内置包身份记录五个默认 prelude 来源，分别排除提供者自身；std 不参与默认注入。其绑定使用普通通配优先级与身份去重，不建立一套绕过解析器的内建短名快表。

属性表达式直接复用普通名称解析结果；属性检查只处理阶段、Object 结果和附着目标。这样属性位置与普通位置不会选择不同的同名声明。

build 查询以定义所属构建实例为环境参数。缓存记录实际读取的信息、依赖结果以及需要的宿主／目标区分；内联和模板接入保留该来源。没有显式读取或真实语义依赖的信息，不必机械地复制进每个缓存键。

如果某项信息的计算反过来依赖正在等待它的构建任务，交给构建依赖诊断。不能用宿主默认值、其他成员的值或上次构建的值填补未解输入。
