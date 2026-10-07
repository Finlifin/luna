# 宿主后端与产物交付

## Intro

Pipeline 回调把待编译的 IR 交给目标后端。Comptime 代码可以只负责连接接口，实际编译器则作为宿主程序执行。

回调返回 void，仍然可以让当前编译任务交付代码、入口和诊断。函数返回值与编译任务的产物通道各有职责。

## 语法

```text
attribute -> ^expr
function -> pure? comptime? inline? (unsafe | spec | verified | atomic)? fn id(parameter*) (-> result)? clauses? (block | (= expr))?
error_elimination -> expr! { error_arm* }
```

投影属性及后端调用使用现有属性、函数和错误处理语法。meta.compilation.IIR.Func 是回调接口中的 IR handle 类型。

## 正式定义

### 回调与执行位置

带相应编译属性的 pipeline 回调接收待处理的 IR handle。它按协议读取输入，调用目标后端，并通过所属任务交付编译结果。

后端可以由 runtime Flurry 编写并在宿主环境执行。这里的 runtime 指后端程序的执行方式；后端生成的代码可以面向不同的目标。

使用 meta.runtime.invoke 连接 comptime 胶水与宿主实现，并不把设备程序安排到宿主执行，也不把任意 runtime 函数自动变成 comptime 函数。调用仍须符合这个桥接接口的输入与任务契约。

### IR handle

IR handle 引用编译器管理的 IR，避免先把完整 IR 转成 comptime 数据，再复制成宿主后端的数据。访问应遵守所属编译任务的有效期和访问权限。

后端不能在 handle 失效后继续读取它。需要跨任务保存的信息，须通过受支持的持久表示交付；原始 handle 不充当跨进程、跨编译会话的稳定身份。

### 产物与类型接口

回调的 void 结果只表示这次函数调用不返回普通值。目标代码、入口关联、依赖和诊断归属于当前编译任务。

任务成功交付的入口必须兑现[先行提供的类型接口](declarations-and-interfaces.md)。后端不能在代码生成后改变宿主已经检查过的参数关系。

消费者只能使用成功交付的产物。失败的任务不能只因留下某个临时文件就被视为完成；缓存命中也必须能恢复实际所需的代码与关联信息。

### 错误与依赖

后端错误通过诊断通道报告。诊断应保留投影声明或相关操作的来源，并阻止失败产物进入后续链接或启动准备。

后端实际使用的输入构成任务依赖，包括 IR、投影属性、相关包配置及影响输出的工具链信息。宿主执行位置不能被当作“这些输入没有依赖”的理由。

该协议不承诺撤销宿主程序的一切外部副作用。任务实现负责隔离临时输出，并只发布完整、成功的结果。

## 常见示例

### Comptime 胶水调用宿主后端

下面给出 cuda.pipelines.global 的回调形态。cuda_compile 是实现目标后端的包；compile_kernel 是该包提供的编译入口。

```flurry
use cuda_compile;

^attrs.compilation.pipeline
pub pure comptime fn global(ir: meta.compilation.IIR.Func) -> void {
    meta.runtime.invoke(
        cuda_compile.compile_kernel,
        meta.compilation.IIR.access(ir),
    )! {
        .diagnostic(diag) => meta.compilation.emit_diagnostic(diag),
    }
}
```

diagnostic 是此桥接接口提供的诊断分支。回调传递 IR 访问接口，没有在 Flurry 源程序中递归复制所有 IR 节点。pure 修饰按[编译时纯性](../comptime/purity-and-normal-form.md)的契约使用；它不免除后端申报真实编译依赖和正确恢复任务产物的责任。

### 从包配置取得设备目标

```flurry
use cuda;

comptime {
    const features = build.package_metadata("cuda").features;
    const arch: ?cuda.config.TargetArch = features.get(.target_arch, cuda.config.TargetArch);
}
```

arch 保留配置查询返回的可选性。缺失时采用哪个目标、是否要求显式指定，由 CUDA 包的配置契约决定；语言不把它偷偷替换为 build.target.arch。实际使用的配置必须进入后端任务的依赖关系。

### 从声明到可用入口

```text
投影声明
    ├─ 签名、属性、必要 comptime 输入 → 可供宿主检查的类型接口
    └─ 目标实例及其依赖 → IR handle → pipeline 回调
                                           ↓
                                      宿主后端程序
                                           ↓
                                 代码产物、入口关联、诊断
                                           ↓
                                   库提供的加载／启动接口
```

类型接口可先用于检查 launch 的参数；实际使用入口仍需要成功交付的目标产物。这两条依赖路径不能用一个“回调返回了 void”合并成已完成。

| 构建情形 | 所需处理 |
|---|---|
| 首次构建 | 编译并交付本任务的完整产物 |
| 输入一致且缓存产物完整 | 恢复代码和入口关联后继续 |
| 只剩缓存记录，代码文件已不存在 | 取回可验证产物或重新生成 |
| 使用的设备目标配置改变 | 重新验证相应依赖并生成适用产物 |
| 后端失败并留下临时文件 | 报告失败，不发布临时文件为成功产物 |

## 与 Rust / Zig / C / C++ 的关键差异

这些语言的普通函数语法没有与此投影协议直接对应的构造。外部编译器、构建脚本和链接步骤可以承担异构产物的生成；CUDA C++ 则使用 CUDA 扩展声明 kernel，并通过启动接口使用设备入口。[CUDA 编程模型](https://docs.nvidia.com/cuda/cuda-programming-guide/01-introduction/programming-model.html)

LLVM、MLIR 提供编译器 pass 和 IR 处理接口，供后端实现组织转换。它们属于编译器基础设施；Flurry 本章约定的是源声明如何接入后端，以及外围程序如何取得稳定的类型接口。[LLVM 语言参考](https://llvm.org/docs/LangRef.html)、[MLIR pass 管理](https://mlir.llvm.org/docs/PassManagement/)

## 运行时实现设计引导

宿主后端接收受控 IR 访问入口，执行目标验证与生成，并把诊断和产物交付给任务管理者。任务结束前完成需要访问该 handle 的工作，避免后端保留悬空引用。

目标程序运行时，由库根据产物契约加载入口、准备参数并启动工作。加载失败、执行失败和编译失败处于不同阶段，应由相应接口报告；不能把一次编译成功解释为后续设备运行必然成功。

同一产物被多个调用者使用时，加载器如何共享已加载对象、管理设备上下文和同步，是该 runtime 库的契约。语言的 pipeline 回调不额外规定 CUDA 库的这些内部结构。

## 编译器实现引导

### 任务状态机

```text
Prepared → Running → Validated → Published
                └──────────────→ Failed
Cached → VerifyDependenciesAndArtifacts → Published
                          └─ miss ──────→ Prepared
```

1. 准备入口、目标实例、可见类型接口和相关配置，建立本次任务身份。
2. 生成有效期受控的 IR handle；桥接只在授权的任务范围内提供访问。
3. 运行回调，记录实际输入依赖，收集产物和诊断。
4. 检查后端结果是否完整，以及入口是否兑现已提供的接口。
5. 成功时发布产物与关联信息，唤醒依赖任务；失败时发布失败状态和诊断。
6. 释放临时资源，使后续访问不能继续使用失效 handle。

状态名是实现伪代码，不是公开的产物注册 API。并发后端若受支持，也必须在发布时满足相同的完整性条件。

### 缓存与恢复

缓存身份至少应区分会影响输出的 IR 实例、投影协议与后端版本、实际读取的构建配置以及目标约束。可通过依赖记录和指纹组织这些信息，不要求把所有工程配置无差别塞入每个键。

缓存记录把稳定产物与临时访问对象分开。命中时验证依赖仍成立，并确认产物存在且与记录匹配；缺失、损坏或不兼容时重新取得或生成产物。不能把旧进程内的 handle、指针或任务编号恢复成可用 IR。

多个消费者等待同一个任务时，共享成功结果或失败诊断；不要分别执行一个具有相同身份的生成任务再竞争覆盖输出。若接口查询、实例准备和代码生成形成环，报告依赖路径，不用空产物打破环。

### 成本与失败模式

Handle 避免的是跨边界的全量对象转换，并不保证访问 IR 或生成代码为常数时间。统计桥接次数、访问量、目标检查时间、生成时间和产物恢复成本，分别定位瓶颈。

必须覆盖后端异常退出、诊断产生后仍留下部分输出、缓存记录与文件不一致、目标配置变化和任务取消。所有失败路径都不得将未完成入口交付为成功结果。
