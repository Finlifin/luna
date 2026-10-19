# Flurry 演进手记 (104)：告别黑盒过程宏派生——用纯语言级 derive fn 编写类型安全的自动派生器

> **系列导读**：在上一期（103）中，我们了解了参数 Elaboration 与收集协议如何帮我们消除调用端的繁琐样板。今天，我们要解决**定义端**最大的样板代码痛点——**接口自动派生（Trait Derivation）**！在日常开发遥测与张量引擎时，我们有成百上千个结构体需要实现 `FormatLabel`、`Serialize`、`Eq`、`Hash` 等接口。为什么在其他语言中写一个自定义 `#[derive(...)]` 过程宏如此痛苦？Flurry 又是如何用一等公民语法 **`derive fn`** 将自动派生彻底拉回纯语言级、类型安全的轨道上的？让我们马上揭晓！

---

## 一、传统过程宏的痛点：版本 `v0` 的“无类型 Token 拼接地狱”

如果你在 Rust 中写过自定义的 `#[derive(MySerialize)]`，你一定经历过以下四重折磨：
1. **工程割裂**：你不能在当前代码文件里直接写派生器，必须专门新建一个独立的 `proc-macro` Cargo 包；
2. **丧失类型与语义视角**：你的派生宏拿到的只是一串语法记号流（`TokenStream`）。你根本不知道某个字段的类型究竟实现了什么接口、它的内存对齐是多少，只能像做字符串拼接一样盲目生成代码；
3. **参数传递靠发明“属性黑话”**：如果你想给派生器传一个配置参数，你得自己解析 `#[my_trait(prefix = "foo", rename_all = "camelCase")]` 这种非标准属性语法；
4. **报错犹如天书**：一旦派生生成的代码不满足某个接口契约，编译器报错往往直接糊在 `#[derive(...)]` 那一行宏调用上，调试时连生成了什么代码都看不见！

在 Flurry 中，设计者问了一个直击灵魂的问题：
> **既然 Flurry 已经拥有了强大的双层类型理论（2LTT）和编译期类型反射，为什么“生成一个 Trait 实现”不能直接写成语言里的一等公民函数？**

---

## 二、迭代演进 `v1`：用 `derive fn` 编写第一个一等公民派生策略

在 Flurry 中，自动派生器（Deriver）被正式定义为一种名为 **`derive fn`** 的编译时实现生成策略，而应用派生则使用独立的 **`derive Trait for Type`** 声明！

让我们看看它们清晰无比的语法结构：

```text
deriver_def -> derive fn id (parameter*)? for expr clauses? { (assoc_decl | definition | statement)* }
derive_def  -> comptime? derive expr for expr clauses?
```

在 `derive fn` 的花括号内部，你写的不是字符串拼接，也不是晦涩的语法树节点构造，而是**直接就像写普通 `impl` 一样，写出目标 Trait 所需的关联项（`assoc`）和方法（`fn`）**！在其中，目标主体类型自动绑定为 **`Self`**。

让我们先写一个最直观的 `v1` 示例：为参数化格式标签接口 `FormatLabel<format>` 编写一个名为 `json_label` 的派生器，并用 `.use(json_label)` 应用到结构体 `Record` 上：

```flurry
-- 1. 声明一个以 Symbol 为参数的格式标签接口
trait FormatLabel
where format: Symbol
{
    fn label(*self) -> str;
}

-- 2. 用 derive fn 编写名为 json_label 的派生策略！
-- 它直接针对完整接口实例 FormatLabel<.json> 生成方法
derive fn json_label for FormatLabel<.json> {
    fn label(*self) -> str {
        return "json";
    }
}

struct Record {
    id: u32,
}

-- 3. 使用 derive ... .use(策略名) for 主体类型 应用该派生器！
derive FormatLabel<.json>.use(json_label) for Record;

test {
    let record = Record { id: 7u32 };
    -- 调用派生出的 FormatLabel<.json> 实现！
    println("{}", record.label<.json>()); -- 输出：json
}
```

### ⚠️ 核心阶段辨析：生成器在编译期运行 $\neq$ 生成的方法在编译期运行！

初学者第一次看到 `derive fn` 时，很容易产生一个疑问：
“既然 `derive fn` 是在编译期执行的生成器，那它里面写的 `fn label(*self) -> str` 会不会变成 `comptime fn`？”

**绝对不会！** 请牢记这条黄金法则：
> **Deriver 的生成逻辑运行在编译期（Comptime），但普通 `derive` 生成出来的依然是 Trait 的默认运行期相（Runtime Phase）实现！**
> 只有当你显式写 **`comptime derive ...`** 时，才会针对接口中带有 **`^comptime_phase`** 属性的成员生成 Comptime 相的实现（等同于生成 `comptime impl`）。

---

## 三、迭代演进 `v2`：带编译期配置参数的 `derive fn` 与结构反射

如果我们的遥测系统需要为不同的结构体派生自定义的标签前缀或配置文本，在 Flurry 里还需要像 Rust 那样去解析字符串属性宏吗？

**完全不需要！** 因为 `derive fn` 本身就是一个函数，它可以**直接声明形参**！
`derive fn` 的形参遵循与 `comptime fn` 完全相同的规则：这些参数作为**编译期输入**服务于实现代码的生成过程，而**不会**变成生成的每个运行期方法的额外运行期参数！

```flurry
-- 带形参的派生器：text 是生成器的编译时输入！
derive fn label_from_text(text: str) for FormatLabel<.text> {
    -- 生成的 label 方法签名依然是 fn(*self) -> str，
    -- 调用 record.label<.text>() 时不需要再传一次 text！
    fn label(*self) -> str {
        return text;
    }
}

struct SensorNode {
    channel: u32,
}

-- 在 .use(...) 中直接像调普通函数一样传入编译期参数 "sensor-node-v1"！
derive FormatLabel<.text>.use(label_from_text("sensor-node-v1")) for SensorNode;

test {
    let node = SensorNode { channel: 1u32 };
    println("{}", node.label<.text>()); -- 输出：sensor-node-v1
}
```

### Code Cartoon：`derive fn` 结合结构反射与 `inline for` 的工作流

当我们要为任意结构体自动派生字段级校验或二进制编码（如 `WireSize` 计算总字节数或逐字段编码）时，`derive fn` 可以直接结合我们在第四卷学过的编译期类型反射（读取 `Self` 的字段列表）与 `inline for` 编译期循环展开：

```text
[ derive WireSize.use(sum_fields) for TelemetryPacket ]
                        │
                        ▼
        ┌───────────────────────────────────────────────┐
        │ 在编译期实例化 derive fn sum_fields 上下文：  │
        │   Self = TelemetryPacket                      │
        │   通过合法反射观察 Self 的字段列表：          │
        │     - field #0: timestamp (u64)               │
        │     - field #1: reading   (i32)               │
        │     - field #2: status    (u8)                │
        └───────────────┬───────────────────────────────┘
                        │ 生成标准 Implementation AST / IR
                        ▼
        ┌───────────────────────────────────────────────┐
        │ 送入普通 impl 全套检查器（零特权！）：        │
        │   1. 包归属检查（Orphan Rules）               │
        │   2. 全局一致性与重叠检查（Coherence）        │
        │   3. 关联项、方法签名、前后置契约与内存效果   │
        └───────────────┬───────────────────────────────┘
                        │ 检查全部通过！
                        ▼
        [ 登记为正式的 WireSize for TelemetryPacket 实现 ]
```

让我们看一段结合 `where` 子句约束的 `derive fn` 示例：

```flurry
trait WireTag {
    assoc tag_code: u32;
    fn wire_tag(*self) -> u32;
}

-- 派生器可以自带 where 子句，要求目标主体 Self 或输入参数满足特定契约！
derive fn fixed_wire_tag(code: u32) for WireTag
where requires code > 0u32
{
    assoc tag_code: u32 = code;

    fn wire_tag(*self) -> u32 {
        return Self.as(WireTag).tag_code;
    }
}

struct GyroPacket {
    x: i32,
    y: i32,
}

-- ✅ 传入的 42u32 满足 requires code > 0u32，成功生成关联常量与方法！
derive WireTag.use(fixed_wire_tag(42u32)) for GyroPacket;

-- ❌ 若写 derive WireTag.use(fixed_wire_tag(0u32)) for GyroPacket;
-- 编译期直接拒绝！违反派生器自身的 requires code > 0u32 前置契约！
```

---

## 四、铁面无私的守门人：为什么 `derive` 绝不是绕过检查的“后门”？

在很多支持宏的语言里，宏生成的代码常常享有某种“法外特权”，或者报错位置混乱不堪。
而在 Flurry 中，有一条铁打的原则：
> **派生器（Deriver）不能以“生成代码”的名义跳过任何一项普通 `impl` 检查！生成物必须先通过包归属（Orphan Rules）、重叠冲突（Coherence）、关联项完整性、方法契约（`requires`/`ensures`/`outcomes`）以及阶段（Phase）的全套验证，才能成为可被依赖的事实！**

举个例子：如果某个 `derive fn` 粗心地写错了返回类型，或者漏写了一个没有默认值的必需方法，编译器在 `derive Trait for Type` 处会**同时展示三个精确的诊断锚点**：
1. `derive` 的使用点；
2. 所选 `derive fn` 策略的定义点；
3. 生成物中未能满足契约的具体成员！

而且，一旦派生检查失败，编译器会立即干净地回退，**绝不在符号表里留下“半份残缺的实现”**去引发后续连环报错。

---

## 五、本节核心概念速查表

| 语法 / 概念 | 作用与写法 | 核心语义与安全边界 |
| :--- | :--- | :--- |
| `derive fn name(...) for Trait` | 定义一个编译期实现生成策略 | 主体类型在内部作为 `Self` 使用；形参遵循 `comptime fn` 规则 |
| `derive Trait.use(strat) for T;` | 显式指定策略为类型 `T` 派生实现 | 复用 `.use(...)` 语法接入策略，无需发明额外宏属性语法 |
| `comptime derive Trait for T;` | 派生 Trait 的 **Comptime 相**实现 | 普通 `derive` 生成 Runtime 相实现，`comptime derive` 生成 Comptime 相实现 |
| **产物零特权原则** | 派生产物等同于手写 `impl` | 必须通过包归属、Coherence 重叠、关联项与方法契约的全套检查 |

---

## 六、下一期预告

在今天的例子里，我们每次调用 `derive` 时都显式写了 `.use(json_label)` 或 `.use(fixed_wire_tag(42u32))`。
但在日常开发中，像 `derive Eq for Record;`、`derive FormatLabel<.text> for Record;` 这样最常用的接口，每次都要手写 `.use(default_eq_deriver)` 也太啰嗦了！

反过来，对于同一个 `Serialize<.json>` 接口，有时我们希望大多数结构体使用**默认策略**（按字段名序列化），而在少数性能敏感的结构体上显式切换为**具名策略**（如 `.use(compact_array_repr)` 按紧凑数组序列化）！

在 Flurry 中：
- 如何用 **`^attrs.derivation.default`**（或通过 `use attrs.derivation.default as ...` 取别名）将一个 `derive fn` 注册为某个接口的默认派生策略？
- 哪些包有资格注册默认派生策略？
- 如果一个人先后用**两个不同的策略**（比如 `json_label` 和 `compact_json_label`）为同一个 `Record` 派生了两次 `FormatLabel<.json>`，编译器会因为“策略名字不同”而放行吗？

下一期 **《Flurry 演进手记 (105)：具名派生策略与默认策略——^attrs.derivation.default 与 derive Trait for Type》**，我们将完成本批次的压轴拼图！
