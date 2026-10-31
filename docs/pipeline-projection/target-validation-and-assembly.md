# 目标检查与汇编封装

## Intro

把入口交给某条 pipeline，还需要把它实际使用的代码交给同一个目标检查。入口调用一个普通 helper，不要求给 helper 重复添加投影属性；但 helper 使用的操作必须能在该目标执行。

内联汇编也走 intrinsic 接口。包装函数负责给调用者一个清楚的签名和契约，后端负责检查并翻译目标操作。

## 语法

```text
intrinsic_application -> expr'intrinsic(argument*)
typed_intrinsic_application -> intrinsic_application.as(expr)
inline_if -> inline if expr block (else block)?
```

函数、unsafe、comptime 参数及属性复用[全局语法](../grammar/syntax)。这里没有独立的 asm 字符串产生式。

## 正式定义

### 实际依赖闭包

Pipeline 检查投影入口实际需要的实例及其依赖，包括普通 helper、使用的类型表示和 intrinsic。依赖是否支持，由目标能力和相应接口决定，不能只检查入口本身。

沿调用关系使用普通 helper，不会改变这个声明向其他使用者提供的类型接口，也不要求所有 helper 都添加相同属性。另一个明确投影的对象仍须遵守它自己的使用接口，不能因此被当作普通函数调用。

编译时已排除的代码不属于最终目标实例的执行依赖，但它在先前阶段承担的检查仍按原规则完成。runtime 条件分支的两个可能出口都需要有合法实现，不能假设某个分支在测试中未执行便忽略它。

递归、函数指针、动态分派和运行时服务均可能受到目标限制。若调用目标不能静态枚举，后端须有能够支持该间接调用的目标协议；否则报告不支持，不能将宿主函数地址直接解释为设备入口。

### 配置与表示

build 默认描述 host pipeline。宿主汇编封装可以据此读取架构与特性；设备库通过包配置读取自己的目标。读取规则见[投影声明与类型接口](declarations-and-interfaces.md)。

编译时检查目标配置不能代替 runtime 部署条件。例如按 AVX2 目标构建的程序，实际执行机器仍须满足该程序的目标要求；读取 build 不会生成运行时 CPU 探测。

静态类型相同也不保证每个目标都接受其表示。后端应检查实参和结果的具体表示、调用约定，以及跨入口边界的数据交付要求。

### 优化边界

尚未获得行为契约的 intrinsic 按未知操作处理。除结果类型外，不能自行假定其无副作用、不会失败、不会访问指针目标，或可以与其他操作交换顺序。

后端识别操作后，可以依据其实际控制、内存、同步和数值契约优化。unsafe 允许使用要求调用者负责的接口，并不赋予优化器额外的纯度证明。

包装中的 inline fn 表达内联意图；quote fn 按其接入规则确定展开，并保留普通实参一次求值。两者都不能取消目标验证，见[Inline 与 quote fn](../functions/inline-and-quote.md)。

## 常见示例

### 检查宿主架构并明确请求标量结果

以下使用 std.asm.x86 中的指令标签。

```flurry
use std.asm.x86.*;

unsafe fn fused_multiply_add(a: f32, b: f32, c: f32) -> f32 {
    inline if build.target.arch != .x86_64 {
        meta.compilation.err("fused_multiply_add requires an x86_64 target");
    }

    X86Instructions.fma'intrinsic(a, b, c).as(f32)
}
```

架构检查只排除非 x86_64 目标。该目标是否支持此 fma 操作及所需指令特性，仍由后端验证；这段检查没有证明全部 x86_64 处理器都能执行它。

### 向量 load、add、store 的完整数据路径

Vector 是显式交付的编译时类型参数。调用者须传入目标库实际支持的向量类型；本例不为标准库杜撰一个向量类型名。后端应拒绝不符合这些指令表示要求的 Vector。

```flurry
use std.asm.x86.*;

unsafe fn add_arrays_avx2(
    comptime Vector: Type,
    xs: *Array<f32, 8>,
    ys: *Array<f32, 8>,
    zs: *Array<f32, 8>,
) {
    inline if build.target.arch != .x86_64 or not (build.target.features.get(.avx2) matches true?) {
        meta.compilation.err("add_arrays_avx2 requires x86_64 with AVX2");
    }

    let left = X86Instructions._mm256_loadu_ps'intrinsic(xs).as(Vector);
    let right = X86Instructions._mm256_loadu_ps'intrinsic(ys).as(Vector);
    let sum = X86Instructions._mm256_add_ps'intrinsic(left, right).as(Vector);
    X86Instructions._mm256_storeu_ps'intrinsic(zs, sum);
}
```

三个产生值的操作分别给出 Vector；store 不交付结果，因而不接 as。输入须各指向可读的八个 f32，输出须指向可写的八个 f32，并满足相应操作的有效性要求。两次装载发生在存储之前；后端不能以 store 返回 void 为由删除它。

若把 Vector 换成 u32，函数的普通参数准备不等于这些向量操作已通过验证。目标操作检查还会核对所请求的表示。

### 普通 helper 随入口进入目标检查

```flurry
use cuda;

fn add_values(left: f32, right: f32) -> f32 {
    left + right
}

^cuda.global
unsafe fn add_kernel(left: *f32, right: *f32, output: *f32) {
    output.* = add_values(left.*, right.*);
}
```

目标检查包括 add_values 的实际实例。后端可以保留调用，也可以合法内联；接受此程序不以某次优化恰好把调用消掉为前提。如果 helper 改为调用目标不支持的运行时服务，诊断应同时说明该服务和从 add_kernel 到它的依赖路径。

### 用 quote fn 确定接入包装主体

```flurry
quote fn fused_multiply_add_expanded(a: f32, b: f32, c: f32) -> f32 {
    unsafe {
        X86Instructions.fma'intrinsic(a, b, c).as(f32)
    }
}
```

这个包装将 intrinsic 表达式接入调用位置，普通实参先按调用规则各求值一次。包装内部承担使用该操作的 unsafe 责任；后端仍检查目标及操作支持。直接把同一包装声明为 inline fn，只提供内联意图，不能据此要求某次优化必定展开。

### 拒绝条件速查

| 情形 | 结果 |
|---|---|
| 标签不受当前 pipeline 支持 | 目标检查拒绝 |
| load 没有 as，却将其结果用于向量 add | 不能取得所需的向量结果 |
| as 请求了操作不支持的类型表示 | 目标检查拒绝 |
| runtime 分支中包含不支持的操作 | 不能因另一分支可执行而接受整个实例 |
| 间接调用缺少目标可用的调用协议 | 目标检查拒绝 |
| 已通过检查的 unsafe 操作收到不满足契约的地址 | 按该操作的 runtime 契约处理 |

## 与 Rust / Zig / C / C++ 的关键差异

Rust 和 Zig 的 inline assembly 接口显式描述寄存器操作数及其他汇编约束；Flurry 通过目标指令标签形成操作，封装函数可以使用普通参数与编译时目标判断。[Rust inline assembly](https://doc.rust-lang.org/reference/inline-assembly.html)、[Zig assembly](https://ziglang.org/documentation/master/#Assembly)

GNU C/C++ extended asm 用约束与 clobber 告知编译器汇编的输入、输出和破坏范围。Flurry 的目标后端须取得 intrinsic 的行为契约后才能据此优化；返回类型本身不承担 clobber 的职责。[GNU extended asm](https://gcc.gnu.org/onlinedocs/gcc/Extended-Asm.html)

## 运行时实现设计引导

装载、算术和存储必须实现各自的数据依赖。寄存器分配可以复用死值的存储，但不能提前覆盖仍要读取的输入。

跨执行域的工作通过库的启动与参数交付接口发生。普通调用、设备内部调用和宿主启动设备工作采用各自的调用协议；只改变地址的位表示不能完成这种转换。

对于具有同步要求的操作，保持同步范围及参与者条件。后端提供的优化说明必须足以判断某次移动、复制或删除是否合法，不能只保留“这是某条机器指令”的名称。

## 编译器实现引导

### 建立闭包

```text
pending = [projected_entry_instance]
visited = {}
while pending is not empty:
    item = pop(pending)
    if item in visited:
        continue
    add item to visited
    check_target_representation_and_calling_contract(item)
    for dependency in required_dependencies(item):
        if dependency is an ordinary concrete instance:
            push dependency to pending
        else:
            check_target_protocol(dependency)
    check_intrinsics(item)
```

以上是内部算法，不是 meta 库 API。实例身份须包含影响结果的替换及目标输入；不能按源声明名字去重不同实例。

已形成的有限依赖图可在 O(V + E) 次图访问内遍历。实例化和目标验证另计；不断产生新实例的递归需要单独的终止或资源限额诊断，不能由 visited 自动保证终止。

### 保留证据再优化

在未知 intrinsic 上保留保守的行为摘要。目标识别成功后，替换成具有明确语义的目标节点，或为原节点附上足够的优化契约。任何变换都须检查它实际依赖的性质：无内存写不等于无控制效果，无输出不等于可删除。

对不支持的依赖，记录入口、调用边和失败操作的源码来源。共享 helper 可只诊断一次主体错误，但每个受影响入口应能追溯到该错误。不能把错误伪装成“优化未能内联”。
