# Flurry 演进手记 (039)：精确控制谁能看见什么——`use` 导入、重导出、`private`/`pub` 与六个内置包

在 [上一篇 (038)](./038-modules-file-mapping-and-sequential-shadowing.md) 中，我们把单文件传感器项目拆分成了清晰的目录与模块树。
现在，我们需要把其中的传感器通信协议模块打包成一个独立的库包（比如 `transport`），交付给其他业务团队使用。

在设计这个库包的公共接口时，作为架构师的你面临三个核心问题：
1. **三档可见性怎么选？** 我希望包内的各个子模块之间无需到处写修饰符就能顺畅协作，但同时又想把底层硬件缓冲池锁死在单个模块内部，最后只把精挑细选的 `Packet` 和 `protocol_version()` 暴露给外部包。
2. **门面重导出（Facade Re-export）怎么做？** 如果我把实现藏在 `private mod backend` 里，然后在包根写 `pub use backend.{Packet, protocol_version}`，外部用户能绕过 `backend` 直接访问它们吗？如果两个模块用 `use first.*; use second.*;` 通配导入了同名项，编译器是怎么区分“菱形同源合并”与“真实冲突歧义”的？
3. **那些无处不在的基础设施从哪来？** 为什么 `u32` 无需 `use`，而 `println` 需要 `use std.prelude.*;`？当我们需要读取编译模式 `build.mode`、写属性 `^attrs...` 或做形式化验证时，Flurry 的**六个内置包**是如何分工的？

今天，我们将深入 `docs/code-organization/visibility.md`、`imports-and-reexports.md` 与 `builtin-packages.md`，补齐模块化工程的最后一块拼图。

---

## 一、 无须到处写 `pub(crate)`——Flurry 的三档可见性设计

如果你写过大型 Rust 项目，一定对满屏的 `pub(crate)` 或 `pub(super)` 印象深刻：因为 Rust 默认是模块私有（Private by default），哪怕在同一个包内部拆分两个小文件，你也得给所有内部协作函数挨个标上 `pub(crate)`。反过来，在 C++ 或 Python 里，默认又太容易把内部实现细节暴露给外部消费者。

Flurry 采用了一套契合工程直觉的**三档可见性模型**（详见 `docs/code-organization/visibility.md`）：

```text
  ══════════════════════════ Flurry 的三档可见性范围 ══════════════════════════

  ┌──────────────────┬─────────────────────────────┬──────────────────────────┐
  │    声明修饰符     │         允许访问的范围        │       典型工程用途        │
  ├──────────────────┼─────────────────────────────┼──────────────────────────┤
  │  (无修饰 / 默认)  │ 声明所属的整个 Package 内部  │ 包内各模块无痛协作        │
  ├──────────────────┼─────────────────────────────┼──────────────────────────┤
  │  private         │ 声明所属模块及其后代子模块    │ 锁死模块内部机密实现细节  │
  ├──────────────────┼─────────────────────────────┼──────────────────────────┤
  │  pub             │ 全局（跨包可见，需合法路径）  │ 对外交付的正式公共 API    │
  └──────────────────┴─────────────────────────────┴──────────────────────────┘
  (⚠️ 语法规则：一项声明至多指定一档显式可见性，无法叠写 `pub private`。)
```

让我们看看 `private`、默认可见性与 `pub` 在同一个包内是如何配合的：

```flurry
use std.prelude.*;

mod service {
    -- 1. `private`：仅 service 模块自身及其子模块 (如 service.worker) 可见。
    private fn secret_limit() -> u32 {
        8
    }

    mod worker {
        -- 子模块可以通过父级查找合法调用 secret_limit()。
        fn limit() -> u32 {
            secret_limit()
        }
    }

    -- 2. 无修饰（默认）：整个包内可见。兄弟模块 main 可以调用 service.limit()
    fn limit() -> u32 {
        worker.limit()
    }
}

fn main() {
    println("{}", service.limit()); -- 输出 8
}
```

如果另一个兄弟模块 `client` 试图越权访问 `service` 里的 `private fn secret_limit`：

```flurry
-- ❌ 编译期直接报错。
mod client {
    -- 错误。secret_limit 是 service 的 private 成员，兄弟模块 client 并非属 service 的后代。
    use .service.secret_limit;
}
```

---

## 二、 `use` 与 `pub use`——隐藏内部路径、原始身份去重与通配规则

有了三档可见性，我们怎么组织优雅的对外 API 呢？
在大型库中，我们经常把代码按职责拆进 `private mod backend` 或多层内部子目录，然后通过 **`pub use`** 在包根建立一个干净的公开门面（Facade）。

### 1. 隐藏组织路径 $\neq$ 隐藏声明本身

请看库包 `transport` 的根文件 `src/lib.fl`：

```flurry
-- 内部实现模块标为 private，外部包完全无法直接访问 transport.backend。
private mod backend {
    pub struct Packet {}

    pub fn protocol_version() -> u32 {
        1
    }

    -- 注意：Scratch 没有标 pub，默认只有包内可见许可。
    struct Scratch {}
}

-- 在包根通过 pub use 建立公开入口。
-- 因为包根自己能访问 backend，且 Packet 和 protocol_version 原始声明是 pub，重导出合法。
pub use backend.{Packet, protocol_version}
```

当外部包依赖 `transport` 时：
- 写 `use transport.{Packet, protocol_version};` **完全合法**。消费者通过包根的新入口访问，无需重新走一遍已经被隐藏的 `backend` 路径。
- 但如果库作者在包根试图用别名把并非 `pub` 的 `Scratch` 泄漏出去呢？

```flurry
-- ❌ 编译期直接报错。
use backend.Scratch as Workspace;
-- 错误。再导出无法扩大原始声明本身的许可。
-- 原始声明 Scratch 只有包内许可（未标 pub），哪怕套一层别名亦无法被 pub use 公开。
pub use Workspace as PublicWorkspace;
```

### 2. 分组、别名、通配 `*` 与“原始声明身份”去重

在书写 `use` 时，Flurry 支持四种灵活的路径形态（详见 `docs/code-organization/imports-and-reexports.md`）：
- **单项导入**：`use codec.Packet;`
- **分组导入**：`use codec.{Packet, version}`（注意：以 `}` 结尾的 `use` 语句同样省略末尾分号。）
- **别名导入**：`use codec.Packet as WirePacket;`
- **通配导入**：`use first.*;` 或公开通配 `pub use implementation.*;`

当一个作用域里同时出现了直接声明、显式导入和通配导入 `*` 时，Flurry 的名称解析器遵循四条清晰明确的法则：

```text
  ══════════════════════ 同一层作用域内的选名与去重法则 ══════════════════════

  [法则 1：显式优先于通配]
  本层的直接声明与显式 `use foo.bar;` 永远优先于通配 `use other.*;`。

  [法则 2：公开通配自动过滤并非 pub 项]
  `pub use implementation.*;` 只会汇集并导出 `implementation` 中标了 `pub` 的成员，
  自动忽略里面默认包内可见的辅助函数（如 `fn scratch_size()`），免因有内部辅助函数而报错。

  [法则 3：菱形同源自动合并（按原始声明身份 Origin Identity 去重）]
           mod source { pub struct Packet {} }
                  ╱                   ╲
       mod first {                  mod second {
         pub use .source.Packet;      pub use .source.Packet;
       }                            }
                  ╲                   ╱
             use first.*;       use second.*;
                           │
                           ▼
        两条通配路径最终指向同一个原始声明 `source.Packet`。
        ──► 自动合并为唯一结果，不误报歧义。

  [法则 4：不同原始身份触发通配歧义]
  如果 `first.Packet` 和 `second.Packet` 是两个各自独立定义的 `struct Packet {}`，
  - 在内部 `use first.*; use second.*;` 时，直到实际使用 `Packet` 时报告歧义（或者你可以加一行显式 `use first.Packet;` 消歧。）；
  - 但如果是包根对外写 `pub use first.*; pub use second.*;` 导致公开出口产生歧义，
    **提供该出口的包在编译时就会直接报错**，免将问题留给下游用户。
```

来看看这组法则在代码中的直观体现：

```flurry
use std.prelude.*;

mod first {
    fn value() -> u32 { 1 }
}

mod second {
    fn value() -> u32 { 2 }
}

use first.*;
use second.*;
-- 显式导入优先于上面的两个通配 `*`，一锤定音选中 first.value。
use first.value;

test {
    println("{}", value()); -- 稳稳输出 1。
}
```

---

## 三、 Flurry 的六大内置包全景——从 `builtin` 到 `build` 与 `verification`

现在，我们来回答那个从第 001 篇一直陪伴我们到现在的问题：
**为什么我们在每个文件开头都会写 `use std.prelude.*;`，而 `u32`、`bool` 却无须导入？Flurry 的语言底座到底由哪些内置包组成？**

翻开 `docs/code-organization/builtin-packages.md`，Flurry 为每个工程提供了**六个职责分明的内置包（Builtin Packages）**：

```text
  ┌──────────────────┬──────────────────────────────────────────────────────────┐
  │    内置包名称     │                      核心职责与代表成员                   │
  ├──────────────────┼──────────────────────────────────────────────────────────┤
  │ 1. builtin       │ 最基础的语言核心内容（如基本类型 u32, f64, bool, BitVec） │
  │                  │ 👉 `builtin.prelude` 默认自动作为通配来源导入所有其他包   │
  ├──────────────────┼──────────────────────────────────────────────────────────┤
  │ 2. meta          │ comptime 基础设施与编译器暴露的编译期反射/类型操纵接口    │
  │                  │ （例如 `meta.List<Type>` 等）                             │
  ├──────────────────┼──────────────────────────────────────────────────────────┤
  │ 3. std           │ 跨平台系统编程标准库                                      │
  │                  │ 👉 `std.prelude` 不默认导入  需显式写 `use std.prelude.*;`│
  ├──────────────────┼──────────────────────────────────────────────────────────┤
  │ 4. build         │ 当前工程、构建实例信息与工程配置接口                      │
  │                  │ （例如 `build.mode matches .debug`, `package_metadata`） │
  ├──────────────────┼──────────────────────────────────────────────────────────┤
  │ 5. attrs         │ 语言提供的标准属性构造接口                                │
  │                  │ （例如 `^attrs.abi(.c)`, `^attrs.derivation.default`）   │
  ├──────────────────┼──────────────────────────────────────────────────────────┤
  │ 6. verification  │ 程序验证（Program Verification）所需的规格对象与操作      │
  │                  │ （例如形式化规格中使用的逻辑集合 `Set`、映射 `Map` 等）   │
  └──────────────────┴──────────────────────────────────────────────────────────┘
```

仔细看这张表，你会发现三个非常精妙的设计细节：

### 1. `builtin.prelude` vs `std.prelude`
- **`builtin.prelude`** 默认导入到除 `builtin` 自身之外的所有包根中，作为一份普通的通配来源参与查找。注意：它**没有高于本地声明的特权**（如果你在本地定义了一个同名项，会正常按显式优先规则遮蔽它），而且它免于自动变成你这个包的 `pub` 公开接口。
- **`std.prelude`**（包含 `println`、`eprintln` 等系统 I/O 与常用工具）**不默认导入**。这样保证了纯内核、裸机固件或无需标准库的包保持高度纯净；需要时只需在包根写一行 `use std.prelude.*;`，包内的所有子模块就能直接通过父级查找使用 `println`。

### 2. 属性 `^expr` 复用完全相同的名称解析
在很多语言里，属性（Attribute / Decorator）住在另一个神秘的词法空间里。
而在 Flurry 中，**`^expr` 里的 `expr` 和普通表达式使用完全相同的名称解析规则**。属性位置不会开小灶去偷偷优先搜索 `attrs` 包；反过来，这也意味着你可以用普通的 `use ... as ...` 给长长的属性路径起一个简短优雅的本地别名：

```flurry
-- 用普通 use 给 attrs 里的属性构造器起一个清爽的别名。
use attrs.derivation.default as default_strategy;

trait Code {
    fn code(*self) -> u32;
}

struct Marker {}

^default_strategy
derive fn constant_code for Code {
    fn code(*self) -> u32 {
        7
    }
}

derive Code for Marker;
```

### 3. `build` 上下文：绑定于定义所属的构建实例，不是可变全局变量
当你用 `build` 内置包查询当前构建模式时，甚至无需任何特殊宏语法，直接用普通的投影和第 015 篇学过的 `matches` 模式测试即可：

```flurry
-- 直接用普通表达式 + matches 判断当前构建实例是否为 debug 模式。
const TRACE_ENABLED: bool = build.mode matches .debug;

fn trace_enabled() -> bool {
    TRACE_ENABLED
}

-- 如果希望让调用者把自己的构建配置传进来，显式使用 comptime 参数：
fn tracing_allowed(comptime requested: bool) -> bool {
    requested
}

fn local_choice() -> bool {
    tracing_allowed(build.mode matches .debug)
}
```

请注意：`TRACE_ENABLED` 在它**定义所属的那个包构建实例**中就已经确定了；其他包调用、内联或使用模板接入它时，免将 `TRACE_ENABLED` 篡改成消费者自己的全局状态。需要消费者传入选择时，就像 `tracing_allowed(comptime requested: bool)` 一样显式传参——一切依赖都在阳光下透明流转。

---

## 本篇小结

| 维度 | 核心语法与机制 | 关键规则与保障 |
| :--- | :--- | :--- |
| **三档可见性** | 无修饰（默认包内）、`private`（当前模块及后代）、`pub`（全局） | 包内协作零样板噪音；`private` 对子模块可见但对兄弟模块隔离；每项至多一个可见性修饰符 |
| **导入与重导出** | `use a.{b, c}`、`use a.*`、`use a as b`、`pub use ...` | `pub use` 可隐藏中间 `private` 模块路径，但无法扩大原始声明本身的许可 |
| **去重与消歧** | 显式优先于通配 `*`；按原始声明身份（Origin Identity）去重 | 菱形导入同一声明自动合并；不同声明冲突时可由显式 `use` 消歧；包根禁止对外导出歧义通配 |
| **六大内置包** | `builtin`, `meta`, `std`, `build`, `attrs`, `verification` | `builtin.prelude` 默认通配导入，`std.prelude` 显式导入；`^expr` 属性复用普通名称解析；`build` 绑定定义所在构建实例 |

---

### 下一篇预告

到这里，我们在 **《Flurry 演进手记》第二卷** 中学习的所有新武器已经全部集结完毕：
- **灵活强大的函数接口**：带前序依赖的默认参数 `.opt = ...`、异构变参 `...items`、扩展调用属性收集 `{ k: v }`、`implicit` 上下文参数与后缀 `do` 块；
- **进阶模式与闭包系统**：模式函数、`and expr is pat` 链式顺序匹配、`as whole` 部分复制与整体交付、带显式捕获环境的 `%c fn` 闭包；
- **代码复用与架构组织**：单次求值的 `quote fn`、带 CMTT 上下文的 `template fn` 与 `template test`、三层接收者适配与 `.as(View)` 视角链，以及 `mod` / `pub use` 模块化系统。

在下一篇（第二卷的里程碑收官篇）**《Flurry 演进手记 (040)：第二卷里程碑——构建模块化、可插拔的流式数据处理管线》** 中，我们将把所有这些积木融会贯通，从零组装出一个完整的工业级**传感器流式遥测处理管线（Modular Stream Pipeline）**，并用 `template test` 完成全套自动化验收。
