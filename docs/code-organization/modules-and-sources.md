# 源码与模块

## Intro

一个普通源码文件对应一个模块，目录也可以容纳模块。把文件放进选定的源码根，就把它纳入程序的源码集合。

模块入口负责组织该模块自己的声明；子文件的发现由目录扫描完成。

## 语法

~~~text
module_def -> mod id { (configuration | definition | statement)* }
filescope -> (configuration | definition | statement)*
configuration -> .id expr
~~~

属性和前缀可见性沿用[属性系统](../comptime/attribute.md)与[可见性](visibility.md)。文件作用域提供所属文件模块的主体。

## 正式定义

### 源码集合

默认递归扫描 package 的 src。纳入源码集合的 `.fl` 文件都尝试作为 Flurry 源码处理，其语法错误不能因为未被引用而忽略。

package.fl、scripts 和显式注册的生成源码按各自工程角色处理，不自动混入 src 的运行时模块树。额外源码根由构建输入明确纳入。一次目录枚举不能无界地追随生成任务不断写出的新文件。

发现文件不等于执行所有 comptime 主体、实例化所有参数化定义或运行所有测试。对应构造仍遵守其自身检查与执行规则。

### 文件到模块的映射

| 来源 | 建立的模块或主体 |
|---|---|
| 选定的 src/main.fl 或 src/lib.fl | 包根主体 |
| 普通 name.fl | 当前目录下的 name 模块 |
| 普通 name/ 目录 | 当前目录下的 name 模块 |
| name/mod.fl | name 目录模块自身的主体，不增加 mod 子模块 |
| 内联 mod name { ... } | 所在容器中的 name 模块 |

目录无需 mod.fl 也能存在。文件名相同不使不同父模块下的定义冲突；冲突以所属容器和名字共同判断。

同级 name.fl 与 name/ 不能隐式合并。内联 mod name 与同名自动模块也冲突。目录自己的 mod.fl 是该目录主体，不是与目录争夺同一名字的第二个定义。

### 模块的属性

自动建立的模块默认包内可见。文件中的 configuration 附着到该文件模块；目录 mod.fl 中的 configuration 附着到目录模块。需要配置一个没有入口文件的目录时，可以添加 mod.fl。

configuration 提供所属模块的属性，其键和值由对应属性契约检查。成员自己的 pub/private 修饰只作用于该成员，不能代替模块自身的访问范围。

### 模块作为类型

mod 是用于组织声明的类型，没有构造实例的方法。满足无子句限制的 mod、struct、enum、union 及其包含关系为静态名称解析提供起点；参数化定义的 normalization／instantiation 不被文件发现替代。

## 常见示例

### 目录不需要空入口文件

~~~text
src/
├── main.fl
└── utility/
    └── checksum.fl
~~~

`utility/checksum.fl`：

~~~flurry
fn pair_sum(first: u32, second: u32) -> u32 {
    first + second
}
~~~

`main.fl`：

~~~flurry
use std.prelude.*;
use utility.checksum.pair_sum;

fn main() {
    println("{}", pair_sum(10, 20));
}
~~~

utility 和 utility.checksum 都成立，输出 `30`。

### mod.fl 为目录提供入口

~~~text
src/
├── lib.fl
└── codec/
    ├── mod.fl
    └── text.fl
~~~

`codec/text.fl`：

~~~flurry
pub fn format_version() -> u32 {
    3
}
~~~

`codec/mod.fl`：

~~~flurry
pub use text.format_version;
~~~

`lib.fl`：

~~~flurry
pub use codec.format_version;
~~~

使用者可以从库包根取得 format_version；目录 codec 与文件 text 不必因此公开。每一层再导出都指向同一个函数。

### 内联模块仍然有用

~~~flurry
mod constants {
    const HEADER_SIZE: u32 = 8;
}

fn header_size() -> u32 {
    constants.HEADER_SIZE
}
~~~

这个模块直接写在所属文件中。如果同时添加同级 constants.fl，就形成静态重名错误，不能按文件扫描次序选择其中一个。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 的外部模块通过模块声明指定，并按其文件规则定位；Flurry 在选定源码根中自动发现文件和目录，同名文件／目录冲突直接拒绝。[Rust 模块](https://doc.rust-lang.org/reference/items/modules.html)

Zig 的 `@import` 可以取得文件容器；Flurry 的目录模块来自扫描，use 随后建立名字。[Zig import](https://ziglang.org/documentation/master/#import)

C 的头文件包含会把源文本提供给包含它的翻译单元；C++ namespace 可以跨多处声明扩展。Flurry 的文件模块不进行文本包含，也不因路径同名而合并两个主体。[C 头文件](https://gcc.gnu.org/onlinedocs/cpp/Header-Files.html)、[C++ namespace](https://eel.is/c++draft/namespace.def)

## 运行时实现设计引导

目录和文件模块不要求分配运行时容器。对其中声明的调用与数据访问，按相应实体的运行时契约生成代码。

同一文件经多个名称入口被使用时仍对应同一份声明集合。不能把每次 use 实现为重新执行文件，以免重复构造存储或重复执行实际 comptime 操作。

## 编译器实现引导

### 建树

输入为本次选定根、源码根集合与已交付的生成输入。先完成角色分类，再建立模块槽位：

~~~text
discover(directory, owner):
    entries = snapshot_entries(directory)
    for directory_entry in entries:
        reserve(owner, directory_entry.name, directory_module)
    for source_file in entries:
        if source_file is selected_package_root:
            attach_body(owner, source_file)
        else if source_file is mod.fl:
            attach_body(owner, source_file)
        else:
            reserve(owner, source_file.stem, file_module)
    reject_conflicting_reservations()
    attach_and_parse_bodies()
    recurse_into_registered_directories()
~~~

根文件识别由构建输入给出，不是在任何子目录里看到 main.fl 就把它当作新的包根。内联模块登记时复用同一槽位冲突检查。语法失败必须保留源位置，并使该输入集合检查失败。

采用名字索引时，F 个文件与 D 个目录的登记期望为 O(F + D)，读取与解析另计。模块身份依据包与模块关系建立，不使用文件枚举次序编号来定义语义。

### 增量发现

记录目录成员集合以及根文件、mod.fl 的角色依赖。新增、删除、重命名文件都可能改变模块树和冲突结果；只缓存已经读过的文件内容会漏掉这些变化。

生成输入交付失败时，不能用上次遗留的同名文件假装本次生成成功。源码集合必须对应已成功交付的构建结果。
