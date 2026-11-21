# Type ID（不稳定）

> **状态：不稳定（Unstable）**
> 本文档描述正在设计中的 ABI 级 `type_id` 机制，它是后续建立 Flurry ABI 稳定性的前提之一。文档中涉及的哈希算法原语、规范序列化字节格式、属性继承声明协议、`__flurry_stable_since` 校验细节、运行时反射入口及 `!E T` 哨兵编码等均**尚未冻结，也尚未在编译器中接通**；在正式宣布稳定之前，任何跨版本细节都可能随实现推进调整。

## Intro

`type_id` 是跨越**编译会话、包边界与包版本**的 **128 位 `Type` 哈希身份**，属于 **ABI 级概念**。当两个独立编译的二进制产物或跨版本库接口需要在运行时或二进制边界上识别同一个具体类型（例如错误限定类型 `!E T` 的错误分支标签、跨包动态错误传播以及后续的稳定二进制接口）时，编译期名义结构本身不足以独立完成跨版本对齐，需要由一个确定的 128 位 Merkle 摘要承载该类型的身份与表示承诺。

`type_id` 与编译期语义类型身份（见[类型身份与类型关系](type-identity.md)）解决的是不同层次的问题：编译期类型相等检查（`==`、PTC 证明与未完全实例化泛型的身份比较）基于完整的语义项与声明身份，**绝不**通过 128 位哈希值替代语义等式证明；而 `type_id` 专用于具体可交付类型在 ABI 边界上的唯一标识与布局一致性校验。

## 语法

`type_id` 本身不新增表面类型运算符。控制版本锚定通过标准库属性包装函数 `attrs.stable_since("...")`（由 `attrs` 与 `attrs.prelude` 导出，构造 `{ __flurry_stable_since: version }` 元数据对象，见[属性系统](../comptime/attribute.md)）附着在类型定义或构造定义前：

```text
attribute -> ^expr
configuration -> .id expr
```

推荐在声明前使用属性包装器，例如：

```flurry
^attrs.stable_since("1.0.0")
pub struct Example { ... }
```

后文出现的带版本路径记号：

```text
std@"1.0.0".collections.Vec<u32, .A = DefaultAllocator, .allocator = DefaultAllocator.GLOBAL.ref>
```

用于描述编译器计算 `type_id` 时构造的**规范化哈希输入表示**，而非要求用户在源码中写出 `@"1.0.0"` 这类路径语法。

## 正式定义

### 128 位 ABI 身份与前置条件

对每个目标平台下可交付 ABI 表示的具体类型，`type_id` 确定性地映射为一个 **128 位整数哈希值**：

1. **跨会话与跨编译单元确定性**：结果只依赖规范类型身份、规范包路径与有效版本、目标 ABI 布局及传递符号依赖，不得依赖 `HirId`、`OwnerId`、`DefId`、进程内存地址、符号枚举顺序或单次编译的文件扫描次序。
2. **具体化前置条件**：只有完成类型实例化、具备明确声明来源及目标布局的具体类型（含已填入有效全局环境身份的类型实例）才有定义良好的跨会话 `type_id`；开放泛型形参或尚未解决的推断变量不拥有跨版本 ABI `type_id`。

### Merkle 树哈希输入（三类必需信息）

`type_id` 按 Merkle 树结构递归哈希计算。以下三类信息**必须全部**进入类型自身的 Merkle 哈希输入：

| 输入分量 | 必须包含的内容 | 作用 |
|---|---|---|
| **完整规范路径与有效版本**（Full Path） | 包名、有效版本（默认当前包版本；若标有 `__flurry_stable_since` 则取其指定版本）、规范模块路径、声明名及全部规范化实参（类型实参、comptime 值实参、提升的运行时依赖来源路径） | 区分来源包、版本归属、模块名义身份与参数化实例 |
| **目标 ABI 布局**（Layout of Type） | 针对目标平台的尺寸、对齐、表示策略（默认、`packed.byte`、`packed.bit`（含逻辑位宽 `W` 与字段位偏移）、`C`、定宽枚举位宽、Tagged Union 标签/载荷格式、指针/BitVec 宽度）、物理字段偏移、变体判别值与载荷布局 | 确保底层内存布局发生任何变化时 `type_id` 自动改变 |
| **全部符号依赖**（Every Symbol Dependency） | 字段类型、枚举变体载荷、基元或包装底层类型、类型/值实参、normal-form 头及关联约束符号的递归 `type_id` / 规范身份 | 形成完整 Merkle 树，使任意深层依赖的改动沿树向上传递 |

补充规则如下：

- **典型示例路径**：如 `std@"1.0.0".collections.Vec<u32, .A = DefaultAllocator, .allocator = DefaultAllocator.GLOBAL.ref>`，同时编码了发布包 `std` 的有效版本 `"1.0.0"`、规范路径 `collections.Vec`、类型参数 `u32`、命名类型参数 `.A = DefaultAllocator` 与规范环境参数 `.allocator = DefaultAllocator.GLOBAL.ref`，并与当前目标下 `Vec` 的真实布局及依赖 Merkle 子树共同参与摘要。
- **`typealias` 与 `newtype`**：`typealias` 在语义与 ABI 上均完全透明，计算 `type_id` 时消解到其目标类型（共享同一 `type_id`）；`newtype` 保留自身的完整规范路径与声明身份作为独立节点，同时将底层类型的 `type_id` 与布局作为 Merkle 子依赖纳入，因而具有独立的 `type_id`。
- **运行时依赖参数**：全局可命名的提升（lift）来源（例如 `DefaultAllocator.GLOBAL.ref`）具有规范模块路径，可形成跨包、跨会话稳定的 `type_id`；局限于某次调用栈帧、堆对象或局部引用的运行时依赖对象仅在当前作用范围依对象身份区分（见[运行时依赖类型](runtime-dependent-type.md)），不自动授予跨编译会话的静态路径身份。
- **递归类型的确定性终止**：对于指针自引用或相互递归的名义类型（例如 `struct Node { value: u32, next: ?*Node }`），按固定先序访问栈将环回边编码为规范环回深度/回指索引（遵循“指针不沿 pointee 无限展开本体”的原则，同时保留环拓扑结构），保证任意合法递归类型都在有限步内得到确定性哈希。

### `attrs.stable_since` 与 `__flurry_stable_since: str` 属性（不稳定）

`attrs.stable_since(version: str) -> Object` 将底层键 `__flurry_stable_since: str` 封装为标准属性入口（`__flurry_stable_since` 仍是属性 Object 的内部键名；具体参数校验与合法版本格式检查仍在演进中）：

- 若 `Vec` 标注了 `^attrs.stable_since("1.0.0")`，即便随后在 `std` 的 `"1.2.0"` 版本中编译，其进入 `type_id` 计算的规范路径前缀仍固定为 `std@"1.0.0".collections.Vec`，而不再取 `"1.2.0"`。
- **四元联合稳定契约（安全兜底）**：固定 `attrs.stable_since` **并不意味着**强制冻结错误旧值。新旧两版包中的同一类型取得相同 `type_id`，**当且仅当**以下四项同时成立：
  1. `__flurry_stable_since` 指定的有效版本字符串未改变；
  2. 规范包名、模块路径与类型声明名未改变（重导出别名不改变原声明的规范路径）；
  3. 该具体实例在目标平台下的 ABI 布局（大小、对齐、偏移、标签/载荷编码）未改变；
  4. 该类型全部符号依赖（包括所有字段、变体载荷及类型实参的 `type_id`）未改变。
- 若 `"1.2.0"` 中即便保留了 `^attrs.stable_since("1.0.0")`，但意外增删了字段、调整了对齐或修改了内部依赖类型导致 `Layout` 或依赖子树变化，Merkle 树会立即产出全新的 `type_id`，从而在 ABI 边界拒绝误把不兼容表示当作同一类型解释。

### Normal-Form 定义到类型实例的属性自动传递（不稳定）

附着在 **normal-form 定义**（例如参数化 `struct`/`enum`/`newtype`，或带 `pure comptime fn<...> -> Type` 签名的 normal-form 类型构造器）上的 `attrs.stable_since`（即 `__flurry_stable_since`）属性，**会自动传递到由它构造出的具体类型实例上**。

例如当 `^attrs.stable_since("1.0.0")` 标在 `Vec` 定义上时，`Vec<u32, ...>` 等所有构造实例都会自动继承该稳定版本锚点，而无需在每处实例化时重复标注。

需要特别说明的是：**并不只有 `__flurry_stable_since` 需要这种特性，标准库与编译器中还有相当多的属性（如布局提示、ABI 策略、诊断/导出元数据等）都需要“从 normal-form 定义自动传递到其构造实例”的行为**。因此编译器不能把 `__flurry_stable_since` 写成孤立特判，而应当在通用属性系统中提供规范的定义-到-实例传播机制，并与[属性系统](../comptime/attribute.md)既有的由内向外合并优先级兼容（实例局部覆写优先于定义级继承属性）。

### 与错误限定类型 `!E T` ABI 的关系（不稳定）

在错误限定类型 `!E T`（见[错误限定类型](qualified-types/error-type.md)与[错误处理](../algebraic-effect-and-delimited-continuation/error-handling.md)）的 ABI 设计中：

- **`0` 保留给成功分支（`Ok`）**：判别值 `0`（128 位 `0`）专用于标识 `!E T` 的正常成功结果；
- **非零 128 位 `type_id` 标识错误分支**：所有合法具体类型的 128 位 `type_id` 必须**非零（`!= 0`）**，从而确保任何错误类型 `E_i` 的标签都不会与 `Ok` 分支冲突（即便同型返回 `![T] T` 也可无歧义区分成功与错误）。具体哈希原语与零哈希避让映射规则标为不稳定直至实现冻结。

采用 128 位全局自描述 `type_id` 作为 `!E T` 标签（配合 `0 = Ok` 保留规则）后，跨错误行嵌入、多态错误透传与跨包版本 ABI 调用无需依赖集中式或按位置重映射的局部 tag 表，错误值本身即可在机器表示中无歧义地保留成功/失败状态与错误类型身份。

## 常见示例

> 示例仅展示设计意图；目前 `attrs.stable_since("...")` 仅可构造并检查 `__flurry_stable_since` 属性对象，编译器尚未接通 `__flurry_stable_since` 消费者与通用 normal-form 属性传递执行，请勿当作当前可运行特性。

### 跨版本锚定与布局安全兜底

```flurry
-- 假定所在包 std 当前版本为 "1.2.0"
-- 因标注了 ^attrs.stable_since("1.0.0")，规范路径部分仍锚定为 std@"1.0.0".collections.Vec
^attrs.stable_since("1.0.0")
pub struct Vec
where
    T: Type,
    .A: Type = DefaultAllocator,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    ...
}
```

- 在 `std@"1.0.0"` 与 `std@"1.2.0"` 之间，若 `Vec<u32, .A = DefaultAllocator, .allocator = DefaultAllocator.GLOBAL.ref>` 的字段布局与依赖（如 `u32`、`DefaultAllocator` 布局）保持完全一致，则两者产出的 128 位 Merkle `type_id` 相同。
- 若后续某次改动导致 `Vec` 新增了一个字段或改变了内存对齐，即便源代码中仍留有 `^attrs.stable_since("1.0.0")`，由于布局输入与符号依赖发生变化，`type_id` 依然改变，防止错误兼容。

### Normal-Form 定义属性向实例自动传递

```flurry
^attrs.stable_since("1.0.0")
pub pure comptime fn TaggedBuffer<T: Type> -> Type {
    struct Impl {
        len: usize,
        ptr: *T,
    }
    Impl
}

-- 实例 TaggedBuffer<u32> 自动继承定义上的 __flurry_stable_since: "1.0.0"
typealias U32Buf = TaggedBuffer<u32>;
```

`TaggedBuffer` 上的 `__flurry_stable_since`（以及遵循同一传播机制的其他定义级属性）会在实例构造完成后自动附着或传递到 `TaggedBuffer<u32>` 对应的实例描述中，供 `type_id` 摘要与下游 ABI 消费者读取。

## 与 Rust / Zig / C / C++ 的关键差异

| 语言机制 | 对应能力 | 与 Flurry `type_id`（不稳定）的核心差异 |
|---|---|---|
| Rust `std::any::TypeId` | 提供编译单元内的类型唯一标识 | Rust `TypeId` 明确声明**不跨编译器版本与编译构建稳定**，且不包含布局或包版本 Merkle 校验契约。[Rust TypeId](https://doc.rust-lang.org/std/any/struct.TypeId.html) |
| C++ `std::type_info` | 依赖编译/链接产物生成 RTTI 描述对象 | 比较依赖实现与符号可见性，不构成带布局指纹与版本锚定开关的 128 位 Merkle ABI 摘要。[C++ typeinfo](https://eel.is/c++draft/type.info) |
| Zig `comptime type` | `type` 为纯粹编译期一等值 | 没有内建的跨版本 128 位布局 Merkle 摘要及属性自动传递契约。[Zig comptime](https://ziglang.org/documentation/0.15.2/#comptime) |
| C | 无语言级运行时/ABI 类型 ID | 跨库结构兼容性完全依靠手工头文件约定与手写版本号，无编译器计算的布局签名 |

## 运行时实现设计引导

在具体设计稳定前，运行时实现应遵守以下隔离原则：

1. **只消费确定摘要，不反向解析哈希**：运行时可比较 128 位 `type_id` 是否相等（如 `!E T` 错误分支匹配或未来动态边界分派），但不得假设能够从 128 位整数逆推出字段表、构造器或执行编译期检查。
2. **保留 `0` 作为 `Ok` 成功判别标识，确保错误标签非零**：`0` 固定保留给 `!E T` 的成功（`Ok`）分支，所有具体类型的 `type_id` 均不得取 `0`（例如遇到零摘要时按规范策略映射到非零值），使运行时无需额外状态位即可区分 `Ok` 与任何错误 `type_id`。
3. **当前未冻结的运行时/接口边界**：
   - exact 128 位哈希算法与域分隔常量（Domain Separation Tag）；
   - 偶然全零哈希映射到非零值的确定性避让规则；
   - 是否向用户暴露 `Type.id_of(T)` 等反射读取入口及其阶段（comptime 确定值 vs 允许传入 runtime 常量）；
   - 跨包动态加载或版本化 ABI 描述表的具体二进制格式。

## 编译器实现引导

当前处于不稳定文档与分层预备阶段，尚未接通部分明确包括：

- **包版本元数据接入**：驱动与包图加载层需要向 NIR 暴露每包的规范包名、当前语义版本与有效导出路径。
- **通用 Normal-Form 属性传播机制（Elaboration / Attrs）**：在属性系统中为需要继承到实例化产物的属性提供统一宣言或传递机制，覆盖结构定义、newtype、enum 以及 normal-form 类型构造器产出的实例，并与实例局部位属性合并顺序一致。
- **规范化 Merkle 序列化与计算服务（NIR）**：应在 NIR 表示与布局服务（参考 [反射与布局](reflection-and-layout.md)）之后建立单一 `type_id` 计算入口：
  1. 解析并规范化 `FullPath`（包名 + `__flurry_stable_since` 或包版本 + 规范模块路径 + 声明名 + 规范实参列表与全局 lift 来源）；
  2. 请求目标平台确定后的 `LayoutPlan`（含大小、对齐、表示策略、字段偏移、枚举判别值与对齐细节）；
  3. 沿字段、变体载荷、基元依赖与符号依赖递归求取子 `type_id`，并在访问栈中检测并确定性编码递归回边；
  4. 使用领域分隔的规范字节流生成 128 位哈希，严禁在相同概念路径下因 AST/HIR 顺序或非规范展示格式产生抖动。
- **ABI 下游消费者（Host / LLVM）**：在以上前提就绪后，将错误限定类型 `!E T`（及其传播、捕获、行嵌入与重映射逻辑）从局部位置索引切换为 `0`（`Ok`）+ 非零 128 位 `type_id`（`Error`）标签表示。

---

**当前总体状态总结**：本规范处于**不稳定**状态；`library/attrs` 已提供 `attrs.stable_since("...")` 包装构造器（生成 `{ __flurry_stable_since: ... }`），但尚未确定哈希原语与序列化字节编码、尚未实现通用 normal-form 属性传递机制，也尚未将包版本元数据与 `!E T` 128 位 `type_id` 标签迁入 NIR 与 LLVM 后端。
