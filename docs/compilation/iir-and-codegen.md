# IIR 与代码交付

## Intro

一个实例的类型已经确定时，它仍可以包含循环、闭包、模式匹配和效应处理。类型准备完成，与把程序拆成底层指令，是两个不同的边界。

IIR 保存这样的高级具体实例，供目标检查与后续转换使用。进入 IIR 不要求先形成 SSA。

## 语法

```text
while_statement -> while label? expr block
lambda -> |anchored_parameter*| (-> expr)? expr
effect_elimination -> expr# { case_arm* }
error_elimination -> expr! { error_arm* }
```

这些源构造可在高级实例表示中保留相应信息。内部 IR 的存储格式不属于源语言语法。

## 正式定义

### 类型已完成的含义

IIR 中的类型已经完成所需 comptime 计算。类型引用的模板洞均有合法替换，runtime 依赖由实例的参数、局部绑定或交付环境解释。

合法范式项保留其头与实参；保留它不表示还有一次函数体展开等待执行。相反，未完成的普通类型计算、未解决的必需推断变量或失效的依赖环境，不能以“高级 IR 可以开放”为由进入成功实例。

类型完成不要求删除全部 runtime 信息。值相关大小、操作环境和源程序的动态分支仍按各自语义处理。

### 高级结构与行为

IIR 可以保留结构化控制流、模式及绑定、闭包捕获、效应请求与 handler 关系，以及资源交付信息。后续转换必须保持这些结构的可观察行为。

进入高级实例不自动固定所有底层布局选择或指令选择。调用、表示与目标能力仍须在相应交付边界验证；最终代码生成不得接收尚未完成的必需义务。

### 错误与效应

错误交付使用错误结果的表示与传播协议。效应请求则保留操作身份、处理能力、返回或恢复关系。两者可以组合，但不能因为都能引起非局部控制变化就合并为一种 ABI。

Capability 通过 trait 协议配合 elaboration 接入。IIR 保存已确定的证据及环境关系，不为函数签名另造一个处理器列表。

### 目标投影

投影后端接收的 IIR 仍可能需要目标自己的高级转换。它必须检查实际依赖、intrinsic、表示及调用协议，并兑现先行提供的类型接口。

build 默认指 host pipeline，设备库显式读取自身包配置。后端不能偷偷改变前端已经求出的类型计算结果。

## 常见示例

### 循环可以保留为结构化操作

```flurry
fn even_by_loop(value: u32) -> bool {
    var remaining = value;
    var even = true;

    while remaining != 0 {
        even = not even;
        remaining -= 1;
    }
    even
}
```

IIR 已知 remaining 是 u32、even 是 bool，并保留循环条件、更新和结果交付。它无需在此时为每次更新创建 SSA 名字；若后续后端采用 SSA，再按控制流构造相应定义与汇合。

### 闭包的类型已知，环境仍在 runtime 创建

```flurry
use std.prelude.*;

fn show_shifted(value: i32, bias: i32) {
    let shift = |input: i32| input + bias;
    println("{}", shift(value));
}
```

编译器确定闭包的输入、结果和捕获关系，但 bias 的值仍由调用提供。IIR 保留捕获的绑定与责任，后续可消除环境分配或内联主体，前提是求值与清理语义成立。

### 效应请求保留处理关系

```flurry
use std.prelude.*;

effect read_number() -> u32;

fn twice_input() -> #[read_number] u32 {
    let number = read_number()#;
    number * 2u32
}

test {
    let result = twice_input()# { read_number() => 7u32, }
    println("{}", result);
}
```

输出 14。IIR 可以保留操作请求、直接分支与请求结果的关联，再由所选控制后端转换；类型计算完成并不要求提前把所有 handler 展平。

### 错误是结果交付

```flurry
use std.prelude.*;

enum InputErr { Missing, }

fn load_number(present: bool) -> !InputErr u32 {
    if present {
        7u32
    } else {
        error InputErr.Missing
    }
}

test {
    let result = load_number(false)! { .Missing => 0u32, }
    println("{}", result);
}
```

输出 0。这条路径处理错误结果，不捕获 load_number 的 continuation。它与前面的 read_number 请求拥有不同的交付协议。

## 与 Rust / Zig / C / C++ 的关键差异

Rust MIR 使用控制流图表达分析所需的中层操作，GCC GENERIC 则提供较高层的表示；LLVM IR 使用 SSA。它们是实际的编译器表示对照，但 Flurry IIR 的契约不能由名字或层次类比替代。[Rust MIR](https://rustc-dev-guide.rust-lang.org/mir/index.html)、[GCC GENERIC](https://gcc.gnu.org/onlinedocs/gccint/GENERIC.html)、[LLVM IR](https://llvm.org/docs/LangRef.html)

C、C++、Rust 和 Zig 的编译器可以选择不同内部管线；源程序用户依赖的是语言的调用、资源和结果契约。Flurry 的不同错误／效应交付方式见[效应 ABI](../algebraic-effect-and-delimited-continuation/effect-abi.md)。

## 运行时实现设计引导

按实例的调用与环境契约传参，落实闭包捕获和清理责任，分别实现错误结果与效应控制协议。不能为了共用一种底层跳转而丢失返回目标、恢复次数或清理顺序。

编译时已经完成的类型计算不进入 runtime；明确的程序检查、合法值相关布局计算与 Splice 方法中的 runtime 工作保留。需要的依赖身份和实际取得方式必须在相应边界一致。

目标入口的加载、启动和完成由相应库接口负责。普通函数地址与另一个执行域的入口不能仅通过位转换混用。

## 编译器实现引导

位向量可以在 IIR 保留宽度明确的拼接、提取和位置更新，再提取有限映射进行目标优化；具体交付和候选核验见[BitVec 代数与程序合成](../program-synthesis/bitvec-algebra.md#编译器实现引导)。

### IIR 验证器

```text
verify_instance_iir(instance):
    check all type references under their binding environments
    reject pending required comptime type computations
    reject unresolved mandatory inference and failed evidence
    check local control targets, value roles and operation interfaces
    check dependency identities and environment-delivery relations
    preserve resource/specification information for its responsible passes
    retain origins for every diagnostic-relevant operation
```

验证器不要求指令序列已经是 SSA。程序验证层有自己的规格与义务；保留这些交付对象不等于让 PTC 证明内存命题。面向目标产物的最终门槛仍须完成该安全等级要求的检查。

### 高级操作的降码契约

| 转换 | 必须保持的关系 |
|---|---|
| 结构化控制流 → 控制流图 | 分支条件、词法跳转目标、循环和结果交付 |
| 模式 → 判别及解构 | 优先级、成功绑定、失败路径及副作用次序 |
| 闭包 → 代码与环境 | 捕获来源、取得次序、仿射状态和清理 |
| 资源操作 → 具体移动、复制及清理 | 已建立的责任与出口路径 |
| 错误处理 → 结果判别及传播 | 原始错误结果、前后 catch 次序 |
| 效应处理 → 所选控制 ABI | 请求身份、处理范围、返回／恢复和退出 |
| Intrinsic → 目标操作 | 显式输出类型及内存、控制、数值契约 |

这些是转换责任，不强制所有后端采用同一 pass 数量或固定的全局次序。有依赖的转换必须显式交付前提；融合 pass 时也要分别保持各项不变量。

### 与类型替换共同转换

重命名 runtime 绑定、移动代码或封闭环境时，同步变换类型引用中的 substitution。建立控制流汇合不能把来自不同依赖对象的值只按本体宽度合并；需要保留分支与环境的对应，或按接口拒绝。

需要 SSA 的后端在这一边界之后构造相应形式，并把旧绑定与新值的关系带入依赖描述。其他后端可以直接消费更高级结构；最终均须验证目标操作和表示。

### 交付检查与验收

发布产物前检查所选目标支持全部实际依赖、必需规格义务完成、入口兑现类型接口，并具备完整可恢复的产物。详细后端通道见[产物交付](../pipeline-projection/backends-and-artifacts.md)。

验收应同时包含循环、捕获闭包、runtime 依赖类型、错误、直接效应处理及投影入口；用缺失类型计算、悬空环境和不受支持 intrinsic 作为拒绝例。记录每项转换使用的契约，而不是只比较是否生成了某段汇编。
