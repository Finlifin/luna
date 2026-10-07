# 可见性

## Intro

可见性规定谁能通过一个名称访问声明。包内协作使用默认范围；公开接口和更小的模块内部接口分别用 pub 与 private 表达。

一个公开声明可以放在隐藏的实现模块里，再由另一个入口提供给使用者。隐藏组织路径与隐藏声明本身，是两个不同的选择。

## 语法

~~~text
visibility -> pub | private
visibility_definition -> visibility definition
use_statement -> visibility? use path
configuration -> .id expr
~~~

可见性修饰定义或导入入口。文件／目录模块自身的属性由所属主体的 configuration 提供，见[源码与模块](modules-and-sources.md)。

## 正式定义

### 三档范围

| 声明形式 | 允许访问的范围 |
|---|---|
| 无可见性修饰 | 声明所属 package |
| private | 声明所属模块及其子模块 |
| pub | 全局，但使用者仍须有合法可达路径 |

默认选择包内可见是为了让包内定义无需逐项标注即可协作，同时防止意外向外部暴露实现细节。private 所在模块是该声明的词法所属模块；把名字导入另一处，不会重新定义原声明的私有范围。包内可见也不等于包内每处都自动取得短名，名称路径与访问许可分别判断。

### 路径与声明

一条直接路径经过的每个入口，都必须允许当前使用者访问。目标声明是 pub，不授予穿过 private 中间模块的资格。

合法再导出提供新的入口。书写 use 的作用域必须能访问来源路径；之后使用者通过新入口访问时，不需要重走原来的隐藏路径。

### 再导出的上限

再导出不能扩大原始声明本身的许可。公开一个 pub 声明可以绕开不公开的容器路径；原始声明只有包内或 private 许可时，pub use 静态拒绝。

普通 use 的名字默认包内可见，private use 的名字限制在所属模块及子模块。实际访问还须满足原始声明的许可。由普通 use 建立的内部别名若指向原本 pub 的声明，可以再被显式 pub use；仅增加别名层数不能公开非 pub 来源。

因此，需要分别判断：

1. 导入者能否访问所写来源路径。
2. 新名字在哪个范围可用。
3. 原始声明是否允许向该范围交付。

同一声明可以有多个合法入口。一个入口不可访问，不会撤销另一个已经合法建立的公开入口；身份去重也不能把非法路径变成合法路径。

## 常见示例

### 默认供包内协作

~~~flurry
use std.prelude.*;

mod limits {
    fn default_limit() -> u32 {
        64
    }
}

mod worker {
    fn run() {
        println("{}", limits.default_limit());
    }
}

fn main() {
    worker.run();
}
~~~

输出 `64`。这些声明默认允许同包访问，不需要为内部协作逐个标 pub。

### private 对子模块可见

~~~flurry
mod service {
    private fn secret_limit() -> u32 {
        8
    }

    mod worker {
        fn limit() -> u32 {
            secret_limit()
        }
    }

    fn limit() -> u32 {
        worker.limit()
    }
}
~~~

service.worker 可以通过父级查找使用 secret_limit。另一个兄弟模块不能导入它；下面是独立的拒绝片段：

~~~flurry
mod client {
    use @service.secret_limit;
}
~~~

### 隐藏路径，保留公开入口

库包 transport 的根文件：

~~~flurry
private mod backend {
    pub struct Packet {}

    pub fn protocol_version() -> u32 {
        1
    }
}

pub use backend.{Packet, protocol_version}
~~~

依赖它的包：

~~~flurry
use std.prelude.*;
use transport.{Packet, protocol_version}

fn main() {
    let packet = Packet {}
    println("{}", protocol_version());
}
~~~

公开入口合法，输出 `1`。直接访问 transport.backend 会失败。

### 别名不能扩大来源许可

下面在包内可以取得 Scratch，但尝试公开它时静态拒绝：

~~~flurry
mod backend {
    struct Scratch {}
}

use backend.Scratch as Workspace;
pub use Workspace as PublicWorkspace;
~~~

拒绝依据是原始 Scratch 的包内许可，不是别名采用了哪个名字。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 默认私有并提供 pub(crate) 等范围；Flurry 的默认范围是 package，private 则缩到所属模块及后代。两者都能通过公开再导出提供隐藏实现路径中的公开项。[Rust 可见性与再导出](https://doc.rust-lang.org/reference/visibility-and-privacy.html)

Zig 通过 pub 控制跨文件成员访问；Flurry 另外保留默认包内可见这一级。[Zig 容器与导入](https://ziglang.org/documentation/master/#import)

C 文件级 static 控制内部链接，C++ 类的 private 控制成员访问；都不能直接替代这里的包／模块访问范围。C++ 模块 export 则控制导出的接口，仍不是 Flurry 的默认包内许可。[C11 §6.2.2](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf)、[C++ 成员访问](https://eel.is/c++draft/class.access)、[模块导出](https://eel.is/c++draft/module.interface)

## 运行时实现设计引导

可见性通常在编译期完成检查，不要求每次函数调用附带运行时访问令牌。后端的链接符号可见性不能替代语言检查：优化可能合并或消去函数，也可能为调试保留符号，但这些选择不改变源码访问资格。

类型或函数从另一公开入口到达时，运行时仍使用同一个语义实体。不要为了隐藏源路径复制状态或制造另一份名义类型描述。

## 编译器实现引导

将访问者表示为包身份和词法模块位置。一个声明的许可域可以表示为 All、Package(p) 或 Descendants(m)，后者包含 m 本身；模块树的祖先查询可通过区间编号或父链实现。

为每个名称入口保存所属作用域、入口许可域、原始声明身份，以及来源路径已在导入点通过检查的依据。

~~~text
check_import(importer, source_path, requested_visibility):
    source = resolve_and_check_every_step(importer, source_path)
    if requested_visibility is Public:
        require source.original_declaration.visibility is Public
    return Binding(
        origin = source.origin,
        audience = requested_visibility.domain(importer),
        source_proof = source.access_proof
    )

check_binding_access(user, binding):
    require user in binding.audience
    require user in binding.origin.declaration_audience
~~~

消费再导出时检查新路径和原声明许可，不把 source_proof 的原始中间路径重新要求给消费者。另一方面，创建绑定时不能仅拿到最终声明身份就绕过来源路径检查。

缓存访问判定时必须带上访问者或可复用的许可域证明。一次内部访问成功，不足以让后续外部访问复用同一 true。失败诊断指出阻挡访问的具体入口，或指出 pub use 试图扩大哪一个原始声明的许可。
