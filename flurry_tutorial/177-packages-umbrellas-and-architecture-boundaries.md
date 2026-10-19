# Flurry 演进手记 (177)：管理大型多包工程——Package、Umbrella 聚合包与最小可见性边界

在[上一篇（第 176 篇）](./176-host-backends-iir-handles-and-artifact-delivery.md)中，我们看到了一个异构计算工程如何通过 `cuda`（管线接口包）、`cuda_compile`（宿主机后端包）和业务包协同完成设备产物的编译与交付。

随着工程规模从“几个源文件”增长到“几十个模块、十几个子包”，几乎每个架构师都会遇到两类令人头疼的工程腐化问题：
1. **多包协作时的边界失控**：为了共享构建配置和依赖版本，我们把多个包放进同一个工作区（在 Flurry 中称为 **`umbrella`**）。但有些工程师误以为“进了同一个 `umbrella` 就是一家人”，试图跨子包直接调用未公开的内部实现，导致子包之间耦合成一团乱麻。
2. **包内重构时的可见性两难**：在同一个包（`package`）内部，我们把代码拆分成 `mod limits` 和 `mod worker` 两个子模块。如果默认可见性是“仅当前模块私有”，那么每次拆分子模块都要到处补写类似 `pub(crate)` 的啰嗦修饰符；如果为了省事全写成 `pub`，这些内部辅助函数又会不小心暴露给外部下游用户！

今天，我们就来系统拆解 Flurry 的代码组织四层模型（`package` / 构建目标 / 构建实例 / `umbrella`）与极其优雅的**三层可见性（Visibility）体系**。

---

## 一、 四层代码组织模型：`package` 与 `umbrella` 的职责铁律

在 Flurry 与构建工具 `anar` 的协作体系中，工程组织被清晰地划分为四个层次（源自 `docs/code-organization/packages-and-umbrella.md`）：

```text
  ┌─────────────────────────────────────────────────────────────────────┐
  │ Umbrella (工作区聚合包: 共享依赖解析、配置面板与多目标任务调度)       │
  │ ⚠️ 铁律：Umbrella 绝不合并成员包的模块树，也绝不突破成员包的可见性边界！│
  │                                                                     │
  │  ┌───────────────────────────────┐ ┌──────────────────────────────┐ │
  │  │ Package A: `protocol`         │ │ Package B: `engine`          │ │
  │  │ (名义类型归属、默认可见性边界) │ │ (依赖 protocol 的公开接口)    │ │
  │  │  ├─ src/lib.fl (库目标根)      │ │  ├─ src/lib.fl (库目标根)     │ │
  │  │  │   ├─ mod limits            │ │  └─ src/main.fl (可执行目标根)│ │
  │  │  │   └─ mod packet            │ │                              │ │
  │  └──┴────────────────────────────┘ └──────────────────────────────┘ │
  └─────────────────────────────────────────────────────────────────────┘
           │ 结合具体平台、特性开关与编译期配置参数
           ▼
  ┌─────────────────────────────────────────────────────────────────────┐
  │ 构建实例 (Build Instance): 例如 protocol (x86_64 + avx2, debug=true) │
  └─────────────────────────────────────────────────────────────────────┘
```

让我们明确这四个概念的工程契约：
1. **`package`（包）**：语义与名义所有权的基本单元。它拥有独立的名称、版本、依赖集合，也是**默认可见性（Package-visible）**和 **`open fn` 默认可扩展范围**的硬边界。
2. **构建目标（Build Target）**：包内的编译单元入口。按照约定，位于包根目录下的 `src/lib.fl` 是库目标根，`src/main.fl` 是可执行目标根（注意：只有包根目录下的 `src/main.fl` / `src/lib.fl` 才是约定根，深层子目录里同名的文件只是普通模块文件，绝不会被误当成包根！）。
3. **构建实例（Build Instance）**：一个构建目标在特定目标平台（如 `x86_64` vs `aarch64`）、特定配置参数下的具体编译实例。代码中读取的 `build` 上下文，观察的就是当前绑定的这一个构建实例。
4. **`umbrella`（聚合包 / 工作区）**：将多个相关的 `package` 组织在一起，统一管理工作区配置与构建调度。
   - **架构红线**：**`umbrella` 不是一个超级大包！** 即使 `protocol` 和 `engine` 处于同一个 `umbrella` 下，`engine` 也只能通过 `protocol` 显式导出的 `pub` 接口访问它，绝不能访问 `protocol` 的默认（包内）声明或 `private` 声明，也不能跨越包边界覆写未标记为 `pub open` 的 `open fn`！

---

## 二、 三层可见性体系：为什么“不写修饰符”就是最舒服的包内协作？

在很多传统语言里，默认可见性是“模块私有（Private to current module）”。这导致只要你在自己的包里把一个 500 行的文件拆成两个子模块，原本能互相调用的内部函数就全都报错了，逼着你满屏写 `pub(crate)` 或 `internal`。

Flurry 做出了一个极其贴合工程直觉的设计：**默认可见性就是当前包可见（`Package(p)`）！**

| 可见性写法 | 语义域表示 | 谁可以访问？ | 典型工程场景 |
| :--- | :--- | :--- | :--- |
| **无修饰符（默认）** | `Package(p)` | **当前 `package` 内的任何模块**（无论父子还是兄弟模块），但对包外绝对隐藏 | 包内各子模块（如 `limits` 与 `worker`）之间自由协作的内部实现 |
| **`private`** | `Descendants(m)` | **仅限定义它的当前模块 `m` 及其子孙模块**；同包的外部兄弟模块也不可见 | 模块内部不想被同包其他同事误碰的底层状态、不变量或局部秘密 |
| **`pub`** | `All`（受路径可达性约束） | **任何拥有可达公开路径（或通过 `pub use` 重导出路径）的包与模块** | 正式对外承诺的跨包公共 API |

### 1. 默认可见性（`Package(p)`）：包内兄弟模块零摩擦协作

看下面这段代码（源自 `docs/code-organization/visibility.md`）：

```flurry
mod limits {
    -- 未写 pub 也未写 private -> 默认在整个当前 Package 内可见！
    fn max_retry() -> i32 {
        3
    }
}

mod worker {
    use super.limits;

    pub fn retry_budget() -> i32 {
        -- ✅ 合法！worker 和 limits 属于同一个 Package，无需把 max_retry 暴露为 pub
        limits.max_retry()
    }
}
```

### 2. `private` 可见性（`Descendants(m)`）：子模块可见，兄弟模块止步

如果某个函数包含了极其敏感的模块内部细节，连同一个包里的兄弟模块都不允许调用，只需加上 `private`：

```flurry
mod service {
    -- 仅限 service 模块及其子孙模块 (如 service.worker) 访问
    private fn secret_limit() -> i32 {
        7
    }

    pub mod worker {
        use super.secret_limit;

        pub fn read() -> i32 {
            -- ✅ 合法！worker 是 service 的子模块，属于 Descendants(service)
            secret_limit()
        }
    }
}

-- ❌ 编译期直接拒绝！
mod sibling_client {
    use super.service;

    pub fn try_steal_secret() -> i32 {
        -- 编译期直接拒绝（Visibility Error）：
        -- `secret_limit` 在 `service` 中声明为 `private`，
        -- 仅对 `service` 及其子孙模块可见，兄弟模块 `sibling_client` 无权访问！
        service.secret_limit()
    }
}
```

---

## 三、 外观模式（Facade）与重导出（`pub use`）的三大铁律

在大型库设计中，我们经常把具体实现放在 `private mod backend` 里，然后在包的根部用 `pub use` 把精选出来的类型和函数导出为干净的顶层 API。

```flurry
-- 1. 将实现模块本身设为 private，外部用户无法直接通过 `pkg.backend.xxx` 访问
private mod backend {
    pub struct Packet {}

    pub fn protocol_version() -> u32 {
        1
    }
}

-- 2. 通过 pub use 将内部声明为 pub 的条目重导出到公开路径！
pub use backend.{Packet, protocol_version};
```

这套“私有模块 + 公开重导出”的组合拳非常强大，但它受三条铁律的严格约束：

### 铁律 1：`pub use` 绝不能放大原声明的可见性上限（No Visibility Widening）
为什么上面 `backend` 里的 `Packet` 必须写成 `pub struct Packet`？
因为一个声明能否通过某条路径被访问，取决于**“原声明自身的可见性”**与**“导出路径的可见性”**两者的交集！如果原声明本身不是 `pub`（比如只是默认的包内可见或 `private`），你试图用 `pub use` 或类型别名把它强行公开到包外，编译器会当场拦截：

```flurry
-- ❌ 编译期直接拒绝！
-- Scratch 没有写 pub，它只是当前包内可见 (Package-visible)
struct Scratch {}

-- 试图通过公开别名或 pub use 把包内私有类型暴露给外部包：
type LocalScratch = Scratch;
pub use LocalScratch as PublicScratch;
--      ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
-- 编译期直接拒绝（Re-export Visibility Ceiling Error）：
-- `pub use` 的可见性不能超过原声明 `Scratch` 的可见性上限（Package-visible）！
-- 绝不允许通过重导出或别名泄漏非公开的内部类型。
```

### 铁律 2：公开 API 签名绝不能泄漏未公开的类型
同样地，如果你写了一个 `pub fn make_scratch() -> Scratch`，而返回类型 `Scratch` 只是一个包内可见的结构体，外部包拿到返回值根本无法合法认识它的类型——编译器也会在公开签名检查中直接拒绝这种“签名泄漏内部类型”的代码。

### 铁律 3：重导出保持类型的名义身份（Nominal Identity）不变
假设 `protocol` 包定义了 `pub struct Packet {}`，另一个 `facade` 包写了 `pub use protocol.Packet;`。
那么在下游用户看来，`protocol.Packet` 和 `facade.Packet` 是两个不同的类型吗？

```flurry
-- 在下游消费包中：
use protocol.Packet as OriginalPacket;
use facade.Packet as PublicPacket;

-- ✅ 它们是完完全全的同一个名义类型！
-- OriginalPacket == PublicPacket
```

- 重导出只是提供了一条新的**可达路径（Accessible Path）**，**绝不会创造一个新的名义类型**。
- `Packet` 的真实定义包（Defining Package）依然是 `protocol`，而不会变成 `facade`！这保证了跨包类型等价性推导和一致性规则（Coherence）绝不会因为重导出而发生分裂。

---

## 本篇小结

| 组织/可见性概念 | 语法与规则 | 核心架构价值 |
| :--- | :--- | :--- |
| **`package` vs `umbrella`** | `package` 是名义与可见性边界；`umbrella` 仅聚合多包配置与构建调度 | 消除工作区内子包之间的隐式越权访问，保持模块树与包边界绝对清晰 |
| **默认可见性（无修饰符）** | `Package(p)`（当前包内所有模块可见，包外不可见） | 包内拆分重构子模块时零摩擦，无需满屏补写 `pub(crate)`，又绝不对外泄漏 |
| **`private` 可见性** | `Descendants(m)`（仅当前模块及其子孙模块可见） | 将模块内部敏感状态与不变量彻底隔离在当前模块子树内 |
| **`pub` 与 `pub use` 重导出** | `private mod backend { pub struct Packet {} }` + `pub use backend.Packet;` | 隐藏复杂的内部模块目录层级，对外呈现扁平优雅的 Facade API，同时严禁放大非 `pub` 声明的可见性 |
| **名义身份守恒** | 跨包 `pub use` 不改变原声明的 Defining Package | 无论通过哪条重导出路径引用，同一声明在类型系统与 Coherence 中始终具备唯一身份 |

---

## 留给下一篇的悬念

我们已经搞清楚了 `package` 和 `umbrella` 的静态组织边界。
但在真实工程构建时，一个 `umbrella` 工作区里可能包含：
- 提供过程宏或编译期代码生成的工具包；
- 需要先运行构建脚本生成源码表的 `protocol` 包；
- 以及依赖 `protocol` 的 10 个上层业务包。

当我们敲下构建命令，只要求构建其中一个子目标时，Flurry 的构建与包管理器 **`anar`** 是怎么工作的？
- `package.fl` 里的编译期配置函数是怎么和代码里的 `build` 上下文配合的？
- 为什么“扫描目录发现脚本”、“编译脚本”和“执行脚本”必须是三个严格分离的阶段？
- 如果某个模块试图一边用宏生成代码，一边又让那个宏依赖当前模块自己解析后的语法树，这种“鸡生蛋、蛋生鸡”的死锁循环是如何被 `anar` 的拓扑调度器当场揪出来的？

下一篇 **[Flurry 演进手记 (178)：Anar 构建调度与编译期脚本——声明式依赖、宏准备与确定性构建缓存](./178-anar-build-scheduling-and-comptime-scripts.md)**，我们将深入 `anar` 与 `luna` 的跨工程协作心脏！
