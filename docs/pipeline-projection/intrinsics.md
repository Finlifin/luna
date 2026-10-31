# Intrinsic 与输出类型

## Intro

某些操作需要由目标管线解释，例如设备指令、向量装载或屏障。Intrinsic 在代码中保留这样的操作，同时让外围表达式知道它是否交付结果。

有结果时，紧接调用的 as 指定结果类型。没有 as，调用按 void 处理。

## 语法

```text
intrinsic_application -> expr'intrinsic(argument*)
typed_intrinsic_application -> intrinsic_application.as(expr)
```

标签通过带 `^intrinsic_enum` 属性的枚举定义。枚举、take view、调用与 as 都复用[全局表面语法](../grammar/syntax)，不引入指令字符串语法。

## 正式定义

### 标签与开放输入

Intrinsic 的枚举变体标识一种扩展 IIR 操作。具体 pipeline 决定接受哪些标签，以及每个标签的操作数、结果、目标能力和执行行为。

枚举定义提供操作身份，不自行实现该操作。两个不同枚举中的同名变体也不能仅凭拼写被当成同一条指令。

Intrinsic 的操作数接口开放。通用层不代替目标操作检查实参数目、操作数匹配或该操作对内存与可达依赖的使用，因此调用要求 unsafe。

普通实参表达式仍须能合法形成输入：名称必须存在，所需编译时计算必须完成，runtime 表达式仍保留其求值步骤。开放接口不允许从不存在的变量或未经接入的代码描述中凭空取得值。

### 显式输出

| 写法 | 交给 pipeline 的输出要求 | 外围表达式的类型 |
|---|---|---|
| 调用后紧接 `.as(T)` | 交付 T | T |
| 没有该 as | 不交付结果 | void |

紧接 intrinsic 调用的 as 是该操作的输出类型接口。它不要求先构造一个 void 值，再把 void 转成 T。

左侧变量注解、外围函数返回类型或后续使用不会代替这个显式指定。需要向量中间结果时，每个产生结果的 intrinsic 都要在自身调用处给出相应类型。

请求的类型应满足其出现位置的类型与阶段要求。Pipeline 进一步检查该操作能否交付请求的类型及表示；一个 as 不会创造后端不支持的表示。

如果操作交付多个结果，可以使用它明确支持的聚合类型。聚合形式与每个分量的含义属于操作协议，不能由通用层任意拼接。

### 输出类型不描述行为

void 操作可以写内存、同步或改变可观察状态。结果未被使用的操作也可以有副作用。通用优化不能仅凭返回 void、调用位于 unsafe 或结果没有使用，就删除、复制或重排操作。

结果类型只提供外围类型接口。某项操作对指针目标的访问、产生值的合法性以及其他目标约束，仍由该操作的协议和使用者责任说明；这些要求不会被 as 消除。

### 拒绝与责任

未受支持的操作标签、与操作不符的输出类型、违反目标静态约束的调用，由 pipeline 报告编译错误。unsafe 不要求后端为这些情况随意生成代码。

只有 runtime 才能决定的前提，按具体 unsafe 接口的契约由使用者保证。静态接受不构成对任意地址、数据或执行环境的无条件保证。

## 常见示例

### 一个有结果的操作与一个无结果的操作

Inst 是本节的教学用指令族。以下例子以目标后端支持三输入 f32 融合乘加和编号屏障为前提。

```flurry
use std.prelude.*;

^intrinsic_enum
enum Inst {
    fma,
    barrier,
}

unsafe fn fused_multiply_add(a: f32, b: f32, c: f32) -> f32 {
    Inst.fma'intrinsic(a, b, c).as(f32)
}

unsafe fn wait_group() {
    Inst.barrier'intrinsic(0u16);
}
```

第一个主体向 pipeline 请求 f32；第二个不请求输出。屏障的作用不能由它的 void 结果删除。

在支持该操作的目标下，可以直接使用结果：

```flurry
test {
    let result = unsafe {
        fused_multiply_add(3.0, 4.0, 5.0)
    }
    println("{}", result);
}
```

数值结果为 17。测试只验证这组输入的数值行为；融合乘加的舍入契约由该操作定义。

### 外围类型不能补写 as

下面是拒绝例：

```flurry
unsafe fn missing_result_type(a: f32, b: f32, c: f32) -> f32 {
    let result: f32 = Inst.fma'intrinsic(a, b, c);
    result
}
```

调用没有输出类型指定，因此得到 void，不能用它交付所要求的 f32。正确写法是在调用处指定：

```flurry
unsafe fn explicit_result_type(a: f32, b: f32, c: f32) -> f32 {
    let result = Inst.fma'intrinsic(a, b, c).as(f32);
    result
}
```

### 一个结果继续成为另一操作的输入

```flurry
unsafe fn two_steps(a: f32, b: f32, c: f32, d: f32) -> f32 {
    let first = Inst.fma'intrinsic(a, b, c).as(f32);
    Inst.fma'intrinsic(first, d, c).as(f32)
}
```

两个结果在各自操作处定型，外围程序无需等待后端才能知道 first 的类型。后端仍须检查两条操作及其数据依赖。

### 开放输入不代表错误输入也被接受

本节 fma 的目标契约要求三个 f32 操作数。以下两个独立示例均应由目标检查拒绝：

```flurry
unsafe fn missing_operand(a: f32, b: f32) -> f32 {
    Inst.fma'intrinsic(a, b).as(f32)
}
```

```flurry
unsafe fn wrong_output(a: f32, b: f32, c: f32) -> u32 {
    Inst.fma'intrinsic(a, b, c).as(u32)
}
```

第二个例子请求的是 u32 输出，而不是先取得 f32 再进行数值转换。需要后续转换时，先请求该操作支持的输出，再对已经取得的值使用普通转换。

### 操作协议可以交付聚合结果

下面的教学操作将一个 u64 拆成低、高两个 u32，并规定结果为该顺序的元组。

```flurry
^intrinsic_enum
enum WordInst {
    split_word,
}

unsafe fn split_word(value: u64) -> Tuple<u32, u32> {
    WordInst.split_word'intrinsic(value).as(Tuple<u32, u32>)
}
```

这说明结果接口可以是已经存在的聚合类型，不规定每个后端都支持该操作，也不为“多结果指令”增加另一套调用语法。

## 与 Rust / Zig / C / C++ 的关键差异

Rust asm 用输出操作数及其寄存器约束描述结果；Zig asm 也在汇编表达式中描述输出。Flurry 的 intrinsic 接入以目标定义的操作标签和显式输出类型为接口。[Rust asm](https://doc.rust-lang.org/reference/inline-assembly.html#operand-type)、[Zig asm](https://ziglang.org/documentation/master/#Assembly)

C/C++ 的 GNU extended asm 使用输出、输入和 clobber 列表；目标 intrinsic 函数还可以有预先声明的签名。Flurry 的开放输入不提供这样的统一函数签名检查，操作合法性由所选 pipeline 负责。[GNU extended asm](https://gcc.gnu.org/onlinedocs/gcc/Extended-Asm.html)

## 运行时实现设计引导

实参求值先产生本次操作需要的输入。后端按照操作契约将其放入目标表示，并把输出交付为请求的类型。SSA 临时值、寄存器或临时存储均是表示选择；它们必须保留源程序的求值次数、数据依赖和资源关系。

void 操作也必须保留自己的执行契约。对屏障而言，仅保留一个“返回 void”的占位节点不足以实现同步；对存储操作而言，删除结果不意味着可以删除写入。

## 编译器实现引导

### 输出类型接入

```text
elaborate_intrinsic(expression):
    if expression directly supplies as(T) to an intrinsic call:
        output = check_type_expression(T)
        call = that intrinsic call
    else:
        output = void
        call = expression

    tag = resolve_intrinsic_variant(call)
    inputs = prepare_input_expressions_in_source_order(call)
    require_unsafe_permission(call)
    return Intrinsic(tag, inputs, output, source_origin)
```

识别此组合时保留普通分组括号的表达式范围。只有直接提供给该调用的输出指定被写入节点；经变量读取或普通函数调用产生的外围期望不能事后改写它。

内部 Intrinsic 节点至少保存标签身份、实际输入、输出类型和来源。它不伪装成具有通用固定形参表的普通函数调用。

### 目标验证

对每个实际需要的节点：

1. 按标签身份定位目标操作描述；没有支持项即报告错误。
2. 验证操作数数量、类别、立即数限制、表示与目标能力。
3. 验证输出要求，包括 void 是否适用、聚合分量及其交付顺序。
4. 检查操作的控制、内存和其他目标约束。
5. 为通过的节点生成已知语义的目标 IR 或代码。

节点通过验证前，通用优化保留未知行为；验证失败不得生成一个任意常量或空操作作为成功替代。

### 成本与诊断

对固定节点集合，接入成本按实参表达式规模计，标签查找可索引化；每种操作的验证成本由其规则决定。类型求值与后端求解不应被记成常数时间。

诊断至少区分缺少输出指定导致的外围类型不符、未知标签、非法输入和不支持的输出表示。指向原调用及其 as，不能只报告内部节点编号。
