# 代码组织

## Intro

把实现拆进文件后，程序仍需要一个明确的公共入口。Flurry 从源码目录建立模块，再由声明和导入决定每个名字指向什么、哪些使用者可以访问它。

包是定义归属与依赖的边界。Umbrella 把多个包组织成工程；它不把这些包合成一个更大的私有作用域。

## 语法

~~~text
module_def -> mod id { (configuration | definition | statement)* }
visibility -> pub | private
use_statement -> visibility? use path
configuration -> .id expr
~~~

路径、声明和文件作用域见[全局语法](../grammar/syntax)。目录与包描述文件属于源码组织约定，不是额外的语言关键字。

## 正式定义

### 阅读路线

| 文档 | 解决的问题 |
|---|---|
| [包与 umbrella](packages-and-umbrella.md) | 包身份、依赖、默认根文件、工程成员与工具职责 |
| [源码与模块](modules-and-sources.md) | 文件／目录映射、mod.fl、扫描、模块属性与重名 |
| [作用域与路径](scope-and-paths.md) | 统一命名空间、相对路径、包根定位与遮蔽 |
| [可见性](visibility.md) | 三档访问范围、路径访问与来源许可 |
| [导入与再导出](imports-and-reexports.md) | 单项、分组、别名、通配、歧义与循环 |
| [内置包](builtin-packages.md) | 六个入口、prelude、属性与 build 上下文 |
| [配置、脚本与构建](configuration-and-scripts.md) | comptime 配置、宏准备、生成源码与依赖调度 |

### 四种关系

源码发现确定当前编译输入。模块包含关系确定名称容器。包依赖使被依赖接口进入构建世界。构建依赖规定某项工作开始之前必须取得什么。

这些关系不能互相代替：扫描到脚本不意味着执行它；导入一个名字不意味着执行其主体；同处 umbrella 不意味着取得另一个包的私有权限。

~~~mermaid
flowchart TD
    U[Umbrella] --> A[应用包]
    U --> L[库包]
    A -->|显式依赖| L
    A --> AR[包根与模块树]
    L --> LR[包根与模块树]
    LR --> I[公开声明与再导出]
    I -->|名称绑定| AR
~~~

导入保持原始声明身份。类型经别名和再导出到达使用者后，仍属于原来定义它的包；[implementation 归属与 coherence](../traits/coherence.md)据此检查。

## 常见示例

### 一个文件内的公共入口

~~~flurry
use std.prelude.*;

private mod implementation {
    pub fn answer() -> u32 {
        42
    }
}

pub use implementation.answer;

fn main() {
    println("{}", answer());
}
~~~

输出 `42`。当前包公开 answer，外部使用者不需要经过 implementation。std.prelude 的普通导入只提供包内便捷名称，不成为公共出口。

### 拆成两个文件

布局：

~~~text
src/
├── main.fl
└── arithmetic.fl
~~~

`arithmetic.fl`：

~~~flurry
fn twice(value: u32) -> u32 {
    value * 2
}
~~~

`main.fl`：

~~~flurry
use std.prelude.*;
use arithmetic.twice;

fn main() {
    println("{}", twice(21));
}
~~~

arithmetic 和 twice 默认在包内可见。文件通过扫描进入模块树，use 只建立 twice 的短名。

## 与 Rust / Zig / C / C++ 的关键差异

| 对照 | 对应机制与区别 |
|---|---|
| Rust | 模块、use、pub use 分别提供组织、绑定与再导出；Flurry 的无修饰声明默认包内可见，且 label 与普通名字共用命名空间。[Rust 可见性](https://doc.rust-lang.org/reference/visibility-and-privacy.html)、[命名空间](https://doc.rust-lang.org/reference/names/namespaces.html) |
| Zig | 文件导入取得容器类型，构建程序组织模块；Flurry 从纳入的源码目录建立文件与目录模块，再解析 use。[Zig import](https://ziglang.org/documentation/master/#import) |
| C | 无对等的包私有模块系统；实际以头文件提供声明，以翻译单元和链接组织实现。Flurry 的 use 不进行源码文本包含。[C 头文件](https://gcc.gnu.org/onlinedocs/cpp/Header-Files.html) |
| C++ | namespace、using 和模块导出分别承担名称与接口组织；Flurry 的包默认可见性与自动目录模块是不同的边界。[C++ using](https://eel.is/c++draft/namespace.udecl)、[模块导入](https://eel.is/c++draft/module.import) |

## 运行时实现设计引导

普通导入没有独立的运行时初始化动作。运行时执行的是实际调用、数据访问和已经生成的代码。

代码生成应按原始声明及其具体实例引用实现。多个导入别名不要求复制函数主体、全局存储或类型描述；是否合并机器代码仍须遵守类型身份与程序行为。

## 编译器实现引导

前端接收已选定构建实例及其源码集合，建立模块树，登记声明和导入，再完成名称、权限及类型检查。各阶段应交付以下信息：

1. 源码发现交付文件角色、所属模块及结构依赖。
2. 名称解析交付原始声明、当前绑定和合法访问路径。
3. 可见性检查交付来源许可与公开出口，不能只留一个布尔 public 标志。
4. 编译时检查沿用定义所属构建上下文；具体实例化保留其依赖。
5. 构建调度把宏与生成源码作为明确产物交付后续阶段。

名称循环、求值循环和构建准备循环分别诊断。详细的有限传播算法见[导入与再导出](imports-and-reexports.md#编译器实现引导)，调度与失效规则见[配置、脚本与构建](configuration-and-scripts.md#编译器实现引导)。
