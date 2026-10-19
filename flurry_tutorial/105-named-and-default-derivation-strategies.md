# Flurry 演进手记 (105)：具名派生策略与默认策略——^attrs.derivation.default 与 derive Trait for Type

> **系列导读**：在上一期（104）中，我们用一等公民语法 `derive fn` 告别了黑盒过程宏，实现了纯语言级、类型安全的自动派生器，并通过 `derive Trait.use(strategy) for Type` 显式调用它。但在大型工程中，我们既希望 90% 的常规场景只需写一句极简的 `derive Trait for Type;`（自动选用默认策略），又希望在 10% 的定制场景下能通过 `.use(custom_strategy)` 灵活切换编码格式（比如在紧凑二进制、人类可读文本或按位表示之间切换）。今天，我们将完整拆解 Flurry 的**默认派生策略注册（`^attrs.derivation.default`）**、**显式与默认策略的选择法则**，以及**策略身份与实现一致性的严格边界**！

---

## 一、单一默认宏的窘境：版本 `v0` 的“要么全手写，要么全一刀切”

在传统语言的派生体系（`v0`）中，一个 Trait 往往只能绑定**唯一一个**同名的派生宏（例如 `#[derive(Serialize)]`）。
这在实际工程中会带来极大的不便：
- 假设在我们的遥测引擎里，`Serialize<.json>` 有两种截然不同的实现策略：
  1. `by_fields`：把结构体按 `{ "id": 7, "temp": 36 }` 的标准键值对象序列化（适合大多数业务日志）；
  2. `by_compact_tuple`：把结构体按 `[7, 36]` 的无键名紧凑数组序列化（适合高吞吐传感器遥测流）。
- 在只有单一派生宏的语言里，你不得不在结构体上挂满各种专用的辅助宏属性（如 `#[serde(serialize_with = "...")]`），甚至为了换一种策略而再去包一层代理结构体！

而在 Flurry 中，**接口（Trait）与生成该接口实现的策略（Deriver Strategy）是彻底解耦的一对多关系**！同一个接口可以拥有任意多个具名策略，并且可以指定其中一个作为**默认策略**。

---

## 二、迭代演进 `v1`：用 `^attrs.derivation.default` 注册默认策略与属性别名

在 Flurry 中，想要把某个 `derive fn` 标记为对应接口实例的默认派生策略，只需在它前面加上内置的 `attrs` 包提供的属性：**`^attrs.derivation.default`**！

### 💡 名称解析小贴士：属性位置没有特殊魔法，直接支持 `use ... as` 别名！

在 Flurry 的 6 个内置包（`builtin`、`meta`、`std`、`build`、`attrs`、`verification`）中，`attrs` 专门提供语言级的属性构造接口。
需要特别注意的是：**前缀属性 `^expr` 中的 `expr` 采用与普通代码完全相同的名称解析规则**！编译器不会在 `^` 后面搞特殊作弊去优先搜索 `attrs`。因此，你既可以写完整路径 `^attrs.derivation.default`，也可以像导入普通符号一样，用 `use attrs.derivation.default as default_strategy;` 给它起一个简短的别名！

让我们看一段同时展示**显式具名策略（`.use(...)`）**与**默认策略（`^attrs.derivation.default`）**的完整示例：

```flurry
-- 也可以取别名：use attrs.derivation.default as default_strategy;

trait FormatLabel
where format: Symbol
{
    fn label(*self) -> str;
}

-- 策略 1：具名策略 json_label（未标记 default，需通过 .use(json_label) 显式选择）
derive fn json_label for FormatLabel<.json> {
    fn label(*self) -> str {
        return "json";
    }
}

-- 策略 2：使用 ^attrs.derivation.default 标记为 FormatLabel<.text> 的默认派生策略！
^attrs.derivation.default
derive fn text_label for FormatLabel<.text> {
    fn label(*self) -> str {
        return "text";
    }
}

struct Record {
    id: u32,
}

-- A. 显式选择具名策略：使用 .use(json_label)
derive FormatLabel<.json>.use(json_label) for Record;

-- B. 省略 .use(...)：编译器自动检索并命中 FormatLabel<.text> 的唯一默认策略 text_label！
derive FormatLabel<.text> for Record;

test {
    let record = Record { id: 7u32 };
    -- FormatLabel<.json> 与 FormatLabel<.text> 是两个不同的完整接口实例，和平共处！
    println("{}", record.label<.json>()); -- 输出：json
    println("{}", record.label<.text>()); -- 输出：text
}
```

让我们再用别名写法演示一次干净的默认策略注册：

```flurry
use attrs.derivation.default as default_strategy;

trait Code {
    fn code(*self) -> u32;
}

struct Marker {}

^default_strategy
derive fn constant_code for Code {
    fn code(*self) -> u32 {
        return 7u32;
    }
}

-- 自动使用标记了 ^default_strategy 的 constant_code 策略
derive Code for Marker;
```

---

## 三、深度揭秘：策略选择的三大安全边界与孤儿规则

为了让默认策略与具名策略在大型多包工程中依然保持 100% 的可预测性，Flurry 制定了三条极其严密的选择与归属规则：

### Code Cartoon：编译器解析 `derive` 声明的决策树

```text
                [ derive TraitExpr for TypeExpr; ]
                                │
                 是否带有显式的 .use(strategy) ？
                       /                 \
                  (是)                     (否)
                   ▼                         ▼
    [ 仅检查所指明的 strategy ]      [ 在可见世界中检索默认策略 ]
    若不适用或契约失败：             必须存在【唯一适用】的默认策略！
    ❌ 直接报错！                    （若存在多个适用默认策略或仍未决：
    （绝不偷偷回退到默认策略！）      ❌ 直接报错！绝不按导入顺序猜！）
                   \                         /
                    ▼                       ▼
          [ 实例化策略体，生成标准 impl 产物并执行全套 Coherence 检查 ]
```

### 边界 1：谁有资格注册 `^attrs.derivation.default`？（默认策略的本地归属规则）

试想一下：如果允许任何一个第三方业务包随便给标准库的 `Eq` 或外部包定义的 `Serialize<.json>` 注册一个全局默认派生策略，那么当下游同时引入两个业务包时，`derive Serialize<.json> for MyType;` 就会因为出现两个默认策略而瞬间瘫痪！

因此，Flurry 规定：**注册默认策略（`^attrs.derivation.default`）必须具备本地定义依据！**
编译器会按照我们在第 099 期学过的有序观察法则，按声明头和实参次序检查 `derive fn` 所针对的完整 Trait 表达式：
- 如果该 Trait 本身是当前包定义的本地 Trait，可以直接注册默认策略；
- 如果该 Trait 来自外部包，则在遇到**第一个本地名义身份（比如本地定义的格式类型参数）之前，绝不能出现自由量化参数**！
- 至于不加 `^attrs.derivation.default` 的普通具名策略（Named Strategy），则可以公开导出供其他包通过 `.use(strategy)` 显式选用（但生成的最终产物依然要在调用包通过普通的包归属检查）。

---

## 四、终极演进 `v2`：策略只决定“怎么生”，绝不创造“第二层方法身份”！

这是理解 Flurry 派生机制最核心的一道分水岭：
> **策略身份（Strategy Identity）只存在于编译期代码生成阶段，绝不进入生成物的接口身份（Interface Identity）！**

这句话是什么意思？
1. **调用方无需记住策略名**：无论 `Record` 的 `FormatLabel<.json>` 是手写 `impl` 的、是用默认策略派生的、还是用 `.use(json_label)` 派生的，生成的实现都是纯粹的 `FormatLabel<.json> for Record`。调用方永远只需要调 `record.label<.json>()`，不需要知道背后是哪个策略生成的！
2. **用两个不同策略派生同一个接口实例 = 同精度重叠冲突！**

有些读者可能会误以为：“既然我定义了两个不同名字的策略 `json_label` 和 `compact_json_label`，那我能不能给同一个 `Record` 分别用这两个策略各 `derive` 一次 `FormatLabel<.json>`？”

**绝对禁止！** 让我们看看编译器是如何当场拦截这种重叠冲突的：

```flurry
trait FormatLabel
where format: Symbol
{
    fn label(*self) -> str;
}

derive fn json_label for FormatLabel<.json> {
    fn label(*self) -> str {
        return "json";
    }
}

derive fn compact_json_label for FormatLabel<.json> {
    fn label(*self) -> str {
        return "compact json";
    }
}

struct Record {
    id: u32,
}

-- 第 1 次派生：为 Record 生成了 FormatLabel<.json> 的实现
derive FormatLabel<.json>.use(json_label) for Record;

-- ❌ 编译期直接拒绝！（Coherence 重叠冲突）
-- 策略名 compact_json_label 不会创造第二层接口身份！
-- 这行代码同样试图为 Record 生成 FormatLabel<.json> 的实现，
-- 与上一行生成的同精度实现正面冲突！
derive FormatLabel<.json>.use(compact_json_label) for Record;
```

### 💡 那么，如果我真的需要在同一个类型上同时支持两种 JSON 格式，正道是什么？

正道我们在前面已经学过了，有两种极其优雅的零开销方案：
1. **方案 A（通过接口参数区分完整接口实例）**：把格式细分反映在 Trait 的参数上，比如 `FormatLabel<.json>` 与 `FormatLabel<.compact_json>`，它们是两个不同的完整接口实例，可以在同一个 `Record` 上共存；
2. **方案 B（通过第 099 期的 `newtype` 与 `.as(...)` 视角适配）**：定义一个 `newtype CompactRecord = Record;`，在 `CompactRecord` 上使用 `compact_json_label` 派生，调用时只需写 `record.as(CompactRecord).label<.json>()`！既零运行时拷贝，又完美守住了全局 Coherence 的唯一性底线！

---

## 五、本节核心概念速查表

| 特性 / 规则 | 标准语法 | 核心语义与防坑指南 |
| :--- | :--- | :--- |
| **注册默认派生策略** | `^attrs.derivation.default` | 标记在 `derive fn` 前；支持先 `use attrs.derivation.default as ...;` 取别名 |
| **使用默认策略派生** | `derive Trait for Type;` | 自动检索唯一适用的默认策略；若有多个默认策略竞争则报错，绝不按导入顺序猜 |
| **显式指定具名策略** | `derive Trait.use(strat) for Type;` | 仅检查所指明的 `strat`；若不适用直接报错，**绝不暗中回退到默认策略** |
| **默认策略注册归属** | 本地 Trait 或首个本地身份前无自由参数 | 防止第三方包随意篡改外部接口的全局默认派生行为 |
| **策略身份擦除铁律** | 策略名 $\notin$ 最终接口身份 | 同一主体、同一完整接口实例、同一相上不能用两个不同策略重复 `derive` |

---

## 六、下一期预告

恭喜你！到这里，我们已经走完了第五卷前半程（096–105）的壮丽旅途：
从 `trait` 与 `assoc` 声明、正负约束组合（`:-` 与 `requires not`）、统一深度结构精度、有序本地身份规则（Coherence），到跨越双层宇宙的两相 Trait、`Splice` 协议、语言运算符协议，直至纯语言级的 `derive fn` 与策略体系！

但在第五卷的后半程（106–115），还有更加硬核的挑战在等待着我们的遥测与张量序列化引擎：
- 当一个对象同时通过固有方法（Inherent Methods）、多个 Trait 实现、甚至多级指针解引用（`*T`）暴露了成员时，**编译器的成员投影（Member Projection）与接收者自动适配（Receiver Adaptation）到底遵循怎样的精确优先级？**
- 当一个 Trait 实现本身**依赖运行时的环境对象**（比如序列化或内存池操作需要绑定一个运行时的 `Arena` 或 `CodecContext` 实例）时，Flurry 是如何通过 `where` 替换与环境交付把运行时依赖无缝注入进 Trait 实现里的？
- 当我们在运行期需要把不同传感器放进同一个异构数组里时，如何用 `expr.dyn(Trait)` 构造**按需动态分派接口（Dynamic Interfaces）**？

下一期 **《Flurry 演进手记 (106)：方法调用的幕后侦探——成员投影、接收者自动适配与显式 .as(Trait) 消歧》**，让我们继续深入类型系统的核心腹地！
