# 包与 umbrella

## Intro

包把一组定义作为一个依赖单位交付。调用者使用它的公开接口，包内实现则可以通过默认可见性相互协作。

Umbrella 让多个包共享工程配置和构建安排。成员的定义仍各自属于自己的包。

## 语法

~~~text
use_statement -> visibility? use path
module_def -> mod id { (configuration | definition | statement)* }
~~~

包布局使用 package.fl 描述文件和默认 src 源码根。描述文件中的计算使用普通 comptime Flurry；接口见[配置与构建](configuration-and-scripts.md)。

## 正式定义

### 包、目标与根文件

| 对象 | 作用 |
|---|---|
| package | 定义归属、包内访问与依赖接口的边界 |
| 构建目标 | 本次要求交付的产物及对应编译输入 |
| 构建实例 | 一份源码在本次目标、配置与依赖输入下接受检查的上下文 |
| umbrella | 成员组织、共享配置与构建协调 |

可执行包默认以 src/main.fl 为根，库包默认以 src/lib.fl 为根。根文件的声明属于包根，不额外嵌套在 main 或 lib 文件模块里。具体构建先确定根和源码集合，再进行目录发现；磁盘上出现另一个根候选文件，本身不建立额外的 package 身份。

### 依赖与成员

一个 umbrella 中的包需要使用另一个包时，仍须具有相应依赖。成员目录的相邻关系不授予名称、私有访问或 implementation 归属权。

共享工程配置不要求每个成员脱离工程后还能独立构建，也不意味着把成员模块树合并。Umbrella 自身不自动成为语言路径的额外一节。

当前构建的合法依赖和静态作用域确定可参与检查的实现世界。仅构建一个成员，不因工程中还有其他成员就纳入所有旁支 implementation。反过来，已进入实现世界的冲突也不能靠减少 use 隐藏。

### 声明身份

再导出的类型、trait 或函数保持原始定义归属。使用者不能通过给外部类型取本地名字，取得本地名义定义的权限。[newtype](../type/user-defined-types/newtype-and-typealias.md)与别名承担不同职责。

[Open fn 的分散定义](../functions/templates.md)在 package 内封闭；umbrella 不扩大这个边界。

### 工具职责

luna 以单二进制 CLI 统一提供语言编译、检查以及包、工程和构建入口。其构建层确定依赖、目标与任务，再把相应输入交给编译管线；不另设 anar 工具。某一配置采用宿主执行环境，不意味着所生成程序也以宿主作为目标。

## 常见示例

### 同工程中的应用与库

下面 editor 对 model 具有显式依赖；目录名称只是工程布局。

~~~text
workspace/
├── editor/
│   ├── package.fl
│   └── src/main.fl
└── model/
    ├── package.fl
    └── src/lib.fl
~~~

`model/src/lib.fl`：

~~~flurry
pub fn default_limit() -> u32 {
    64
}

fn internal_limit() -> u32 {
    8
}
~~~

`editor/src/main.fl`：

~~~flurry
use std.prelude.*;
use model.default_limit;

fn main() {
    println("{}", default_limit());
}
~~~

输出 `64`。editor 不能以同处 umbrella 为由调用 model.internal_limit。

### 再导出仍属于原包

库包 protocol 定义：

~~~flurry
pub struct Packet {}
~~~

依赖 protocol 的包 facade 提供：

~~~flurry
pub use protocol.Packet;
~~~

同时依赖两者的使用者可以写：

~~~flurry
use protocol.Packet as OriginalPacket;
use facade.Packet as PublicPacket;

comptime {
    assumes OriginalPacket == PublicPacket;
}
~~~

两个名字指向同一个名义类型。facade 没有因此成为 Packet 的定义包。

## 与 Rust / Zig / C / C++ 的关键差异

Cargo workspace 也组织多个 package 并共享工程设置；Flurry 的 umbrella 保持成员独立，语言中的归属与访问仍以 package 为边界。[Cargo workspace](https://doc.rust-lang.org/cargo/reference/workspaces.html)

Zig 构建图可以组织多个模块和产物；Flurry 另以 package 定义名义归属与默认访问范围。[Zig 构建系统](https://ziglang.org/learn/build-system/)

C 的翻译单元、C++ 的模块和命名空间都不能直接等同于这里的 umbrella；C/C++ 工程可以用 CMake 组织多个目标，但构建目标关系本身不建立 Flurry 式包内访问资格。[C 头文件](https://gcc.gnu.org/onlinedocs/cpp/Header-Files.html)、[C++ 模块](https://eel.is/c++draft/module.import)、[CMake 构建步骤](https://cmake.org/cmake/help/latest/command/add_custom_command.html)

## 运行时实现设计引导

包依赖最终体现为生成程序实际引用的代码、数据及链接要求。工程成员关系没有额外运行时对象。

链接与实例缓存必须保留声明身份和构建输入。两个包重导出同一声明，不要求生成两份独立存储；两个真正不同的名义声明也不能因名称和布局相同而合并成一个语言类型。

## 编译器实现引导

luna 的构建层向编译过程交付包身份、根文件、源码集合、合法依赖和本次构建环境。实现可按以下步骤组织：

1. 从选定目标取得所需的包与准备任务，不先把全部 umbrella 成员并为一个作用域。
2. 为原始声明保留定义包与模块身份，使用导入边记录其他入口。
3. 将可见性检查与 implementation 归属检查分别执行，不能用“名字可见”替代“允许定义该实现”。
4. 对生成定义保持原阶段的线性可见性与整体 coherence 要求。
5. 以实际消费的配置和依赖结果记录缓存依赖。

依赖集合大小为 P、依赖边数为 E 时，收集一个已知合法图中的可达集合可用 O(P + E)；配置求值、版本解算与代码检查另计。缺失依赖、非法访问和定义冲突应分别报告，不能统一降为模糊的“找不到包”。
