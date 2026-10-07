# 投影声明与类型接口

## Intro

声明的形状与声明的使用接口可以分开。一个 kernel 需要参数和主体，因此适合用 fn 写出来；宿主使用它时，需要的却是目标入口和启动配置。

投影属性连接这两端：使用者先获得可以检查的类型接口，后端再交付符合接口的代码。

## 语法

```text
attributed_definition -> ^expr definition
attribute -> ^expr
extended_application -> expr { (property | expr)* }
property -> id: expr
```

参数、子句和主体沿用[函数语法](../functions/calls-and-returns.md)。`^cuda.global` 使用普通名称解析；该属性提供的投影配置由相应协议解释。

## 正式定义

### 声明接口与主体

投影协议依据声明的签名、属性和必要的编译时输入提供公开类型接口。接口可以保留正常的类型查询依赖，但不要求完整的目标代码生成先完成。

后端必须交付与该接口一致的入口、参数表示和结果协议。不能让同一次声明先以一种接口通过宿主检查，再根据生成结果将其替换为另一种接口。

源主体中的参数说明目标计算的输入。它们不自动成为宿主普通调用的 ABI。对于 CUDA kernel，直接调用该声明不能替代启动接口；取到一个入口引用也不自动取得宿主函数指针的调用资格。

普通函数包装、加载、启动和等待由库定义。某个投影协议可以提供普通函数接口，但此能力必须由该协议给出，不能从 fn 的表面形状推断。

### Kernel 与实参的类型联系

CUDA 库可以用 Kernel 的参数描述检查启动实参。例如，约束 F 匹配 `cuda.Kernel<A, Any, any>`，同时令配置中的 args 具有类型 A，便把目标入口和实际参数收集结果联系起来。

这里保留的是 F 的实际类型信息。Any/any 出现在某些参数位置，不会使 F 变成可以任意调用的对象，也不取消 A 对 args 的约束。Kernel 的其他参数含义属于库接口。

拓展变参收集使用既有[参数 elaboration](../functions/parameter-elaboration.md)协议。输入的归属、求值次数和顺序遵循调用规则；协议不能为了检查 kernel 而重复计算普通实参。

### Host 配置与目标库配置

build 中的内容默认描述 host pipeline。这里的 host pipeline 是编译宿主程序的管线，不是“运行编译器的这台机器”的别名。

投影到 CUDA 等其他管线，不会使 `build.target` 隐式改为描述该设备。需要对应库的配置时，显式读取 `build.package_metadata`，再按库定义的类型取得 feature。

包元数据的读取仍受[构建实例规则](../code-organization/builtin-packages.md#build-上下文)约束。一个 helper 被调用或内联，不会因此改读调用者的一份动态全局配置。

已经由编译时表达式取得的数值按原来的语义使用。后端不能把一次已经完成的布局查询重新解释成另一个结果；设备端表示与已交付接口的关系必须通过目标合规检查。需要设备专属的布局或能力信息时，应从相应目标接口取得，不能把默认 build 查询当作设备查询。

### 失败边界

| 条件 | 结果 |
|---|---|
| 投影属性的值不符合所选协议 | 编译错误 |
| 宿主按普通函数调用不提供此接口的 kernel | 编译错误 |
| 启动实参与 kernel 参数接口不匹配 | 类型检查失败 |
| 必需的目标配置缺失或不受支持 | 相应库或后端报告编译诊断 |
| 后端产物不能兑现已公开接口 | 编译任务失败 |

可选配置查询得到 null 与某个具体配置值不同。是否允许缺省、缺省为何值，由库接口确定，不能由编译器悄悄选取当前机器的值补齐。

## 常见示例

### 一个普通 helper 被目标主体使用

```flurry
use cuda;
use cuda.prelude.*;

fn add_values(left: f32, right: f32) -> f32 {
    left + right
}

^cuda.global
unsafe fn add_kernel(left: *f32, right: *f32, output: *f32) {
    output.* = add_values(left.*, right.*);
}
```

add_values 不必重复标记为 kernel。它参与目标主体实际需要的依赖集合，是否能在该目标执行由 pipeline 检查。它若也用于宿主程序，宿主使用仍按普通函数接口进行。

### 用配置类型表达 kernel 与参数的联系

下面定义一个教学用启动配置，其字段采用普通 snake_case 命名。它说明类型关系，并不替换 CUDA 库自身的 LaunchConfig API。

```flurry
struct LaunchConfig
where F, A,
    requires F matches cuda.Kernel<A, Any, any>
{
    kernel: *F,
    grid_dim: Tuple<u32, u32, u32>,
    block_dim: Tuple<u32, u32, u32>,
    args: A,
    shared_mem_bytes: u32 = 0,
}

fn take_arguments(config: LaunchConfig<F, A>) -> A
where F, A,
    requires F matches cuda.Kernel<A, Any, any>
{
    config.args
}
```

kernel 与 args 共用类型约束中的 A。收集协议须产出满足该约束的配置；它不能接受任意元组后寄希望于设备执行时碰巧读懂。

### 显式读取 CUDA 包配置

```flurry
use cuda;

comptime {
    const features = build.package_metadata("cuda").features;
    const arch: ?cuda.config.TargetArch = features.get(.target_arch, cuda.config.TargetArch);
}
```

arch 是按 CUDA 库给出的配置类型读取的可选值。这里没有修改 build.target 的默认含义；读取失败的处理应由使用这份配置的接口决定。

### 普通宿主调用不能替代启动

接续第一个示例，下面是有意展示的拒绝例：

```flurry
unsafe fn invalid_host_call(left: *f32, right: *f32, output: *f32) {
    add_kernel(left, right, output);
}
```

unsafe 不会将设备入口变成宿主函数。正确的使用方式见[总览中的启动代码](index.md#常见示例)。

## 与 Rust / Zig / C / C++ 的关键差异

| 对照机制 | 差异 |
|---|---|
| CUDA C++ 的 kernel 与启动配置 | 它通过语言扩展表达入口和启动；Flurry 将投影属性与库的启动接口连接起来。[CUDA kernel](https://docs.nvidia.com/cuda/cuda-programming-guide/02-basics/writing-cuda-kernels.html) |
| C/C++ 使用 CUDA Driver API | 实际可以显式加载入口并启动；Flurry 的配置类型还可以把入口的参数描述和 args 的类型联系起来。[CUDA Driver API](https://docs.nvidia.com/cuda/cuda-driver-api/group__CUDA__EXEC.html) |
| Rust、Zig 的普通函数指针 | 它们描述相应函数调用接口；不能用这种对应物替代 Flurry 的设备入口协议。异构编程仍可通过设备编译工具和库完成。[Rust 函数指针](https://doc.rust-lang.org/reference/types/function-pointer.html)、[Zig 函数](https://ziglang.org/documentation/master/#Functions) |

## 运行时实现设计引导

启动库应遵循自己的接口契约完成以下步骤：

1. 取得属于正确模块、设备或运行环境的入口。
2. 按该入口的参数接口编码已经求值的实参；核对表示、对齐和目标地址的要求。
3. 交付网格、线程块及其他显式配置。
4. 提交工作，并按接口约定交付提交错误、工作状态或完成结果。
5. 在规定的完成边界之后结束仍被设备工作使用的资源责任。

这不是规定统一的 CUDA 包函数签名，而是入口和实参类型检查必须兑现的运行时关系。不同库可以采用同步调用或显式工作对象；它们各自的返回与清理不能混用。

## 编译器实现引导

### 属性接入

属性表达式先按普通名称规则解析并求值得到 Object。CUDA 属性将所选回调编码到 pipeline 接口使用的属性字段中；下面是与前文 kernel 属性对应的显式写法：

```flurry
^{ __flurry_pipeline_projection: cuda.pipelines.global }
unsafe fn add_kernel_explicit(left: *f32, right: *f32, output: *f32) {
    output.* = left.* + right.*;
}
```

编译器从已求值属性读取回调配置，保留属性和声明的来源，并检查回调符合接入协议。不能仅因属性表达式的最后一段名字是 global，就选择 CUDA 后端。

### 接口与生成的依赖分离

```text
projection_interface(declaration):
    attributes = checked_attributes(declaration)
    protocol = select_projection(attributes)
    signature = checked_signature(declaration)
    interface = protocol.interface(signature, attributes, required_inputs)
    check_interface_is_a_valid_type_description(interface)
    return interface

generate_projection(declaration):
    interface = projection_interface(declaration)
    body = instantiated_body_with_recorded_dependencies(declaration)
    result = invoke_pipeline(body, interface, explicit_configuration)
    check_delivery_matches(interface, result)
    return result
```

以上名称是内部伪代码，不是公开 meta API。接口查询需要的正常依赖不能被省略；代码生成也不能反向修改已被使用的接口。

### 配置来源

记录 build 查询所属的构建实例，以及实际读取的包元数据。host pipeline 配置与目标库的配置分别保存来源；不能因为某个函数有投影属性，就替换其所有 build 读取。

元数据或接口变化时，使实际依赖它的检查与产物失效。缓存键中的进程内 handle、源码路径相同或后端名称相同，都不能单独证明两次任务语义相同。

### 检查成本与失败

公开接口的处理成本取决于签名和实际读取的元数据；不应为了回答“这组启动实参是否匹配”无条件扫描全部生成代码。接口查询循环、配置计算失败和代码生成失败分别诊断，并保留投影属性、入口声明与调用位置的来源。
