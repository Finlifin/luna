# Flurry 演进手记 (038)：把项目拆成清晰的模块树——文件映射、`mod`、父级查找与顺序遮蔽

在前面的 37 篇手记里，为了演示方便，我们几乎把所有代码都写在了单个文件里。
但现在，我们的传感器监控平台已经成为一个包含**协议编解码（`codec`）**、**校验工具（`utility`）**和**主控入口（`main`）**的中大型系统。把几千行代码堆在一个文件里已经彻底不可维护了。

今天，我们要把单文件原型重构成清晰的多文件模块树（详见 `docs/code-organization/modules-and-sources.md` 与 `docs/code-organization/scope-and-paths.md`）。在这个过程中，我们将解决四个每个项目都会遇到的核心问题：
1. **文件和文件夹是怎么变成模块（`mod`）的？** 需要像 Rust 那样到处手写 `mod foo;` 声明文件，或者在每个空文件夹下被迫建一个空的 `mod.rs` / `__init__.py` 吗？
2. **当模块层层嵌套时，怎么精准定位“上一级父容器”或“整个包的根目录”？**
3. **为什么说 Flurry 是“统一命名空间（Unified Namespace）”？** 当类型名、变量名、模块名甚至循环标签 `:retry` 撞名时，编译器会怎么处理？
4. **块内的顺序绑定 `let value = value + 1;` 是如何工作的？**

让我们从磁盘上的第一个文件夹开始，一步步搭起 Flurry 的代码大厦！

---

## 第一版：告别手动 `mod foo;` 与空入口文件——目录自动扫描映射（v0 → v1）

如果你写过 Rust 或 C/C++，在拆分文件时一定有过这样的烦恼：
- 在 C/C++ 里，`#include "checksum.h"` 本质上是把头文件里的几千行源码文本原封不动地复制粘贴进当前文件；
- 在 Rust 里，你在目录里新建了 `utility/checksum.rs`，不仅要在 `main.rs` 里写 `mod utility;`，还得专门建一个哪怕完全为空的 `utility/mod.rs`（或 `utility.rs`）写上 `pub mod checksum;`，否则编译器根本当它不存在！

在 Flurry 中，**源码目录本身就是模块树的蓝图**！
编译器默认递归扫描当前包（Package）的 `src/` 目录。只要你把 `.fl` 文件放进选定的源码根目录，它就自动成为程序源码集合的一部分（而且即便某个 `.fl` 文件暂时还没被别人调用，里面的语法错误也绝不会被忽略！）。

让我们画一张 **Code Cartoon**，看清磁盘上的文件与目录是如何一对一映射为内存中的模块树的：

```text
  磁盘上的 src/ 目录结构：                 编译器自动建立的模块树：
  src/                                    (Package Root: @)
  ├── main.fl  (或 lib.fl) ─────────────► 包根主体 (Package Root Body)
  ├── arithmetic.fl        ─────────────► mod arithmetic (文件模块)
  ├── utility/             ─────────────► mod utility    (目录模块，无需 mod.fl 也能存在！)
  │   └── checksum.fl      ─────────────►   └── mod checksum
  └── codec/               ─────────────► mod codec      (目录模块)
      ├── mod.fl           ─────────────►   ├── (附着为 codec 目录模块自身的主体！)
      └── text.fl          ─────────────►   └── mod text
```

看看这张映射表有多么清爽：
1. **目录无需空 `mod.fl` 也能存在**：比如上面的 `utility/` 目录下只有 `checksum.fl`，完全没有 `utility/mod.fl`，但 `utility` 和 `utility.checksum` 模块已经自动成立了！
2. **`name/mod.fl` 是目录模块自身的主体**：当你需要给 `codec/` 这个目录模块写配置属性（如 `.id expr`）、写公共函数或做 `pub use` 重导出时，只需添加 `codec/mod.fl`。注意：它提供的是 `codec` 模块自己的主体，**不会**多生出一个叫 `codec.mod` 的子模块！
3. **文件内部依然可以写内联 `mod name { ... }`**：比如在一个文件内部用 `mod constants { const HEADER_SIZE: u32 = 8; }` 组织一组局部常量。

来看看这套多文件工程跑起来的样子：

```flurry
-- [文件 1] src/utility/checksum.fl
fn pair_sum(first: u32, second: u32) -> u32 {
    first + second
}
```

```flurry
-- [文件 2] src/main.fl
use std.prelude.*;
use utility.checksum.pair_sum;

mod constants {
    const HEADER_SIZE: u32 = 8;
}

fn main() {
    -- 同一个包内的模块与函数默认包内可见（无需标 pub 即可在包内协作）！
    println("sum: {}, header: {}", pair_sum(10, 20), constants.HEADER_SIZE);
}
```

### 编译器严防死守的边界：拒绝同名模块隐式合并！

在 C++ 里，你可以在十个地方写 `namespace foo { ... }`，编译器会把它们悄悄合并成一个命名空间。这导致你在读代码时永远不知道 `foo` 里到底塞了多少东西。
在 Flurry 中，**一个模块名字只允许有一个唯一的主体来源，绝不隐式合并**！

以下三种情况，编译器在建树阶段就会当场报错拒绝：

```text
  ❌ 编译期直接拒绝的三种模块重名冲突：
  1. 同级目录下同时存在 `codec.fl` 文件 和 `codec/` 目录！（注意：codec/mod.fl 与 codec/ 目录不冲突，因为 mod.fl 就是该目录自己的主体）
  2. 文件里写了内联 `mod constants { ... }`，同时同级目录下又放了一个 `constants.fl` 文件！
  3. 同一个容器内写了两个同名的 `mod helper { ... }`！
```

---

## 第二版：在模块迷宫里指路——父级查找、`.Parent` 与 `@Root`（v2）

当我们的模块树长出了多层枝叶（甚至在 `struct` 内部还嵌套定义了子 `struct`），在深层模块里怎么引用外面的名字呢？

Flurry 区分了两种完全不同的查找方式（详见 `docs/code-organization/scope-and-paths.md`）：

### 1. 普通首名：从当前词法环境逐层向外找

如果你在代码里直接写一个不带前缀的名字（比如 `println`），编译器会从当前最内层的作用域出发，沿着包含它的父模块一直往外找，直到包根（Package Root）：

```flurry
use std.prelude.*;

mod report {
    fn show() {
        -- report 内部没有导入 println，但它可以通过父级作用域（包根）找到 println！
        println("ready");
    }
}
```

> **💡 核心心智模型：“在里面能找到” $\neq$ “变成了它的成员”！**
> 在上面的例子里，`report.show` 之所以能调用 `println`，是因为词法作用域向外找到了包根导入的 `println`。
> 但这**绝不意味着 `println` 变成了 `report` 模块自己的成员**！如果外部有人写 `report.println("hi")` 或者 `use report.println;`，编译器会直接报错——因为指定了容器后，就只查该容器自己的成员，绝不回退祖先！

### 2. 显式路径起点：前导点 `.path` 与包根 `@path`

当我们在深层模块（比如 `codec.decoder`）或嵌套类型容器（比如 `Outer.Inner`）里写 `use` 时，为了不写冗长易碎的相对路径，Flurry 提供了两个极其精准的定位锚点：
- **前导点 `.path`**：从**所属静态容器的父静态容器**开始查找！（注意：普通函数体和运行时 `{ ... }` 块不计入静态容器层级；而无子句限制的 `mod`、`struct`、`enum`、`union` 都是静态容器！）
- **前导 `@path`**：直接从**当前 Package 的根（包根）**开始查找！

让我们画一张 **Code Cartoon**，看看 `.path` 和 `@path` 是如何在模块树和类型容器里精准跳跃的：

```text
  [Package Root (@)]
  ├── struct RootToken {}  ◄─────────────────────────── `use @RootToken;` 直接跳回包根定位！
  ├── mod codec
  │   ├── struct LocalToken {}  ◄────────────────────── `use .LocalToken;` 跳到父容器 codec！
  │   └── mod decoder
  │       ├── use .LocalToken;
  │       └── use @RootToken;
  └── struct Outer
      ├── struct Token {}  ◄─────────────────────────── `use .Token;` 跳到父类型容器 Outer！
      └── struct Inner
          └── use .Token;
```

写成代码清晰无比：

```flurry
struct RootToken {}

mod codec {
    struct LocalToken {}

    mod decoder {
        -- `.` 指向父静态容器 `codec`
        use .LocalToken;
        -- `@` 指向当前包根
        use @RootToken;

        fn make_tokens() -> (LocalToken, RootToken) {
            (LocalToken {}, RootToken {})
        }
    }
}

struct Outer {
    struct Token {}

    struct Inner {
        -- 这里的父静态容器是 `Outer`，而不是外面的文件模块！
        use .Token;

        fn token() -> Token {
            Token {}
        }
    }
}
```

注意：**越过包根 `@` 向上查找是静态错误**（比如在包根写 `use .Something;` 会直接被拒绝）——即使多个包同处于一个工程（Umbrella）下，每个包的包根依然是不可逾越的静态边界！

---

## 第三版：统一命名空间与顺序遮蔽——为什么 `let retry` 会挡住 `:retry` 标签？（v3）

最后，我们来解开两个关乎编译器名称解析底层逻辑的重要设计：**顺序绑定遮蔽（Sequential Shadowing）**与**统一命名空间（Unified Namespace）**。

### 1. 顺序绑定：先在旧环境算右边，再把新绑定放进新环境

在同一个代码块里，当我们连续写两行同名 `let` 时：

```flurry
use std.prelude.*;

test {
    let value: u32 = 20;
    -- 右边的 `value + 1` 在新绑定生效前的旧环境中解析，读到的是上面的 20！
    -- 初始化完成后，新的 `value` (21) 进入后续环境，遮蔽旧的短名！
    let value = value + 1;
    println("{}", value); -- 输出 21
}
```

请注意：在同一个顺序块里写第二个 `let value = ...`，是**建立了一个拥有全新身份的新绑定**，而不是给第一个 `value` 重新赋值！第一个 `value` 虽然失去了短名，但它原有的生命周期和 `Drop` 清理责任依然丝毫不变地保留在原声明槽位上。

### 2. 统一命名空间：名字先确定唯一声明，绝不按位置“跳槽”！

在 Rust 中，编译器在内部维护了三个不同的命名空间：Type Namespace（类型/模块）、Value Namespace（函数/变量）和 Macro/Label Namespace（生命周期与循环标签）。所以在 Rust 里，你写出一个既叫 `foo` 的类型、又叫 `foo` 的变量、再加一个叫 `'foo` 的循环标签，编译器会按语法位置去不同的表里查。

但这种“分表回退查找”在语言演进到了**类型是一等值（Type as Value）**和**编译期元编程**时，会造成严重的语义撕裂！
因此，Flurry 采用了彻彻底底的**统一命名空间（Unified Namespace）**：

> **普通程序名称——无论是类型（Type）、值（Value）、模块（Module）还是循环标签（`:label`）——统统住在同一个命名空间里！**
> **解析规则永远是两步走：第一步，先按词法作用域从内向外找到那个名字对应的唯一声明；第二步，再检查这个声明能不能用于当前语法位置！如果种类不对，直接报错，绝不跳过它去外层找同名项！**

让我们看一个让人恍然大悟的编译期拒绝例：

```flurry
-- ❌ 编译期直接拒绝！
fn invalid_label() {
    -- 外层循环定义了一个名为 `retry` 的循环标签：
    while :retry true {
        -- 内层定义了一个同样名为 `retry` 的局部整数变量，遮蔽了外层的 `:retry`！
        let retry: u32 = 1;

        -- 此时执行 `break retry;`：
        -- 第一步：查找名字 `retry`，在内层找到了局部变量 `retry: u32`！
        -- 第二步：检查 `retry: u32` 能不能作为 `break` 的循环标签目标？不能！
        -- 编译器当场报错，绝不会跳过局部变量 `retry` 去外层偷偷找 `:retry` 标签！
        break retry;
    }
}
```

同理，如果你在函数里写了一个局部变量 `let Window: u32 = 10;`，遮蔽了外层的 `struct Window`，那么在这行之后如果你写 `let w: Window = ...;`，编译器在内层查到 `Window` 是一个 `u32` 变量而不是类型，也会直接报错，绝不会因为冒号后面需要类型就跳过内层变量去外层捞 `struct Window`！

---

## 本篇小结

| 概念与机制 | 语法与规则 | 核心设计哲学 |
| :--- | :--- | :--- |
| **文件与目录映射** | `src/main.fl`（包根）、`foo.fl` / `foo/`（子模块）、`foo/mod.fl`（目录模块主体） | 扫描目录自动建树，无需空 `mod.fl`；严禁同级同名文件、目录与内联 `mod` 隐式合并 |
| **路径定位起点** | 普通名（词法向外）、`.path`（父静态容器）、`@path`（当前包根） | 区分“词法向上可见”与“容器自有成员”；指定容器后只查其成员，绝不回退祖先 |
| **顺序绑定遮蔽** | `let x = x + 1;` | 初始化表达式在旧环境中解析；新绑定建立独立身份，不抹除旧绑定的 `Drop` 责任 |
| **统一命名空间** | 类型、值、模块、`:label` 共处一室 | 先按作用域锁定唯一声明，再检查用途合法性；绝不因种类不符而跨层回退查找 |

---

### 留给下一篇的悬念

现在我们已经把代码分门别类地放进了不同的目录和模块里。
但马上，我们就需要控制**模块之间的进出口海关**了：
1. 为什么在前面的所有例子里，我们在同一个包的不同模块之间调用函数，从来没写过 `pub`，它们却能互相访问？Flurry 的默认可见性到底是什么？`private` 和 `pub` 又分别管多大范围？
2. 当我们用 `use codec.{Packet, version}`、`use first.*`、`use second.*` 导入名字时，如果两个通配符 `*` 都导入了同名的 `Packet`，什么情况下会报歧义，什么情况下（比如菱形导入 `source.Packet`）编译器又能聪明地认出它们是同一个声明而自动合并？
3. 为什么我们在每篇手记的第一行都要写 `use std.prelude.*;`，而像 `u32`、`bool`、`BitVec` 却不用导入就能直接用？Flurry 的**六个内置包（`builtin`、`meta`、`std`、`build`、`attrs`、`verification`）**各自掌管着什么神通？

在下一篇 **《Flurry 演进手记 (039)：精确控制谁能看见什么——`use` 导入、重导出、`private`/`pub` 与六个内置包》** 中，我们将全面掌握 Flurry 的权限控制、重导出图算法与六大内置包全景！
