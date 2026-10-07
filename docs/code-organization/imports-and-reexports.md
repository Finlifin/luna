# 导入与再导出

## Intro

use 给已有声明建立一个便于使用的名字。pub use 还把这个名字作为所在模块的公开入口。

导入路径可以变化，声明身份不变。判断同名是否冲突时，要看它们最终指向的声明。

## 语法

~~~text
use_statement -> visibility? use path
visibility -> pub | private
projection_path -> path . id
projection_multi_path -> path . { path* }
projection_all_path -> path . *
path_as_bind -> path as id
~~~

基础名称、父级与包根定位见[路径](scope-and-paths.md)。分组中的每个 path 接在共同前缀之后解释；as 为所选项建立本地别名。

## 正式定义

### 名称绑定

普通 use 默认建立包内可见入口，private use 建立模块及后代可见入口，pub use 建立公开入口。三者都使合法引入的名称能在当前作用域使用。

导入不执行被导入函数，也不产生新的类型声明、implementation 归属或运行时存储。别名没有改变声明的 comptime/runtime 要求。

显式导入必须取得唯一、可访问的目标。路径不存在、越权、或与本层确定命名项形成不同来源的重名，均是静态错误。

### 通配汇集与优先级

普通通配汇集导入位置可以访问的成员，包含合法的包内成员；公开通配仅汇集来源容器的 pub 成员入口，并核验原始来源也允许公开。

容器成员包括它的直接声明与合法导入入口，不包括只经词法父级查找到的名称。因此，某个模块内部能调用 println，不足以说明模块自身公开了 println。

在同一层，直接声明与显式导入优先于通配项。选名先于公开过滤：一个非公开确定项遮蔽了公开通配项后，pub 通配不能为了找到可导出项而恢复被遮蔽者。

### 身份与歧义

| 来源集合 | 结果 |
|---|---|
| 多条路径到达同一原始声明 | 合并声明身份，保留有效访问路径 |
| 同一优先级到达多个不同声明 | 歧义 |
| 找不到任何真实来源 | 名称不存在 |
| 来源存在但当前访问无资格 | 访问错误 |

两个类型结构相同、两个常量值相等，都不表示声明身份相同。导入次序不能用来选择歧义候选。

内部通配歧义到实际使用该名字时报告。公开出口的歧义由提供该出口的包检查，不能留给下游首次使用时才发现。显式导入本身就是对目标名称的要求，不享有“尚未使用”的延迟。

### 循环

模块之间允许循环导入与再导出，但名称必须有真实声明来源。环本身不会制造声明；同一来源绕环返回也不会变成新身份。

互相通配的空模块可以稳定为空。显式导入要求环中一个没有真实来源的名字时，最终报告缺失。名称关系中的环不授权构建准备环或编译时求值环。

## 常见示例

### 分组与别名

~~~flurry
use std.prelude.*;

mod codec {
    struct Packet {}

    fn version() -> u32 {
        2
    }
}

use codec.{Packet, version}
use codec.Packet as WirePacket;

test {
    let packet: WirePacket = Packet {}
    println("{}", version());
}
~~~

输出 `2`。WirePacket 与 Packet 指向相同声明。

### 显式导入消除通配竞争

~~~flurry
use std.prelude.*;

mod first {
    fn value() -> u32 { 1 }
}

mod second {
    fn value() -> u32 { 2 }
}

use first.*;
use second.*;
use first.value;

test {
    println("{}", value());
}
~~~

输出 `1`。删除显式 use 后，两个通配声明仍可存在，但 value() 的使用歧义。交换两个通配 use 不改变结果。

### 公开通配过滤内部辅助项

~~~flurry
mod implementation {
    pub fn version() -> u32 { 4 }
    fn scratch_size() -> u32 { 8 }
}

pub use implementation.*;
~~~

所在包公开 version，不公开 scratch_size。新增一个默认可见的辅助函数不会使整条公开通配失败。显式写 pub use implementation.scratch_size 则被拒绝。

### 先遮蔽，再决定出口

~~~flurry
mod source {
    pub struct Packet {}
}

mod facade {
    pub use @source.*;
    struct Packet {}
}
~~~

facade 中选中的是本地 Packet，它仅包内可见。其他位置对 facade 做公开通配时，不会重新捞出 source.Packet 充当该名字的公开版本。

### 菱形路径只有一个声明

~~~flurry
mod source {
    pub struct Packet {}
}

mod first {
    pub use @source.Packet;
}

mod second {
    pub use @source.Packet;
}

use first.*;
use second.*;

fn packet() -> Packet {
    Packet {}
}
~~~

两个通配都指向 source.Packet，packet 的结果类型唯一。

### 有来源的循环

~~~flurry
mod source {
    pub struct Packet {}
}

pub mod left {
    pub use @source.Packet;
    pub use @right.*;
}

pub mod right {
    pub use @left.*;
}

comptime {
    assumes left.Packet == right.Packet;
}
~~~

两条路径指向 source.Packet。删除真实来源及指向它的显式导入后，左右通配环为空；再要求从其中取得 Packet 才形成缺失错误。

### 提供包拒绝歧义出口

以下是独立的拒绝案例：

~~~flurry
mod first {
    pub struct Packet {}
}

mod second {
    pub struct Packet {}
}

pub use first.*;
pub use second.*;
~~~

包根的公开 Packet 有两个不同来源。没有任何下游使用者时，提供包也应报告这个出口歧义。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 的 item 和显式导入也可遮蔽 glob 来源，同一项经多条 glob 路径引入可合并；但其查找按多个命名空间区分，Flurry 的普通名称统一处理。[Rust use](https://doc.rust-lang.org/reference/items/use-declarations.html)、[名称解析](https://doc.rust-lang.org/reference/names/name-resolution.html)

Zig 可以将导入容器的成员绑定为公开声明；C++ using-declaration 可以引入已有成员。Flurry 的 pub use 还受原始声明许可与公开通配集合约束。[Zig import](https://ziglang.org/documentation/master/#import)、[C++ using](https://eel.is/c++draft/namespace.udecl)

C 没有对等的声明再导出语句，实际通过头文件声明、包装函数或预处理宏提供接口；头文件包含会处理源文本，use 保持已存在声明的身份。[C 头文件](https://gcc.gnu.org/onlinedocs/cpp/Header-Files.html)

## 运行时实现设计引导

解析后的普通导入不产生查找表遍历或额外函数调用。使用多个名字调用一个函数，可以直接引用同一目标；调试信息仍可记录用户写下的入口。

再导出不替代动态接口包装、ABI 适配或资源转换。只有名字改变时，不应生成这类额外动作；确实存在相应语言操作时，再按其自身契约处理。

## 编译器实现引导

### 表示与查询

至少保留以下概念，物理上可以合并存储：

- 原始声明：稳定身份、所属包、声明许可与承载作用域。
- 名称入口：所在作用域、可见性、显式／通配来源及最终原始身份。
- 导入关系：路径定位、每段依赖、来源证明与消费者。
- 查询状态：待解、无来源、唯一、歧义、访问失败。

本地查找必须同时考虑普通导入和再导出；来源查询必须取得来源的有效成员，不能只读取来源的直接声明。

### 先登记，后传播

1. 登记所有静态直接声明，预先登记显式导入引入的名字。不同确定项的冲突在取得真实身份后判定；未解显式项仍占位。
2. 将分组路径拆成共享前缀的选择任务，保留原始源码位置；定位与逐段访问使用相同规则。
3. 建立查询依赖图，区分容器成员查询、普通词法查询与公开出口查询。
4. 对循环依赖建立强连通分量；已有直接声明是传播的真实来源，纯引用边不是来源。
5. 在没有显式名字占位的位置传播通配候选，按原始身份去重；每个入口仍保留能证明访问的来源边。
6. 待相关分量稳定，再确定唯一性、权限和公开集合，发布结果。

对于路径前缀已经确定、遮蔽关系已经登记的一个分量，可使用下面的有限工作队列：

~~~text
facts = direct_declaration_facts()
queue = newly_added_facts(facts)
while queue is not empty:
    fact = queue.pop()
    for edge in consumers(fact):
        if blocked_by_explicit_name(edge, fact.name):
            continue
        if eligible_for_this_import(edge, fact):
            add_origin_and_support(edge.destination, fact, queue)
validate_required_names_and_public_exports()
~~~

事实以“作用域、名字、原始声明、入口许可”去重，支持关系存导入边，不保存无限长的绕环路径。待解路径前缀不能被当作已确定边加入此算法；其查询先保留全部可能来源，只有完成唯一性和访问检查后，才能将依赖结果发布给消费者。

显式占位、公开过滤与路径唯一性是算法的前提和约束，不能简单对所有可达声明无条件求并集。若一条被要求的导入路径最终歧义或无来源，其结果及依赖该结果的绑定都不能交付。实现需要保留支持依赖，避免把尚未成立的候选变成独立的真实来源。

### 完成条件与错误

空通配分量可以完成为空。含显式名字要求却无真实来源的分量报告缺失，并给出相互依赖链。不同真实来源不能通过任意选择一个来打破循环；按内部使用或公开出口的规则报告歧义。

若有效候选事实总数为 F，导入边数为 E，固定边上工作队列的成本由实际事实传播次数决定；粗略上界为 O(FE)，索引可以减少无关边访问。路径前缀求解、属性求值及访问证明成本另计。不能声称每条 use 只处理一次即可完成任意传递导入。

### 增量与验收

缓存应依赖来源的成员集合、所查名字的存在／缺失、公开出口、许可域和显式遮蔽关系。新增一个冲突声明会让此前唯一的通配结果失效；删除来源也必须撤销没有其他支持的传播结果。

至少检查两跳别名、菱形来源、种子循环、空循环、公开冲突、包内通配、非公开项遮蔽公开项，以及交换声明扫描次序后结果不变。全量与增量解析必须得到相同的声明身份、权限与诊断。
