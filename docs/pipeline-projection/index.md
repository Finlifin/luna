# Pipeline projection

## Intro

一段代码可以用 Flurry 描述，却交给另一个编译管线生成产物。例如，CUDA kernel 的参数和主体仍写在源码中，宿主程序则通过 CUDA 库启动它。

Pipeline projection 用属性指定这条编译路径。理解投影，首先要分清声明对使用者提供什么接口，以及主体最终在哪里执行。

## 语法

```text
attributed_definition -> ^expr definition
intrinsic_application -> expr'intrinsic(argument*)
typed_intrinsic_application -> intrinsic_application.as(expr)
```

这些形式分别复用[属性](../comptime/attribute.md)、take view、调用与 as 的表面语法。`intrinsic` 是 view 名；pipeline 的名称是普通表达式。

## 正式定义

| 对象 | 使用者可以依赖的内容 | 进一步处理的责任 |
|---|---|---|
| 投影声明 | 所选协议提供的类型与操作接口 | Pipeline 交付符合该接口的代码和产物 |
| 普通 helper | 自己的参数、结果和契约 | 所在目标管线验证其实际依赖是否可用 |
| Intrinsic 调用 | 显式 as 指定的结果类型；缺省为 void | Pipeline 解释标签、操作数和目标约束 |
| Host pipeline | build 默认描述的管线环境 | 普通宿主程序的编译与目标检查 |
| 其他 pipeline 的配置 | 显式查询包元数据取得的信息 | 对应库检查配置并驱动自己的后端 |

投影协议提供类型接口，不要求外围程序等待完整代码生成后才能进行类型检查。后端交付必须兑现已经确定的接口。

主体采用 fn 写法，并不自动承诺生成一个可按宿主函数 ABI 调用的入口。CUDA kernel 通过库的启动接口使用；目标调用限制和运行时同步规则由相应接口规定。

Intrinsic 的输入接口开放，其操作含义由 pipeline 定义。调用要求 unsafe；对普通实参表达式的检查、操作自身的目标检查以及外围程序的类型检查，各自保留边界。

按下面的路线阅读：

| 文档 | 主要问题 |
|---|---|
| [投影声明与类型接口](declarations-and-interfaces.md) | Kernel 如何声明、配置、传递和启动 |
| [Intrinsic 与输出类型](intrinsics.md) | 开放输入如何接回确定的结果类型 |
| [目标检查与汇编封装](target-validation-and-assembly.md) | Helper、指令、内联与优化需要满足什么条件 |
| [宿主后端与产物交付](backends-and-artifacts.md) | IIR handle 如何进入后端，产物怎样交付和复用 |

## 常见示例

下面使用 CUDA 库提供的投影属性。三个指针须满足该库对设备存储的要求；主体执行一次浮点加法。

```flurry
use cuda;
use cuda.prelude.*;

^cuda.global
unsafe fn add_kernel(left: *f32, right: *f32, output: *f32) {
    output.* = left.* + right.*;
}
```

宿主侧通过启动接口提交这个 kernel。这里沿用 CUDA 库接口的字段名 `gridDim`、`blockDim`；它们分别给出网格和线程块的维度。

```flurry
unsafe fn submit_add(left: *f32, right: *f32, output: *f32) -> !CudaErr void {
    cuda.launch_kernel {
        kernel: add_kernel,
        gridDim: (1u32, 1u32, 1u32),
        blockDim: (1u32, 1u32, 1u32),
        args: (left, right, output),
    }!;
}
```

只启动一个线程，使本例只有一次写入。调用者仍须遵守库的存储有效性和完成协议；提交函数返回不在本章中被解释成设备写入已经完成。错误通过普通 error 结果传播，见[错误处理](../algebraic-effect-and-delimited-continuation/error-handling.md)。

## 与 Rust / Zig / C / C++ 的关键差异

CUDA C++ 提供 kernel 声明和专门的启动配置语法；Flurry 用属性连接投影声明，用库的调用协议组织启动参数。[CUDA 编程模型](https://docs.nvidia.com/cuda/cuda-programming-guide/01-introduction/programming-model.html)

Rust、Zig、标准 C 和标准 C++ 中无对等的 Flurry 投影协议；工程中可用外部设备编译器、生成代码和运行时库组合宿主与设备产物。具体到指令封装，Rust asm、Zig asm 和 GNU extended asm 则有可直接对照的操作数与约束接口，见[汇编封装](target-validation-and-assembly.md#与-rust--zig--c--c-的关键差异)。

## 运行时实现设计引导

运行时消费编译产物和库接口，不重新执行投影属性或 comptime 回调。一次设备操作可能涉及入口解析、参数编码、工作提交和完成通知，这些步骤须与库公开的责任和同步契约对应。

代码地址、设备入口引用和已提交工作的状态分别管理。不能因为它们都可由一个机器字表示，就把其中任意一种作为另一种调用。

## 编译器实现引导

```text
声明及属性 ──→ 投影协议 ──→ 可检查的公开接口
                    │
                    └──→ 主体与实际依赖 ──→ IIR handle
                                                │
显式读取的包配置 ────────────────────────────────┤
                                                ↓
                                           目标后端
                                                ↓
                                  验证、代码与符号、产物交付
```

这个图规定数据责任，不要求所有工作线性执行。接口查询、主体检查和后端调用可以通过依赖图调度；每个可复用结果须记录实际输入。

在每个交界检查三个不变量：已经交付的类型接口没有被后端改写；目标程序中不存在未获支持的操作或伪造的宿主地址；成功的编译任务具备完整且可恢复的产物集合。
