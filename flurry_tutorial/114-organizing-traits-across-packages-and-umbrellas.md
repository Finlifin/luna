# Flurry 演进手记 (114)：跨包扩展与 Umbrella 架构中的 Trait 组织艺术

当我们的通用零拷贝编解码器从一个单包原型演进为支撑全公司边缘网关的大型基础设施时，代码组织架构立刻升级为了多包协作模式：

```text
gateway_workspace/               <-- Umbrella 工程根目录
├── codec_core/                  <-- 核心包：定义 Encode, Decode, Pair 等基础协议与容器
│   ├── package.fl
│   └── src/lib.fl
├── sensor_a/                    <-- 业务子包 A：定义 A 型传感器 LocalA 及其编解码实现
│   ├── package.fl
│   └── src/lib.fl
├── sensor_b/                    <-- 业务子包 B：定义 B 型传感器 LocalB 及其编解码实现
│   ├── package.fl
│   └── src/lib.fl
└── gateway_facade/              <-- 门面聚合包：通过 pub use 统一向外部重导出常用 API
    ├── package.fl
    └── src/lib.fl
```

在这样真实的多包工程中，团队里的工程师们很快提出了三个直击架构底层的灵魂问题：
1. **Umbrella 边界问题**：“既然 `sensor_a` 和 `codec_core` 放在同一个 Umbrella 工作区目录下，我在 `sensor_a` 里能不能直接调 `codec_core` 没标 `pub` 的内部函数？或者在 `gateway_facade` 里用 `pub use` 重新导出一个类型后，直接替它实现外部 Trait？”
2. **跨包泛型的“菱形相撞”危机**：如果 `sensor_a` 为 `Pair<LocalA, T>` 实现了 `codec_core.Label`，而互不知情的 `sensor_b` 为 `Pair<U, LocalB>` 也实现了 `codec_core.Label`——当顶层网关把 `sensor_a` 和 `sensor_b` 一起链接进来并构造出一个 `Pair<LocalA, LocalB>` 时，岂不是两份实现当场撞车？！编译器该允许谁、拒绝谁？
3. **外部类型适配之道**：如果 Trait 和目标类型（比如 `u32`）都来自外部包，我们在当前包里怎么不污染全局实现世界，又能丝滑地给 `u32` 挂上当前包的专属编码逻辑？

今天，我们就深入 `docs/code-organization/packages-and-umbrella.md` 与 `docs/traits/coherence.md`，揭秘 Flurry 在大型工程中的 **Package / Umbrella 职责边界** 与 **有序本地身份规则（Ordered Local Identity Rule）**！

---

## 尝试 v0：把工作区目录当成“法外之地”的认知误区

在很多使用 Monorepo 或多模块构建系统的项目里，开发者容易产生一种错觉：
“只要几个子目录放在同一个工程（Umbrella）下面，或者我在门面包 `facade` 里写了一句 `pub use protocol.Packet;` / `typealias MyPacket = protocol.Packet;`，那 `Packet` 就算是我们当前包的自己人了，想访问私有函数或随便写外部 `impl` 都行！”

在 Flurry 中，这种企图绕过边界的做法会被编译器和构建系统（`anar` + `luna`）严格制止！

---

## 演进 v1：厘清 Package 与 Umbrella 的铁律——重导出与别名绝不篡改“定义出生证明”！

让我们先通过一张对照表（源自 `docs/code-organization/packages-and-umbrella.md`）看清 Flurry 工程组织的四个基本概念：

| 概念对象 | 核心职责与语义边界 |
| :--- | :--- |
| **`package`（包）** | **语言级定义归属、包内默认可见性访问与依赖接口的唯一边界！** 默认以 `src/lib.fl`（库）或 `src/main.fl`（可执行程序）为包根 |
| **`umbrella`（工作区聚合）** | 组织多个成员包、共享工程配置与构建协调（由构建工具 `anar` 调度并将编译输入交付给编译器 `luna`） |
| **`pub use`（再导出）** | 仅提供新的名称访问路径，**保持原始定义归属不变**！绝不会把外部包的类型变成当前包的本地定义 |
| **`typealias` vs `newtype`** | `typealias` 只是透明别名，保留原类型身份；唯有 **`newtype`** 才会建立归属当前包的全新本地名义身份！ |

这意味着三条在大型工程中至关重要的铁律：

1. **同处一个 Umbrella 不授予特权**：
   即使 `editor` 和 `model` 位于同一个 Umbrella 目录下，`editor` 依然必须显式声明对 `model` 的依赖，且只能访问 `model` 中标记为 `pub` 的公开项（如 `pub fn default_limit()`），绝不能以“邻居关系”调用 `model` 内部的非公开函数 `fn internal_limit()`！
2. **`pub use` 重导出不改变定义包身份**：
   假设 `protocol` 包定义了 `pub struct Packet {}`，而门面包 `facade` 写了 `pub use protocol.Packet;`。当下游同时引入两者时：

```flurry
use protocol.Packet as OriginalPacket;
use facade.Packet as PublicPacket;

comptime {
    -- ✅ 两个名字指向完全相同的名义类型身份！
    -- facade 只是转发了名字，并没有因此成为 Packet 的定义包，
    -- 因此 facade 也不能替 Packet 违规实现其他外部包的 Trait！
    assumes OriginalPacket == PublicPacket;
}
```

3. **不能靠“不写 `use`”来掩盖实现冲突**：
   当前构建的合法依赖图决定了整个**实现世界（Implementation World）**。普通 `use` 只控制源码里的名字可见性；如果依赖图里已经存在两份冲突的 `impl`，你绝不能靠“在当前文件里少写一句 `use`”就把冲突藏起来！

---

## 演进 v2：跨包实现外部 Trait 的黄金法则——有序本地身份规则（Ordered Local Identity Rule）

现在，我们来攻克开头提出的第 2 个硬核难题：
假设基础包 `base` 定义了一个公开接口 `Label` 和一个通用二元组结构体 `Pair`：

```flurry
-- 位于包 `base` 中：
pub trait Label {
    fn label(*self) -> str;
}

pub struct Pair
where A, B
{
    first: A,
    second: B,
}
```

现在，只依赖 `base` 的两个独立业务子包 `pkg_a`（拥有本地类型 `LocalA`）和 `pkg_b`（拥有本地类型 `LocalB`），都想为 `base` 包里的外部泛型容器 `Pair` 实现外部接口 `Label`！
- `pkg_a` 想写：`impl Label for Pair<LocalA, T> where T`
- `pkg_b` 想写：`impl Label for Pair<U, LocalB> where U`

请仔细想一想：如果编译器对这两份代码都放行，会发生什么可怕的后果？
当下游网关同时依赖 `pkg_a` 和 `pkg_b`，并创建一个 `Pair<LocalA, LocalB>` 时：
- 它完美匹配 `pkg_a` 的 `Pair<LocalA, T>`（令 `T = LocalB`）；
- 它也完美匹配 `pkg_b` 的 `Pair<U, LocalB>`（令 `U = LocalA`）！
更致命的是，这两份实现的头部结构精度都是 `Pair<◇, ◇>`（精度同为 1），没有任何一方比另一方更具体！下游程序当场陷入无法消解的二义性爆炸！

为了在**包定义当场**（而不是等到下游合流时才爆雷）就从根源上斩断这种跨包对称撞车，Flurry 在 `docs/traits/coherence.md` 中制定了极其严密、可形式化验证的**有序本地身份规则（Ordered Local Identity Rule）**！

让我们用一张 **Code Cartoon** 看懂编译器是如何像安检扫描仪一样，按**从外到内、从左到右**的顺序扫描实现头的：

```
+-------------------------------------------------------------------------+
|  🎨 Code Cartoon：跨包实现外部 Trait 的“有序本地身份扫描仪”             |
|                                                                         |
|  当你在当前包为【外部 Trait】编写 `impl ExtTrait<...> for Subject` 时： |
|  编译器对归一后的 Subject（随后是 Trait 显式实参）                      |
|  按【从外到内、从左到右】的顺序逐个扫描节点：                           |
|                                                                         |
|  ✅ 通关条件：                                                          |
|     1. 必须遇到至少一个归当前包所有的名义类型（本地身份 LocalType）；   |
|     2. 在到达这【第一个本地身份】之前，绝对不能遇到任何仍可自由替换的   |
|        量化参数（如泛型 T、U）或未决结构！                              |
|     3. 一旦成功抵达第一个本地身份，它内部及后续位置的参数均可自由量化！ |
+-------------------------------------------------------------------------+
|  🔍 扫描案例 1（在 pkg_a 中写 `impl Label for Pair<LocalA, T> where T`）|
|     扫描序列：Pair (外部) ──► 第 1 参数 LocalA (🎉 命中本地身份！)      |
|               ──► 后续第 2 参数 T (已在本地身份之后，允许自由量化！)    |
|     结论：✅ 合法通过！                                                 |
|                                                                         |
|  🚫 扫描案例 2（在 pkg_b 中写 `impl Label for Pair<U, LocalB> where U`）|
|     扫描序列：Pair (外部) ──► 第 1 参数 U (❌ 警报！在遇到本地身份      |
|               LocalB 之前，先撞上了自由量化参数 U！)                    |
|     结论：❌ 在 pkg_b 编译当场直接拒绝！从数学上彻底杜绝与 pkg_a 撞车！ |
+-------------------------------------------------------------------------+
```

让我们把 `pkg_a`（正例）与 `pkg_b`（拒绝例）写成直观的 Flurry 代码对照：

```flurry
-- ==========================================
-- 在包 `pkg_a` 中（仅依赖 `base`）：
-- ==========================================
struct LocalA {}

-- ✅ 编译通过！
-- 从外到内、从左到右扫描 `Pair<LocalA, T>`：
-- 在遇到自由参数 `T` 之前，先遇到了属于当前包的 `LocalA`！
impl Label for Pair<LocalA, T>
where T
{
    fn label(*self) -> str {
        "a"
    }
}
```

```flurry
-- ==========================================
-- 在包 `pkg_b` 中（仅依赖 `base`）：
-- ==========================================
struct LocalB {}

-- ❌ 编译期在 `pkg_b` 定义处直接拒绝！
-- 扫描 `Pair<U, LocalB>` 时，在到达本地身份 `LocalB` 之前，
-- 先遇到了可自由替换的量化参数 `U`！违反有序本地身份规则！
impl Label for Pair<U, LocalB>
where U
{
    fn label(*self) -> str {
        "b"
    }
}
```

看到这条规则的数学美感了吗？
因为在任何一个像 `Pair<A, B>` 这样的具体实例里，“第一个参数位置”只有一个！既然所有包都必须保证在“第一个本地身份之前不能有自由变量”，那么永远不可能有两个不同的包同时在不相交的参数槽位上写出对称匹配的通配实现！

---

## 演进 v3：当 Trait 和底层类型都属于外部包——用 `newtype` + `.as(LocalType)` 零成本破局！

最后一个问题：如果我们在写一个业务包，既想复用外部 `codec_core` 包里的 `Code` Trait，又想给内置的 `u32` 类型提供一套当前业务专属的编码实现——因为 `Code` 和 `u32` 都不是当前包定义的，直接写 `impl Code for u32` 肯定会被包归属规则拒绝。

在传统语言里，你用 `struct LocalWord(u32);` 包装之后，每次调用都得手动构造 `LocalWord(7).code()`。
而在 Flurry 中，我们可以结合 **`newtype`（建立本地名义身份）** 与上一篇学到的 **`u32.as(LocalWord)` 类型标注视角**，实现零入侵、零运行时开销的本地适配！

```flurry
trait Code {
    fn code(*self) -> u32;
}

-- 1. 使用 newtype 建立归属当前包的本地名义身份 LocalWord！
-- （注意：如果用 typealias 则不会建立新身份，无法获得 impl 归属权）
newtype LocalWord = u32;

-- 2. LocalWord 是当前包的本地类型，合法实现 Code 接口！
impl Code for LocalWord {
    fn code(*self) -> u32 {
        16
    }
}

-- 3. 在函数参数类型上使用 `.as(LocalWord)` 视角：
--    调用者直接传普通的 u32（如 7），函数内部直接调 `word.code()`，
--    同时绝不会向全局所有 u32 注入污染性的 Code 实现！
fn show_word(word: u32.as(LocalWord)) {
    println("code: {}", word.code());
}

test {
    show_word(7);
}
```
运行这段代码，直接输出 `code: 16`！调用方传的是最自然的 `7`，定义方拿到了专属的 `LocalWord` 实现，而全局的 `u32` 依然保持纯洁，不会和任何其他第三方库产生 Coherence 冲突！

---

## 本篇小结

| 架构维度 | 核心规则与机制 | 工程价值 |
| :--- | :--- | :--- |
| **`package` vs `umbrella`** | `package` 是归属、可见性与依赖边界；`umbrella` 仅协调多包工程配置与构建 | 保持大型 Monorepo 内部各子包的严格封装，相邻目录不越权访问私有实现 |
| **`pub use` 与 `typealias`** | 重导出与类型别名均保留原定义包身份（`Original == Public`） | 门面包（Facade）可安全聚合导出 API，而不会篡改类型的原始定义归属 |
| **有序本地身份规则** | 外部 Trait 的 `impl` 头从外到内、从左到右扫描，抵达首个本地名义身份前禁含自由参数 | 在子包编译当场阻断 `Pair<LocalA, T>` 与 `Pair<U, LocalB>` 式的菱形撞车 |
| **`newtype` + `.as(...)` 视角** | `newtype Local = Ext;` 建立本地身份，配合 `Ext.as(Local)` 参数视角调用 | 无需污染全局外部类型即可为任意外部类型挂载当前包专属的接口行为 |

---

### 留给下一篇的悬念

激动人心的时刻到了！
从第 096 篇走到第 114 篇，我们已经完整掌握了 Flurry Trait 系统的全套十八般武艺：
- 名义接口声明、关联类型/关联值 `assoc`、默认方法、父接口正向合取 `:- A + B` 与否定约束 `requires not (...)`；
- 统一深度结构精度选择（Structural Precision）与跨包有序本地身份 Coherence；
- 接收者适配三层顺序与 `.as(Trait)` / 类型视角链消歧；
- 开放实现替换推导（Substitution Inference）与 `<.dep = obj>` 运行时环境交付；
- Trait 的 `comptime` / `runtime` 两相、`derive fn` 策略派生、关联错误行 `![Err]` 与 `dyn Trait` 动态接口！

在下一篇——也是《Flurry 演进手记》第五卷的**终极里程碑篇 `115-milestone-universal-codec-framework.md`** 中，我们将把所有这些散落的珍珠串成一条璀璨的项链，组装出一套支持**编译期帧长预计算、自动策略派生、外部校准环境注入与动态插件热切换**的完整通用编解码器框架！准备好迎接第五卷的大结局吧！
